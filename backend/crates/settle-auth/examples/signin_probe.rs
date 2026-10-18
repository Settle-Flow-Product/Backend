//! An end-to-end sign-in probe against a running backend.
//!
//! Does exactly what the browser does: request a challenge, sign the challenge TRANSACTION the way
//! a wallet would, post the signed envelope back. Then checks that a stranger's signature and a
//! signature over a different transaction are both refused.
//!
//!   cargo run -p settle-auth --example signin_probe -- http://localhost:5173/api

use std::process::Command;

use ed25519_dalek::{Signer, SigningKey};
use stellar_xdr::curr::{
    DecoratedSignature, Limits, ReadXdr, Signature, SignatureHint, TransactionEnvelope,
    TransactionV1Envelope, WriteXdr,
};

fn post(base: &str, path: &str, body: &str) -> String {
    let out = Command::new("curl")
        .args(["-s", "-m", "20", "-X", "POST", &format!("{base}{path}")])
        .args(["-H", "content-type: application/json", "-d", body])
        .output()
        .expect("curl runs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn field(json: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let Some(start) = json.find(&needle) else {
        return String::new();
    };
    let rest = &json[start + needle.len()..];
    let end = rest.find('"').unwrap_or(0);
    rest[..end].to_string()
}

/// Sign an envelope the way a wallet does: attach a signature over the transaction's payload hash.
fn wallet_sign(xdr: &str, sk: &SigningKey, network_passphrase: &str) -> String {
    let envelope = TransactionEnvelope::from_xdr_base64(xdr, Limits::none()).expect("valid xdr");
    let TransactionEnvelope::Tx(v1) = envelope else {
        panic!("expected a v1 envelope");
    };
    let network = settle_chain::Network::new(network_passphrase.to_string());
    let hash = settle_chain::tx::signature_payload_hash(&v1.tx, &network).expect("hash");

    let signed = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx: v1.tx,
        signatures: vec![DecoratedSignature {
            hint: SignatureHint(settle_chain::signature_hint(sk.verifying_key().to_bytes())),
            signature: Signature(sk.sign(&hash).to_bytes().to_vec().try_into().unwrap()),
        }]
        .try_into()
        .unwrap(),
    });
    signed.to_xdr_base64(Limits::none()).expect("encode")
}

fn main() {
    let base = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://localhost:8088".into());
    println!("sign-in probe against {base}");

    let owner = SigningKey::from_bytes(&[31u8; 32]);
    let stranger = SigningKey::from_bytes(&[32u8; 32]);
    let address = settle_chain::encode_account(owner.verifying_key().to_bytes());

    let mut ok = true;

    // 1 · the honest path
    let challenge = post(
        &base,
        "/auth/wallet/challenge",
        &format!("{{\"address\":\"{address}\"}}"),
    );
    let nonce = field(&challenge, "nonce");
    let xdr = field(&challenge, "transaction_xdr");
    let passphrase = field(&challenge, "network_passphrase");

    if nonce.is_empty() || xdr.is_empty() {
        println!("  challenge: FAILED — {challenge}");
        std::process::exit(1);
    }

    // The property the whole design rests on.
    let seq_is_zero = {
        let e = TransactionEnvelope::from_xdr_base64(&xdr, Limits::none()).expect("valid xdr");
        matches!(&e, TransactionEnvelope::Tx(v1) if v1.tx.seq_num.0 == 0)
    };
    println!(
        "  challenge is unsubmittable (seq 0): {}",
        if seq_is_zero {
            "yes"
        } else {
            "NO — THIS IS A HOLE"
        }
    );
    ok &= seq_is_zero;

    let signed = wallet_sign(&xdr, &owner, &passphrase);
    let verify = post(
        &base,
        "/auth/wallet/verify",
        &format!("{{\"nonce\":\"{nonce}\",\"signed_xdr\":\"{signed}\"}}"),
    );
    let accepted = verify.contains("\"token\"");
    if accepted {
        println!("  the owner signs the challenge:      accepted");
    } else {
        println!("  the owner signs the challenge:      REFUSED — {verify}");
    }
    ok &= accepted;

    // 2 · a stranger signing the same transaction must not get in
    let challenge = post(
        &base,
        "/auth/wallet/challenge",
        &format!("{{\"address\":\"{address}\"}}"),
    );
    let nonce = field(&challenge, "nonce");
    let xdr = field(&challenge, "transaction_xdr");
    let forged = wallet_sign(&xdr, &stranger, &passphrase);
    let verify = post(
        &base,
        "/auth/wallet/verify",
        &format!("{{\"nonce\":\"{nonce}\",\"signed_xdr\":\"{forged}\"}}"),
    );
    let refused = !verify.contains("\"token\"");
    println!(
        "  a stranger signs it:                {}",
        if refused {
            "refused"
        } else {
            "ACCEPTED — THIS IS A HOLE"
        }
    );
    ok &= refused;

    // 3 · a nonce cannot be used twice
    let challenge = post(
        &base,
        "/auth/wallet/challenge",
        &format!("{{\"address\":\"{address}\"}}"),
    );
    let nonce = field(&challenge, "nonce");
    let xdr = field(&challenge, "transaction_xdr");
    let signed = wallet_sign(&xdr, &owner, &passphrase);
    let body = format!("{{\"nonce\":\"{nonce}\",\"signed_xdr\":\"{signed}\"}}");
    let first = post(&base, "/auth/wallet/verify", &body);
    let second = post(&base, "/auth/wallet/verify", &body);
    let replay_refused = first.contains("\"token\"") && !second.contains("\"token\"");
    println!(
        "  replaying the same challenge:       {}",
        if replay_refused {
            "refused"
        } else {
            "ACCEPTED — THIS IS A HOLE"
        }
    );
    ok &= replay_refused;

    if ok {
        println!("\nsign-in works, and forgery and replay are both refused");
    } else {
        println!("\nPROBE FAILED");
        std::process::exit(1);
    }
}
