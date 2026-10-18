//! Contract calls, through the Stellar CLI.
//!
//! ## Why a subprocess rather than an in-process RPC client
//!
//! A Soroban invocation is not one request. It is: read the source account's sequence, build an
//! `InvokeHostFunction` transaction, call `simulateTransaction` to obtain the footprint and the
//! resource fee, patch both into the transaction, sign, submit, then poll for the result. Every
//! one of those steps has a protocol-version-specific encoding, and getting any of them wrong
//! produces a transaction the network rejects for reasons that are hard to read.
//!
//! The Stellar CLI already does all of it, is maintained alongside the protocol, and is already a
//! dependency of the deploy scripts. For a twenty-day deliverable, reusing it is the choice that
//! spends the risk budget where the risk actually is — on the treasury payment path, which this
//! module does *not* handle. That path builds its own XDR in `settle-chain`, because the
//! signatures collected there are the product.
//!
//! ## Keys
//!
//! Secrets live in the CLI's own keystore and are referenced by name. **No private key passes
//! through this process, its environment, or its logs.**

use std::process::Stdio;

use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("the stellar CLI is not installed or not on PATH")]
    CliMissing,
    #[error("contract call `{function}` failed: {stderr}")]
    CallFailed { function: String, stderr: String },
    #[error("could not run the stellar CLI: {0}")]
    Spawn(String),
    #[error("the contract returned something unreadable: {0}")]
    Unreadable(String),
}

/// A handle to the deployed registry.
/// What one CLI invocation produced.
///
/// The transaction hash lives on **stderr**, not stdout — stdout carries the contract's return
/// value. Discarding stderr on success meant every on-chain write completed without anyone learning
/// which transaction did it, and evidence milestone E3 asks for exactly that hash.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// The contract's return value, as printed on stdout.
    pub output: String,
    /// The submitted transaction's hash. `None` for a simulated read, which submits nothing.
    pub tx_hash: Option<String>,
}

/// Pull the submitted transaction's hash out of the CLI's stderr.
///
/// The explorer link is preferred over the `Signing transaction:` line because it is unambiguous.
/// Several 64-character hex strings appear on stderr — the request id shows up in the decoded event
/// — so "the first thing shaped like a hash" is a rule that works until the day a contract argument
/// happens to sort earlier.
fn tx_hash_from(stderr: &str) -> Option<String> {
    fn hex64_after(haystack: &str, marker: &str) -> Option<String> {
        let start = haystack.find(marker)? + marker.len();
        let candidate: String = haystack[start..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        (candidate.len() == 64).then_some(candidate)
    }

    hex64_after(stderr, "/tx/").or_else(|| hex64_after(stderr, "Signing transaction:"))
}

#[derive(Debug, Clone)]
pub struct Registry {
    pub contract_id: String,
    pub network_passphrase: String,
    pub rpc_url: String,
}

/// What `get_request` returns.
///
/// Every field the contract stores is captured, not only the ones read today. This struct is the
/// decoding contract with the chain: a field that is silently dropped here is a field nobody
/// notices has changed shape until a reconciliation gives the wrong answer.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OnChainRequest {
    pub id: String,
    pub creator: String,
    pub payee: String,
    pub asset: String,
    /// A string, because the contract works in stroops and JSON numbers are doubles.
    pub amount: serde_json::Value,
    pub reference: String,
    pub expiry: serde_json::Value,
    pub status: serde_json::Value,
    pub paid_tx: serde_json::Value,
}

impl OnChainRequest {
    /// The status name, whichever shape the CLI rendered the enum in.
    ///
    /// **The number is the shape that actually happens.** `ReqStatus` is `#[repr(u32)]`, so the CLI
    /// prints `"status":2`, and the fallback here used to stringify that to `"2"` — which no caller
    /// matched. The visible effect was a payment that reached `Paid` on chain and stayed `Approved`
    /// in the interface forever, because the readback that gates the local write could never agree
    /// that it had been paid.
    ///
    /// The string and object arms are kept: the CLI has rendered enums both ways across versions,
    /// and a decoder that only understands the current one breaks on an upgrade.
    pub fn status_name(&self) -> String {
        match &self.status {
            serde_json::Value::Number(n) => match n.as_u64() {
                Some(0) => "pending".into(),
                Some(1) => "approved".into(),
                Some(2) => "paid".into(),
                Some(3) => "cancelled".into(),
                _ => n.to_string(),
            },
            serde_json::Value::String(s) => s.to_lowercase(),
            serde_json::Value::Object(o) => {
                o.keys().next().cloned().unwrap_or_default().to_lowercase()
            }
            other => other.to_string().trim_matches('"').to_lowercase(),
        }
    }
}

impl Registry {
    pub fn new(contract_id: String, network_passphrase: String, rpc_url: String) -> Self {
        Self {
            contract_id,
            network_passphrase,
            rpc_url,
        }
    }

    async fn invoke(
        &self,
        source_key: &str,
        function: &str,
        args: &[(&str, String)],
        send: bool,
    ) -> Result<Invocation, RegistryError> {
        let mut cmd = Command::new("stellar");
        cmd.arg("contract")
            .arg("invoke")
            .arg("--id")
            .arg(&self.contract_id)
            .arg("--source")
            .arg(source_key)
            .arg("--rpc-url")
            .arg(&self.rpc_url)
            .arg("--network-passphrase")
            .arg(&self.network_passphrase);

        // A read is simulated, never submitted. Soroban has no view functions: submitting
        // `get_request` as a transaction would cost a fee and a ledger slot for an answer that
        // simulation gives for nothing.
        cmd.arg("--send").arg(if send { "yes" } else { "no" });

        cmd.arg("--").arg(function);
        for (name, value) in args {
            cmd.arg(format!("--{name}")).arg(value);
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RegistryError::CliMissing
            } else {
                RegistryError::Spawn(e.to_string())
            }
        })?;

        if !output.status.success() {
            return Err(RegistryError::CallFailed {
                function: function.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(Invocation {
            output: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            // Only a submitted transaction has a hash; a simulated read never produces one.
            tx_hash: if send { tx_hash_from(&stderr) } else { None },
        })
    }

    /// Record a payment request on chain — step S-09.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_request(
        &self,
        business_key: &str,
        creator: &str,
        payee: &str,
        id_hex: &str,
        asset_contract: &str,
        amount_stroops: i128,
        reference_hash_hex: &str,
        expiry_unix: u64,
    ) -> Result<Invocation, RegistryError> {
        self.invoke(
            business_key,
            "create_request",
            &[
                ("creator", creator.to_string()),
                ("payee", payee.to_string()),
                ("id", id_hex.to_string()),
                ("asset", asset_contract.to_string()),
                ("amount", amount_stroops.to_string()),
                ("reference", reference_hash_hex.to_string()),
                ("expiry", expiry_unix.to_string()),
            ],
            true,
        )
        .await
    }

    /// `Pending → Approved`, once the collected weight meets the threshold.
    pub async fn approve(
        &self,
        settler_key: &str,
        id_hex: &str,
    ) -> Result<Invocation, RegistryError> {
        self.invoke(settler_key, "approve", &[("id", id_hex.to_string())], true)
            .await
    }

    /// `Approved → Paid`.
    ///
    /// **Called only after a ledger readback confirms the payment settled.** Calling it on the
    /// strength of a successful submit is the defect the whole design exists to prevent.
    pub async fn mark_paid(
        &self,
        settler_key: &str,
        id_hex: &str,
        amount_stroops: i128,
        tx_hash_hex: &str,
    ) -> Result<Invocation, RegistryError> {
        self.invoke(
            settler_key,
            "mark_paid",
            &[
                ("id", id_hex.to_string()),
                ("amount", amount_stroops.to_string()),
                ("tx", tx_hash_hex.to_string()),
            ],
            true,
        )
        .await
    }

    /// The reject branch.
    pub async fn cancel_request(
        &self,
        business_key: &str,
        id_hex: &str,
    ) -> Result<Invocation, RegistryError> {
        self.invoke(
            business_key,
            "cancel_request",
            &[("id", id_hex.to_string())],
            true,
        )
        .await
    }

    /// Read a request back — the authoritative answer to "what status is this in".
    ///
    /// Simulated, never submitted.
    pub async fn get_request(
        &self,
        reader_key: &str,
        id_hex: &str,
    ) -> Result<OnChainRequest, RegistryError> {
        let out = self
            .invoke(
                reader_key,
                "get_request",
                &[("id", id_hex.to_string())],
                false,
            )
            .await?;
        // A read is simulated, so only stdout carries anything.
        let json = out.output;
        serde_json::from_str(&json).map_err(|e| RegistryError::Unreadable(format!("{e}: {json}")))
    }

    /// Whether a request can still be paid.
    pub async fn is_payable(&self, reader_key: &str, id_hex: &str) -> Result<bool, RegistryError> {
        let out = self
            .invoke(
                reader_key,
                "is_payable",
                &[("id", id_hex.to_string())],
                false,
            )
            .await?;
        Ok(out.output.trim() == "true")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STDERR: &str = "\
\u{2139}\u{fe0f}  Signing transaction: 057fc9e1591a296e71c2c5a13ccaeea3526ee02c7cff8c07a28709d601c1941f
\u{1f30e} Sending transaction\u{2026}
\u{2705} Transaction submitted successfully!
\u{1f517} https://stellar.expert/explorer/testnet/tx/057fc9e1591a296e71c2c5a13ccaeea3526ee02c7cff8c07a28709d601c1941f
\u{1f4c5} CAYZ - Success - Event: RequestCreated (settle, req_new), id: \"a5b77045e8b5747c5c7240aea6a6b9120cfd712202172788aae80cf1901f971e\"";

    #[test]
    fn the_transaction_hash_comes_from_the_explorer_link() {
        assert_eq!(
            tx_hash_from(STDERR).as_deref(),
            Some("057fc9e1591a296e71c2c5a13ccaeea3526ee02c7cff8c07a28709d601c1941f")
        );
    }

    #[test]
    fn the_request_id_in_the_event_is_not_mistaken_for_the_hash() {
        // The id is also 64 hex characters and also on stderr. Anything that just scans for the
        // first hash-shaped string is one output-ordering change away from recording the wrong one.
        assert_ne!(
            tx_hash_from(STDERR).as_deref(),
            Some("a5b77045e8b5747c5c7240aea6a6b9120cfd712202172788aae80cf1901f971e")
        );
    }

    #[test]
    fn the_signing_line_is_the_fallback_when_there_is_no_link() {
        let s = "Signing transaction: 057fc9e1591a296e71c2c5a13ccaeea3526ee02c7cff8c07a28709d601c1941f\nSending\u{2026}";
        assert_eq!(
            tx_hash_from(s).as_deref(),
            Some("057fc9e1591a296e71c2c5a13ccaeea3526ee02c7cff8c07a28709d601c1941f")
        );
    }

    #[test]
    fn output_with_no_hash_yields_none() {
        assert_eq!(tx_hash_from("simulating only, nothing submitted"), None);
    }

    #[test]
    fn a_truncated_hash_is_refused_rather_than_returned_short() {
        assert_eq!(tx_hash_from("/tx/abc123"), None);
    }

    fn with_status(v: serde_json::Value) -> OnChainRequest {
        OnChainRequest {
            id: String::new(),
            creator: String::new(),
            payee: String::new(),
            asset: String::new(),
            amount: serde_json::Value::Null,
            reference: String::new(),
            expiry: serde_json::Value::Null,
            status: v,
            paid_tx: serde_json::Value::Null,
        }
    }

    #[test]
    fn a_numeric_status_is_named() {
        // The shape the CLI actually emits, because ReqStatus is #[repr(u32)].
        for (n, name) in [
            (0, "pending"),
            (1, "approved"),
            (2, "paid"),
            (3, "cancelled"),
        ] {
            assert_eq!(with_status(serde_json::json!(n)).status_name(), name);
        }
    }

    #[test]
    fn a_paid_request_reads_as_paid_so_the_local_write_can_proceed() {
        // The exact check `confirm_and_mark_paid` makes. When this returned "2" the settlement sat
        // at `releasing` forever while the chain said Paid.
        assert!(with_status(serde_json::json!(2))
            .status_name()
            .contains("paid"));
    }

    #[test]
    fn a_string_status_still_works_and_is_lowercased() {
        assert_eq!(with_status(serde_json::json!("Paid")).status_name(), "paid");
    }

    #[test]
    fn an_object_status_still_works() {
        assert_eq!(
            with_status(serde_json::json!({"Approved": []})).status_name(),
            "approved"
        );
    }
}
