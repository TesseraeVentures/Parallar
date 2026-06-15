#![cfg(test)]
use super::*;
use ed25519_dalek::{Signer, SigningKey};

fn issuer() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32]) // deterministic; no rng
}
fn holder(b: u8, bal: i128) -> Holder {
    Holder { id: [b; 32], balance: bal, has_trustline: true, frozen: false }
}

/// Assemble a credit_v2 witness. `payments` is what the guest sees; `sign_over` is what the
/// issuer actually signed; `committed_pk` is the key bound in terms; `signer` produces the sig.
/// Defaults line up (valid) unless a test deliberately diverges them.
fn assemble(
    rate: u32,
    payments: Vec<Payment>,
    sign_over: &[Payment],
    committed_pk: [u8; 32],
    signer: &SigningKey,
) -> Inputs {
    let snapshot = vec![holder(1, 10_000), holder(2, 10_000)];
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let terms = Terms { coupon_rate_bps: rate, issuer_pubkey: committed_pk };
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&snapshot),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    let attestation = signer.sign(&payments_digest(sign_over)).to_bytes().to_vec();
    Inputs {
        type_id_xdr,
        rules_version: 1,
        config,
        instrument_id,
        epoch: 1,
        deadline: 500,
        terms,
        collateral: 2000,
        snapshot,
        payments,
        attestation,
        positions: positions.clone(),
        position_root: position_root(&positions),
    }
}

// holder 2 paid in full (1000), holder 1 unpaid -> Σ owed 2000, Σ shortfall 1000, severity 0.5.
fn paid_h2() -> Vec<Payment> {
    vec![Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }]
}

#[test]
fn attested_partial_default_settles() {
    let sk = issuer();
    let p = paid_h2();
    let (allocs, j) = settle(&assemble(1000, p.clone(), &p, sk.verifying_key().to_bytes(), &sk))
        .expect("attested data must settle");
    assert_eq!(allocs.len(), 1);
    assert_eq!(allocs[0].amount, 400, "cover 800 × 0.5 severity");
    assert_eq!(j.total_payout, 400);
}

#[test]
fn tampered_payments_rejected_removed() {
    // keeper REMOVES holder 2's payment (to inflate the default) but the issuer signed the
    // ORIGINAL set -> the digest no longer matches -> AttestationInvalid.
    let sk = issuer();
    let signed = paid_h2();
    let inputs = assemble(1000, vec![], &signed, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn tampered_payments_rejected_injected() {
    // keeper INJECTS a payment the issuer never signed -> digest mismatch -> AttestationInvalid.
    let sk = issuer();
    let signed = paid_h2();
    let mut injected = paid_h2();
    injected.push(Payment { holder: [1; 32], amount: 1000, paid_at: 400, clawed_back: false });
    let inputs = assemble(1000, injected, &signed, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn signature_by_wrong_key_rejected() {
    // committed key is the issuer's, but the signature is by a different key -> AttestationInvalid.
    let issuer = issuer();
    let impostor = SigningKey::from_bytes(&[7u8; 32]);
    let p = paid_h2();
    let inputs = assemble(1000, p.clone(), &p, issuer.verifying_key().to_bytes(), &impostor);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn fully_paid_attested_is_unprovable() {
    // both holders paid in full, properly attested -> reaches determination -> NoDefault.
    let sk = issuer();
    let full = vec![
        Payment { holder: [1; 32], amount: 1000, paid_at: 400, clawed_back: false },
        Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false },
    ];
    let inputs = assemble(1000, full.clone(), &full, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::NoDefault));
}

#[test]
fn terms_bind_the_issuer_key() {
    // tampering the committed issuer key (without re-deriving instrument_id) breaks terms_hash.
    let sk = issuer();
    let p = paid_h2();
    let mut inputs = assemble(1000, p.clone(), &p, sk.verifying_key().to_bytes(), &sk);
    inputs.terms.issuer_pubkey = [0xEE; 32]; // no longer opens the committed terms_hash
    assert_eq!(settle(&inputs), Err(SettleError::TermsMismatch));
}

#[test]
fn determination_matches_credit_v1_for_the_same_attested_data() {
    use settle_credit_v1 as v1;
    let sk = issuer();
    let p = paid_h2();
    let v2_inputs = assemble(1000, p.clone(), &p, sk.verifying_key().to_bytes(), &sk);
    let (_v2_allocs, v2_j) = settle(&v2_inputs).unwrap();

    // the same snapshot/payments/positions/rate under credit_v1 (no attestation) -> same payout
    let snapshot = vec![holder(1, 10_000), holder(2, 10_000)]
        .into_iter()
        .map(|h| v1::Holder { id: h.id, balance: h.balance, has_trustline: h.has_trustline, frozen: h.frozen })
        .collect::<Vec<_>>();
    let positions = vec![v1::Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let terms = v1::Terms { coupon_rate_bps: 1000 };
    let config = v1::ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: v1::terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: v1::snapshot_root(&snapshot),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let v1_inputs = v1::Inputs {
        type_id_xdr: type_id_xdr.clone(),
        rules_version: 1,
        instrument_id: v1::derive_instrument_id(&type_id_xdr, 1, &v1::config_hash(&config)),
        config,
        epoch: 1,
        deadline: 500,
        terms,
        collateral: 2000,
        snapshot,
        payments: vec![v1::Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }],
        positions,
        position_root: v1::position_root(&[v1::Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }]),
    };
    let (_v1_allocs, v1_j) = v1::settle(&v1_inputs).unwrap();
    assert_eq!(v2_j.total_payout, v1_j.total_payout, "v2's determination matches v1's");
}
