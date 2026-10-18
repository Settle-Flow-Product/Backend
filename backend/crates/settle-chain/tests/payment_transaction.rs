//! The transaction a treasury actually submits.
//!
//! These tests exist because every one of their assertions has a matching way to lose money or a
//! demo: a missing memo makes reconciliation impossible, absent timebounds leave a signed cheque
//! in circulation, and a hash that does not cover the sequence number means signatures survive a
//! rebuild they should not survive.

use ed25519_dalek::{Signer, SigningKey};
use settle_chain::{
    attach_signatures, build_payment, fee_bump, verify_signature, CollectedSignature, Network,
    PaymentSpec,
};
use settle_domain::{Asset, Stroops};
use stellar_xdr::curr::{Limits, Memo, Preconditions, ReadXdr, TransactionEnvelope, WriteXdr};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn address(seed: u8) -> String {
    settle_chain::encode_account(key(seed).verifying_key().to_bytes())
}

fn spec() -> PaymentSpec {
    PaymentSpec {
        source: address(1),
        destination: address(2),
        asset: Asset::Usdc,
        asset_issuer: Some(address(3)),
        amount: Stroops::from_units(4_500).unwrap(),
        sequence: 42,
        memo_request_id: [0xAB; 32],
        min_time: 1_700_000_000,
        max_time: 1_700_003_600,
        fee: 1_000,
    }
}

#[test]
fn a_built_payment_carries_the_request_id_as_its_memo() {
    let tx = build_payment(&spec(), &Network::testnet()).unwrap();
    let TransactionEnvelope::Tx(v1) = &tx.envelope else {
        panic!("expected a v1 envelope");
    };
    match &v1.tx.memo {
        Memo::Hash(h) => assert_eq!(h.0, [0xAB; 32]),
        other => panic!("the memo must be the request id, got {other:?}"),
    }
}

#[test]
fn a_built_payment_always_has_a_closing_timebound() {
    let tx = build_payment(&spec(), &Network::testnet()).unwrap();
    let TransactionEnvelope::Tx(v1) = &tx.envelope else {
        panic!("expected a v1 envelope");
    };
    match &v1.tx.cond {
        Preconditions::Time(tb) => {
            assert_eq!(tb.min_time.0, 1_700_000_000);
            assert_eq!(tb.max_time.0, 1_700_003_600);
            assert!(
                tb.max_time.0 > tb.min_time.0,
                "the window must be non-empty"
            );
        }
        other => panic!("timebounds are mandatory, got {other:?}"),
    }
}

#[test]
fn timebounds_that_do_not_open_a_window_are_refused() {
    let mut s = spec();
    s.max_time = s.min_time;
    assert!(build_payment(&s, &Network::testnet()).is_err());
}

#[test]
fn the_hash_covers_the_sequence_number() {
    // This is why the sequence is reserved BEFORE the hash is computed and held until the
    // settlement resolves: change it and every collected signature is void.
    let a = build_payment(&spec(), &Network::testnet()).unwrap();
    let mut other = spec();
    other.sequence = 43;
    let b = build_payment(&other, &Network::testnet()).unwrap();
    assert_ne!(a.tx_hash, b.tx_hash);
}

#[test]
fn the_hash_covers_the_amount_and_the_destination() {
    let base = build_payment(&spec(), &Network::testnet()).unwrap();

    let mut cheaper = spec();
    cheaper.amount = Stroops::from_units(4_499).unwrap();
    assert_ne!(
        base.tx_hash,
        build_payment(&cheaper, &Network::testnet())
            .unwrap()
            .tx_hash
    );

    let mut elsewhere = spec();
    elsewhere.destination = address(9);
    assert_ne!(
        base.tx_hash,
        build_payment(&elsewhere, &Network::testnet())
            .unwrap()
            .tx_hash
    );
}

#[test]
fn the_hash_covers_the_network_so_a_signature_cannot_cross_networks() {
    let testnet = build_payment(&spec(), &Network::testnet()).unwrap();
    let public = build_payment(
        &spec(),
        &Network::new("Public Global Stellar Network ; September 2015"),
    )
    .unwrap();
    assert_ne!(testnet.tx_hash, public.tx_hash);
}

#[test]
fn building_the_same_specification_twice_gives_the_same_hash() {
    // Determinism is what lets a rebuilt transaction be compared against a stored hash instead of
    // being trusted.
    let a = build_payment(&spec(), &Network::testnet()).unwrap();
    let b = build_payment(&spec(), &Network::testnet()).unwrap();
    assert_eq!(a.tx_hash, b.tx_hash);
}

#[test]
fn an_approver_signs_the_hash_and_it_verifies() {
    let tx = build_payment(&spec(), &Network::testnet()).unwrap();
    let signer = key(5);
    let sig = signer.sign(&tx.tx_hash);
    assert!(verify_signature(
        signer.verifying_key().to_bytes(),
        &tx.tx_hash,
        &sig.to_bytes()
    )
    .is_ok());
}

#[test]
fn a_signature_from_a_rebuilt_transaction_does_not_verify_against_the_original() {
    let original = build_payment(&spec(), &Network::testnet()).unwrap();
    let mut rebuilt_spec = spec();
    rebuilt_spec.sequence = 43;
    let rebuilt = build_payment(&rebuilt_spec, &Network::testnet()).unwrap();

    let signer = key(5);
    let sig_for_rebuilt = signer.sign(&rebuilt.tx_hash);
    assert!(verify_signature(
        signer.verifying_key().to_bytes(),
        &original.tx_hash,
        &sig_for_rebuilt.to_bytes()
    )
    .is_err());
}

#[test]
fn two_signatures_attach_and_the_envelope_still_encodes() {
    let tx = build_payment(&spec(), &Network::testnet()).unwrap();
    let signers = [key(5), key(6)];
    let collected: Vec<CollectedSignature> = signers
        .iter()
        .map(|s| CollectedSignature {
            signer_public_key: s.verifying_key().to_bytes(),
            signature: s.sign(&tx.tx_hash).to_bytes(),
        })
        .collect();

    let signed = attach_signatures(&tx, &collected).unwrap();
    let TransactionEnvelope::Tx(v1) = &signed else {
        panic!("expected a v1 envelope");
    };
    assert_eq!(v1.signatures.len(), 2);

    // Each hint is the last four bytes of its signer's key, which is how the network matches them.
    for (i, s) in signers.iter().enumerate() {
        let expected = settle_chain::signature_hint(s.verifying_key().to_bytes());
        assert_eq!(v1.signatures[i].hint.0, expected);
    }

    let xdr = signed.to_xdr_base64(Limits::none()).unwrap();
    assert!(!xdr.is_empty());
    // It decodes back to the same thing, which is what the CLI and Horizon will do with it.
    let round_tripped = TransactionEnvelope::from_xdr_base64(&xdr, Limits::none()).unwrap();
    assert_eq!(round_tripped, signed);
}

#[test]
fn attaching_signatures_does_not_change_the_transaction_being_signed() {
    let tx = build_payment(&spec(), &Network::testnet()).unwrap();
    let signer = key(5);
    let signed = attach_signatures(
        &tx,
        &[CollectedSignature {
            signer_public_key: signer.verifying_key().to_bytes(),
            signature: signer.sign(&tx.tx_hash).to_bytes(),
        }],
    )
    .unwrap();

    let (TransactionEnvelope::Tx(before), TransactionEnvelope::Tx(after)) = (&tx.envelope, &signed)
    else {
        panic!("expected v1 envelopes");
    };
    assert_eq!(before.tx, after.tx);
}

#[test]
fn a_fee_bump_leaves_the_inner_signatures_untouched() {
    // The whole point of C7: the approvers are not asked again.
    let tx = build_payment(&spec(), &Network::testnet()).unwrap();
    let signer = key(5);
    let signed = attach_signatures(
        &tx,
        &[CollectedSignature {
            signer_public_key: signer.verifying_key().to_bytes(),
            signature: signer.sign(&tx.tx_hash).to_bytes(),
        }],
    )
    .unwrap();

    let bumped = fee_bump(&signed, &address(7), 10_000).unwrap();
    let TransactionEnvelope::TxFeeBump(outer) = &bumped else {
        panic!("expected a fee-bump envelope");
    };
    let stellar_xdr::curr::FeeBumpTransactionInnerTx::Tx(inner) = &outer.tx.inner_tx;
    assert_eq!(inner.signatures.len(), 1);
    assert_eq!(outer.tx.fee, 10_000);
}

#[test]
fn a_native_xlm_payment_needs_no_issuer() {
    let mut s = spec();
    s.asset = Asset::Xlm;
    s.asset_issuer = None;
    assert!(build_payment(&s, &Network::testnet()).is_ok());
}

#[test]
fn a_usdc_payment_without_an_issuer_is_refused() {
    let mut s = spec();
    s.asset_issuer = None;
    assert!(build_payment(&s, &Network::testnet()).is_err());
}

#[test]
fn a_malformed_destination_is_refused_before_anything_is_built() {
    let mut s = spec();
    s.destination = "not-an-address".into();
    assert!(build_payment(&s, &Network::testnet()).is_err());
}
