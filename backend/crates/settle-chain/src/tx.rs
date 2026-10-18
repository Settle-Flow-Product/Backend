//! Building the payment transaction, and computing the hash approvers sign.
//!
//! Module C1. Four properties are mandatory and each prevents a specific, expensive failure:
//!
//! * **The sequence number is reserved before the hash is computed.** The hash covers the sequence,
//!   so changing it after signatures are collected voids every one of them.
//! * **The memo is the request id.** It is the reverse-lookup key: reconciliation can find this
//!   transaction on chain without ever having received its hash.
//! * **Timebounds are set and narrow.** A fully-signed, unsubmitted transaction is a signed
//!   cheque; without `maxTime` it stays usable indefinitely by anyone holding a copy.
//! * **Nothing here submits.** Building and submitting are separate so that the guard against
//!   double submission lives in one place.

use stellar_xdr::curr::{
    AlphaNum4, Asset as XdrAsset, AssetCode4, DecoratedSignature, FeeBumpTransaction,
    FeeBumpTransactionEnvelope, FeeBumpTransactionExt, FeeBumpTransactionInnerTx, Limits, Memo,
    Operation, OperationBody, PaymentOp, Preconditions, SequenceNumber, Signature, SignatureHint,
    TimeBounds, TimePoint, Transaction, TransactionEnvelope, TransactionExt,
    TransactionSignaturePayload, TransactionSignaturePayloadTaggedTransaction,
    TransactionV1Envelope, VecM, WriteXdr,
};

use settle_domain::{Asset, Stroops};
use sha2::{Digest, Sha256};

use crate::error::ChainError;
use crate::keys::{account_id, hash, muxed_account, signature_hint};
use crate::network::Network;

/// The protocol's base fee, in stroops, per operation.
pub const BASE_FEE: u32 = 100;

/// The signing window. One hour, per module C1.
pub const TIMEBOUNDS_SECONDS: u64 = 3_600;

/// Everything needed to build one vendor payment.
#[derive(Debug, Clone)]
pub struct PaymentSpec {
    /// The treasury account. It supplies the sequence number and the signatures.
    pub source: String,
    /// The vendor being paid.
    pub destination: String,
    pub asset: Asset,
    /// The USDC issuer. Ignored for native XLM.
    pub asset_issuer: Option<String>,
    pub amount: Stroops,
    /// The reserved sequence number. This is the account's current sequence **plus one**.
    pub sequence: i64,
    /// The request id, which becomes the memo.
    pub memo_request_id: [u8; 32],
    /// Unix seconds. The window opens here.
    pub min_time: u64,
    /// Unix seconds. The window closes here, and it must.
    pub max_time: u64,
    /// Fee bid, in stroops. A multiple of `BASE_FEE` buys resistance to surge pricing.
    pub fee: u32,
}

/// A built, unsigned transaction, and the hash its approvers must sign.
#[derive(Debug, Clone)]
pub struct UnsignedTx {
    pub envelope: TransactionEnvelope,
    /// The signature payload hash. **This exact value is what a signer signs.**
    pub tx_hash: [u8; 32],
    pub sequence: i64,
    pub max_time: u64,
}

impl UnsignedTx {
    pub fn to_xdr_base64(&self) -> Result<String, ChainError> {
        self.envelope
            .to_xdr_base64(Limits::none())
            .map_err(|e| ChainError::Encode(e.to_string()))
    }

    pub fn tx_hash_hex(&self) -> String {
        hex::encode(self.tx_hash)
    }
}

fn xdr_asset(asset: Asset, issuer: Option<&str>) -> Result<XdrAsset, ChainError> {
    match asset {
        Asset::Xlm => Ok(XdrAsset::Native),
        Asset::Usdc => {
            let issuer = issuer.ok_or_else(|| ChainError::InvalidAssetCode("USDC".into()))?;
            let mut code = [0u8; 4];
            code[..4].copy_from_slice(b"USDC");
            Ok(XdrAsset::CreditAlphanum4(AlphaNum4 {
                asset_code: AssetCode4(code),
                issuer: account_id(issuer)?,
            }))
        }
    }
}

/// Build the unsigned payment.
pub fn build_payment(spec: &PaymentSpec, network: &Network) -> Result<UnsignedTx, ChainError> {
    if spec.max_time <= spec.min_time {
        return Err(ChainError::EmptyTimebounds);
    }
    let amount: i64 = spec
        .amount
        .as_i128()
        .try_into()
        .map_err(|_| ChainError::AmountOutOfRange)?;

    let op = Operation {
        source_account: None,
        body: OperationBody::Payment(PaymentOp {
            destination: muxed_account(&spec.destination)?,
            asset: xdr_asset(spec.asset, spec.asset_issuer.as_deref())?,
            amount,
        }),
    };

    let operations: VecM<Operation, 100> = vec![op]
        .try_into()
        .map_err(|_| ChainError::Encode("too many operations".into()))?;

    let tx = Transaction {
        source_account: muxed_account(&spec.source)?,
        fee: spec.fee,
        seq_num: SequenceNumber(spec.sequence),
        // MANDATORY. Without maxTime a signed transaction never stops being usable.
        cond: Preconditions::Time(TimeBounds {
            min_time: TimePoint(spec.min_time),
            max_time: TimePoint(spec.max_time),
        }),
        // THE RECONCILIATION KEY. memo = request id, so a transaction whose hash was never
        // received can still be found on chain.
        memo: Memo::Hash(hash(spec.memo_request_id)),
        operations,
        ext: TransactionExt::V0,
    };

    let tx_hash = signature_payload_hash(&tx, network)?;

    Ok(UnsignedTx {
        envelope: TransactionEnvelope::Tx(TransactionV1Envelope {
            tx,
            signatures: VecM::default(),
        }),
        tx_hash,
        sequence: spec.sequence,
        max_time: spec.max_time,
    })
}

/// `sha256(networkId ‖ ENVELOPE_TYPE_TX ‖ tx)` — the value a signer signs.
///
/// The network id is inside the hash, which is what stops a signature collected on testnet from
/// being replayable on mainnet.
pub fn signature_payload_hash(tx: &Transaction, network: &Network) -> Result<[u8; 32], ChainError> {
    let payload = TransactionSignaturePayload {
        network_id: hash(network.id()),
        tagged_transaction: TransactionSignaturePayloadTaggedTransaction::Tx(tx.clone()),
    };
    let bytes = payload
        .to_xdr(Limits::none())
        .map_err(|e| ChainError::Encode(e.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

/// One approver's contribution.
#[derive(Debug, Clone)]
pub struct CollectedSignature {
    pub signer_public_key: [u8; 32],
    pub signature: [u8; 64],
}

/// Attach the collected signatures to the envelope — module C5.
///
/// The order does not matter to the protocol, but the **hint** does: it is how the network matches
/// each signature to a signer, and a wrong hint produces a transaction refused after everyone has
/// already signed.
pub fn attach_signatures(
    unsigned: &UnsignedTx,
    signatures: &[CollectedSignature],
) -> Result<TransactionEnvelope, ChainError> {
    let TransactionEnvelope::Tx(inner) = &unsigned.envelope else {
        return Err(ChainError::Encode(
            "expected a v1 transaction envelope".into(),
        ));
    };

    let mut decorated: Vec<DecoratedSignature> = Vec::with_capacity(signatures.len());
    for s in signatures {
        decorated.push(DecoratedSignature {
            hint: SignatureHint(signature_hint(s.signer_public_key)),
            signature: Signature(
                s.signature
                    .to_vec()
                    .try_into()
                    .map_err(|_| ChainError::MalformedSignature)?,
            ),
        });
    }

    Ok(TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: inner.tx.clone(),
        signatures: decorated
            .try_into()
            .map_err(|_| ChainError::Encode("too many signatures".into()))?,
    }))
}

/// Wrap an already-signed envelope in a fee bump — module C7.
///
/// The inner envelope is untouched, so **every approver signature stays valid**. Only the fee
/// source signs the wrapper. This is why a transaction that is not being included for fee reasons
/// is never rebuilt: rebuilding would discard valid signatures and interrupt two people over a
/// problem the protocol already solves.
pub fn fee_bump(
    inner: &TransactionEnvelope,
    fee_source: &str,
    fee: i64,
) -> Result<TransactionEnvelope, ChainError> {
    let TransactionEnvelope::Tx(v1) = inner else {
        return Err(ChainError::Encode(
            "only a v1 transaction envelope can be fee-bumped".into(),
        ));
    };

    Ok(TransactionEnvelope::TxFeeBump(FeeBumpTransactionEnvelope {
        tx: FeeBumpTransaction {
            fee_source: muxed_account(fee_source)?,
            fee,
            inner_tx: FeeBumpTransactionInnerTx::Tx(v1.clone()),
            ext: FeeBumpTransactionExt::V0,
        },
        signatures: VecM::default(),
    }))
}
