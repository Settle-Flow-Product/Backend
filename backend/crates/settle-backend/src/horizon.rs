//! Horizon: read an account's sequence, submit a transaction, and find one by its memo.
//!
//! The memo lookup is the important one. A settlement whose transaction hash was never received is
//! still findable, because the request id is written into the transaction itself — which is what
//! makes `tx_bad_seq` recoverable instead of a coin flip between "already paid" and "pay again".

use settle_domain::SubmitOutcome;

#[derive(Debug, thiserror::Error)]
pub enum HorizonError {
    #[error("could not reach Horizon: {0}")]
    Unreachable(String),
    #[error("Horizon returned something unreadable: {0}")]
    Unreadable(String),
    #[error("account {0} does not exist on this network")]
    AccountNotFound(String),
    /// Horizon refused the request itself — an account id that fails its strkey checksum is the
    /// usual cause. Distinct from `AccountNotFound`: the address is not merely absent, it is not a
    /// well-formed address, and the interface has to say something different about each.
    #[error("Horizon rejected the request: {0}")]
    BadRequest(String),
    /// Any other non-success status. Named rather than folded into `Unreadable` because a 429 or a
    /// 503 is Horizon being busy, not Horizon being broken.
    #[error("Horizon returned HTTP {0}")]
    Status(u16),
}

/// The answer to "can this address receive this asset". A missing ACCOUNT is
/// `HorizonError::AccountNotFound`, not a variant here: the two need different copy in the
/// interface, and collapsing them loses which one to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receivable {
    Yes,
    /// The account exists but holds no authorised trustline for the asset.
    NoTrustline,
}

#[derive(Debug, Clone)]
pub struct Horizon {
    base: String,
    http: reqwest::Client,
}

/// What a submit told us, and what it did not.
#[derive(Debug, Clone)]
pub struct SubmitResult {
    pub outcome: SubmitOutcome,
    /// Present only when the network accepted the envelope. **Not the same as settled.**
    pub tx_hash: Option<String>,
    pub detail: Option<String>,
}

impl Horizon {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                // A request path that hangs is worse than one that fails: the caller is a person
                // waiting on a button.
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("a default HTTP client always builds"),
        }
    }

    /// The account's current sequence number.
    ///
    /// A transaction's sequence must be this **plus one**. The number exists only on the network
    /// and cannot be cached: any other transaction from the account invalidates it immediately.
    pub async fn sequence(&self, account: &str) -> Result<i64, HorizonError> {
        let url = format!("{}/accounts/{}", self.base, account);
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| HorizonError::Unreachable(e.to_string()))?;

        // Classified before the body is parsed, for the same reason as `account` below. This path
        // fails safe either way — a missing `sequence` field is an error — but "Horizon returned
        // HTTP 400" is a diagnosis and "no sequence in {…}" is a puzzle.
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(HorizonError::AccountNotFound(account.to_string()));
        }
        if status == reqwest::StatusCode::BAD_REQUEST {
            return Err(HorizonError::BadRequest(account.to_string()));
        }
        if !status.is_success() {
            return Err(HorizonError::Status(status.as_u16()));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| HorizonError::Unreadable(e.to_string()))?;

        body.get("sequence")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| HorizonError::Unreadable(format!("no sequence in {body}")))
    }

    /// The account's signer configuration and thresholds — what `verify_treasury` asserts.
    pub async fn account(&self, account: &str) -> Result<serde_json::Value, HorizonError> {
        let url = format!("{}/accounts/{}", self.base, account);
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| HorizonError::Unreachable(e.to_string()))?;

        // Every non-success status has to be classified BEFORE the body is parsed. Horizon answers
        // an error with a well-formed JSON document, so `res.json()` on a 400 succeeds and hands
        // back an "account" with no signers, no thresholds and no balances — which reads to every
        // caller as an account that exists and holds nothing. A checksum-invalid address was
        // reported as receivable that way.
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(HorizonError::AccountNotFound(account.to_string()));
        }
        if status == reqwest::StatusCode::BAD_REQUEST {
            return Err(HorizonError::BadRequest(account.to_string()));
        }
        if !status.is_success() {
            return Err(HorizonError::Status(status.as_u16()));
        }

        res.json()
            .await
            .map_err(|e| HorizonError::Unreadable(e.to_string()))
    }

    /// Whether `account` is able to RECEIVE an issued asset — module E2's destination check.
    ///
    /// Paying USDC to an address that holds no USDC trustline fails at the ledger with
    /// `op_no_trust`, and it fails **at submit** — which is to say after both approvers have already
    /// been interrupted to sign. Asking Horizon while the person is still typing the address moves
    /// that failure from the end of the process to the field that caused it.
    ///
    /// Native XLM needs no trustline, so callers only ask this for assets where
    /// `Asset::requires_trustline()` is true.
    pub async fn can_receive(
        &self,
        account: &str,
        asset_code: &str,
        issuer: &str,
    ) -> Result<Receivable, HorizonError> {
        let body = self.account(account).await?;

        Ok(if holds_trustline(&body, asset_code, issuer) {
            Receivable::Yes
        } else {
            Receivable::NoTrustline
        })
    }

    /// Submit a signed envelope.
    ///
    /// Every outcome is classified rather than collapsed into success or failure. `tx_bad_seq` and
    /// a timeout are **not failures** — they mean the outcome is unknown, and the caller must ask
    /// the chain before doing anything else.
    pub async fn submit(&self, envelope_xdr: &str) -> SubmitResult {
        let url = format!("{}/transactions", self.base);
        let res = self
            .http
            .post(&url)
            .form(&[("tx", envelope_xdr)])
            .send()
            .await;

        let res = match res {
            Ok(r) => r,
            Err(e) => {
                // EVERY transport failure is an unknown outcome, not a failure — a timeout, a
                // dropped connection, a DNS blip. In all of them the envelope may already have
                // reached the network and may still be included seconds later. Treating any of
                // them as "it did not happen" is how a vendor gets paid twice.
                return SubmitResult {
                    outcome: SubmitOutcome::Timeout,
                    tx_hash: None,
                    detail: Some(format!(
                        "transport failure (timeout={}): {e}",
                        e.is_timeout()
                    )),
                };
            }
        };

        let status = res.status();
        let body: serde_json::Value = match res.json().await {
            Ok(b) => b,
            Err(e) => {
                return SubmitResult {
                    outcome: SubmitOutcome::Timeout,
                    tx_hash: None,
                    detail: Some(format!("unreadable response: {e}")),
                }
            }
        };

        if status.is_success() {
            let hash = body
                .get("hash")
                .and_then(|h| h.as_str())
                .map(|s| s.to_string());
            return SubmitResult {
                outcome: SubmitOutcome::Accepted,
                tx_hash: hash,
                detail: None,
            };
        }

        let code = body
            .pointer("/extras/result_codes/transaction")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let outcome = match code.as_str() {
            // The sequence has been consumed. It does NOT say by whom, and the most common cause
            // is that this very envelope already reached the ledger.
            "tx_bad_seq" => SubmitOutcome::BadSequence,
            "tx_insufficient_fee" => SubmitOutcome::InsufficientFee,
            "tx_too_late" | "tx_too_early" => SubmitOutcome::Failed,
            "" => SubmitOutcome::Timeout,
            _ => SubmitOutcome::Failed,
        };

        SubmitResult {
            outcome,
            tx_hash: None,
            detail: Some(if code.is_empty() {
                body.to_string()
            } else {
                code
            }),
        }
    }

    /// Find a transaction on an account by its memo — the reconciliation key.
    ///
    /// Returns the hash of the most recent successful transaction whose memo hash matches. This is
    /// what makes recovery possible without ever having received a transaction hash.
    pub async fn find_by_memo(
        &self,
        account: &str,
        memo_hash_hex: &str,
    ) -> Result<Option<String>, HorizonError> {
        let url = format!(
            "{}/accounts/{}/transactions?order=desc&limit=50",
            self.base, account
        );
        let body: serde_json::Value = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| HorizonError::Unreachable(e.to_string()))?
            .json()
            .await
            .map_err(|e| HorizonError::Unreadable(e.to_string()))?;

        let Some(records) = body
            .pointer("/_embedded/records")
            .and_then(|r| r.as_array())
        else {
            return Ok(None);
        };

        // Horizon renders a hash memo as base64. Compare on the decoded bytes so the encoding
        // Horizon happens to choose is not part of the contract.
        let want = hex::decode(memo_hash_hex).unwrap_or_default();

        for record in records {
            if record.get("successful").and_then(|s| s.as_bool()) != Some(true) {
                continue;
            }
            if record.get("memo_type").and_then(|m| m.as_str()) != Some("hash") {
                continue;
            }
            let Some(memo) = record.get("memo").and_then(|m| m.as_str()) else {
                continue;
            };
            let decoded = base64_decode(memo).unwrap_or_default();
            if decoded == want {
                return Ok(record
                    .get("hash")
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string()));
            }
        }
        Ok(None)
    }
}

/// Standard base64 with padding, which is what Horizon emits for a hash memo.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, c) in TABLE.iter().enumerate() {
        lookup[*c as usize] = i as u8;
    }

    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;

    for byte in cleaned {
        let v = lookup[byte as usize];
        if v == 255 {
            return None;
        }
        buffer = (buffer << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// Does this account payload carry an authorised trustline for `asset_code` from `issuer`?
///
/// Split out from `can_receive` because the matching is the part that can be wrong, and it is only
/// testable away from the network. Both the code AND the issuer must match: a USDC trustline from
/// some other issuer is a different asset that happens to share a ticker, and paying against it is
/// paying somebody in the wrong currency.
fn holds_trustline(account: &serde_json::Value, asset_code: &str, issuer: &str) -> bool {
    let Some(balances) = account.get("balances").and_then(|b| b.as_array()) else {
        return false;
    };

    balances.iter().any(|b| {
        let matches_asset = b.get("asset_code").and_then(|c| c.as_str()) == Some(asset_code)
            && b.get("asset_issuer").and_then(|i| i.as_str()) == Some(issuer);

        // `is_authorized` is absent for assets whose issuer does not require authorisation, and
        // absent must read as authorised — treating a missing flag as "not authorised" would reject
        // every ordinary trustline, including testnet USDC.
        let authorized = b
            .get("is_authorized")
            .and_then(|a| a.as_bool())
            .unwrap_or(true);

        matches_asset && authorized
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUER: &str = "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5";

    fn account_with(balances: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "balances": balances })
    }

    #[test]
    fn an_authorised_trustline_can_receive() {
        let a = account_with(serde_json::json!([
            { "asset_type": "native", "balance": "100.0" },
            { "asset_type": "credit_alphanum4", "asset_code": "USDC",
              "asset_issuer": ISSUER, "balance": "0.0000000" },
        ]));
        assert!(holds_trustline(&a, "USDC", ISSUER));
    }

    #[test]
    fn a_zero_balance_still_counts() {
        // Receiving needs a trustline, not a balance. Requiring a balance would refuse every vendor
        // being paid for the first time — which is most of them.
        let a = account_with(serde_json::json!([
            { "asset_code": "USDC", "asset_issuer": ISSUER, "balance": "0.0000000" },
        ]));
        assert!(holds_trustline(&a, "USDC", ISSUER));
    }

    #[test]
    fn an_account_holding_only_xlm_cannot_receive_usdc() {
        let a = account_with(serde_json::json!([{ "asset_type": "native", "balance": "100.0" }]));
        assert!(!holds_trustline(&a, "USDC", ISSUER));
    }

    #[test]
    fn the_same_code_from_a_different_issuer_is_a_different_asset() {
        // The failure this prevents is paying somebody in a currency that merely shares a ticker.
        let other = "GCTY1111111111111111111111111111111111111111111111111111";
        let a = account_with(serde_json::json!([
            { "asset_code": "USDC", "asset_issuer": other, "balance": "500.0" },
        ]));
        assert!(!holds_trustline(&a, "USDC", ISSUER));
    }

    #[test]
    fn an_explicitly_unauthorised_trustline_cannot_receive() {
        let a = account_with(serde_json::json!([
            { "asset_code": "USDC", "asset_issuer": ISSUER,
              "balance": "0.0", "is_authorized": false },
        ]));
        assert!(!holds_trustline(&a, "USDC", ISSUER));
    }

    #[test]
    fn a_missing_authorisation_flag_reads_as_authorised() {
        // Horizon omits the flag for issuers that do not require authorisation, testnet USDC among
        // them. Reading absence as "not authorised" would refuse every ordinary trustline.
        let a = account_with(serde_json::json!([
            { "asset_code": "USDC", "asset_issuer": ISSUER, "balance": "0.0" },
        ]));
        assert!(holds_trustline(&a, "USDC", ISSUER));
    }

    #[test]
    fn a_payload_without_balances_is_not_a_trustline() {
        assert!(!holds_trustline(&serde_json::json!({}), "USDC", ISSUER));
    }

    #[test]
    fn base64_decodes_what_horizon_emits() {
        // 32 bytes of 0xAB, base64 encoded — the shape of a memo hash.
        let encoded = "q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s=";
        assert_eq!(base64_decode(encoded).unwrap(), vec![0xAB; 32]);
    }

    #[test]
    fn base64_rejects_rubbish_rather_than_returning_wrong_bytes() {
        assert!(base64_decode("not base64 !!!").is_none());
    }

    #[test]
    fn a_base_url_with_a_trailing_slash_is_normalised() {
        let h = Horizon::new("https://horizon-testnet.stellar.org/");
        assert_eq!(h.base, "https://horizon-testnet.stellar.org");
    }
}
