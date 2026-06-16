#![cfg(test)]
use super::*;
use ed25519_dalek::{Signer, SigningKey};

fn issuer() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32]) // deterministic; no rng
}
fn holder(b: u8, bal: i128) -> Holder {
    Holder { id: [b; 32], balance: bal, has_trustline: true, frozen: false }
}
fn paid(id: u8, amount: i128) -> Payment {
    Payment { holder: [id; 32], amount, paid_at: 400, clawed_back: false }
}

/// Assemble a credit_v3 witness. The guest sees `(epoch, snapshot, payments)`; the issuer actually
/// signed `(sign_epoch, sign_snapshot, sign_payments)`. They line up (valid) unless a test diverges
/// them. `config.snapshot_root` is arbitrary here — credit_v3 does NOT pin the snapshot to it.
#[allow(clippy::too_many_arguments)]
fn assemble(
    epoch: u32,
    rate: u32,
    snapshot: Vec<Holder>,
    payments: Vec<Payment>,
    sign_epoch: u32,
    sign_snapshot: &[Holder],
    sign_payments: &[Payment],
    committed_pk: [u8; 32],
    signer: &SigningKey,
) -> Inputs {
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let terms = Terms { coupon_rate_bps: rate, issuer_pubkey: committed_pk };
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: [0x33; 32], // arbitrary — credit_v3 attests the per-epoch snapshot instead
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64), (2u32, 1000u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    let attestation = signer
        .sign(&record_date_msg(sign_epoch, sign_snapshot, sign_payments))
        .to_bytes()
        .to_vec();
    Inputs {
        type_id_xdr,
        rules_version: 1,
        config,
        instrument_id,
        epoch,
        deadline: if epoch == 1 { 500 } else { 1000 },
        terms,
        collateral: 2000,
        snapshot,
        payments,
        attestation,
        positions: positions.clone(),
        position_root: position_root(&positions),
    }
}

#[test]
fn attested_record_date_default_settles() {
    let sk = issuer();
    let snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let pays = vec![paid(2, 1000)]; // holder 1 unpaid -> Σowed 2000, Σshort 1000, severity 0.5
    let (allocs, j) = settle(&assemble(1, 1000, snap.clone(), pays.clone(), 1, &snap, &pays, sk.verifying_key().to_bytes(), &sk))
        .expect("attested record-date data must settle");
    assert_eq!(allocs.len(), 1);
    assert_eq!(allocs[0].amount, 400, "cover 800 × 0.5 severity");
    assert_eq!(j.total_payout, 400);
}

#[test]
fn tampered_holder_set_rejected() {
    // THE record-date guarantee: the keeper changes the holder set the guest settles over, but the
    // issuer signed the ORIGINAL record-date set -> the digest no longer matches -> AttestationInvalid.
    let sk = issuer();
    let signed_snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let pays = vec![paid(2, 1000)];
    // guest sees an EXTRA holder (3) the issuer never attested for this record date
    let tampered_snap = vec![holder(1, 10_000), holder(2, 10_000), holder(3, 10_000)];
    let inputs = assemble(1, 1000, tampered_snap, pays.clone(), 1, &signed_snap, &pays, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn tampered_payments_rejected() {
    let sk = issuer();
    let snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let signed_pays = vec![paid(2, 1000)];
    let inputs = assemble(1, 1000, snap.clone(), vec![], 1, &snap, &signed_pays, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn cross_epoch_replay_rejected() {
    // the issuer signed epoch 1's (snapshot+payments); resubmitting it as epoch 2 fails — the epoch
    // is in the signed message, so an attestation cannot be replayed across record dates.
    let sk = issuer();
    let snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let pays = vec![paid(2, 1000)];
    let inputs = assemble(2, 1000, snap.clone(), pays.clone(), 1, &snap, &pays, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn signature_by_wrong_key_rejected() {
    let issuer = issuer();
    let impostor = SigningKey::from_bytes(&[7u8; 32]);
    let snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let pays = vec![paid(2, 1000)];
    let inputs = assemble(1, 1000, snap.clone(), pays.clone(), 1, &snap, &pays, issuer.verifying_key().to_bytes(), &impostor);
    assert_eq!(settle(&inputs), Err(SettleError::AttestationInvalid));
}

#[test]
fn fully_paid_attested_is_unprovable() {
    let sk = issuer();
    let snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let full = vec![paid(1, 1000), paid(2, 1000)];
    let inputs = assemble(1, 1000, snap.clone(), full.clone(), 1, &snap, &full, sk.verifying_key().to_bytes(), &sk);
    assert_eq!(settle(&inputs), Err(SettleError::NoDefault));
}

#[test]
fn terms_bind_the_issuer_key() {
    let sk = issuer();
    let snap = vec![holder(1, 10_000), holder(2, 10_000)];
    let pays = vec![paid(2, 1000)];
    let mut inputs = assemble(1, 1000, snap.clone(), pays.clone(), 1, &snap, &pays, sk.verifying_key().to_bytes(), &sk);
    inputs.terms.issuer_pubkey = [0xEE; 32]; // no longer opens the committed terms_hash
    assert_eq!(settle(&inputs), Err(SettleError::TermsMismatch));
}

#[test]
fn different_record_date_sets_settle_across_epochs() {
    // The headline: the holder set is NOT fixed at issuance. Epoch 1 has holders {1,2}; epoch 2 has
    // a DIFFERENT traded set {3,4}. Both settle when the issuer attests that epoch's record date.
    let sk = issuer();
    let pk = sk.verifying_key().to_bytes();

    let snap1 = vec![holder(1, 10_000), holder(2, 10_000)];
    let pays1 = vec![paid(2, 1000)];
    let (_a1, j1) = settle(&assemble(1, 1000, snap1.clone(), pays1.clone(), 1, &snap1, &pays1, pk, &sk)).unwrap();
    assert_eq!(j1.total_payout, 400);

    let snap2 = vec![holder(3, 10_000), holder(4, 10_000)]; // the bond traded; new holders on record
    let pays2 = vec![paid(4, 1000)];
    let (_a2, j2) = settle(&assemble(2, 1000, snap2.clone(), pays2.clone(), 2, &snap2, &pays2, pk, &sk)).unwrap();
    assert_eq!(j2.epoch, 2);
    assert_eq!(j2.total_payout, 400, "same severity, a wholly different record-date holder set");
}
