use super::*;

fn id(n: u8) -> [u8; 32] {
    [n; 32]
}
/// Placeholder canonical-Address-XDR bytes for a buyer (variable length, like a real
/// ScAddress XDR). Byte-exact parity with the contract's `addr.to_xdr` is tested in the host.
fn buyer_xdr(n: u8) -> Vec<u8> {
    let mut v = vec![0u8; 40];
    v[0] = 0x12;
    v[39] = n;
    v
}
fn holder(n: u8, balance: i128) -> Holder {
    Holder { id: id(n), balance, has_trustline: true, frozen: false }
}
fn pay(holder_n: u8, amount: i128, at: u64) -> Payment {
    Payment { holder: id(holder_n), amount, paid_at: at, clawed_back: false }
}
fn pos(buyer_n: u8, cover: i128) -> Position {
    Position { buyer: buyer_xdr(buyer_n), cover, salt: [buyer_n; 32] }
}

/// Build a fully VALID (binding-passing) Inputs: the config commits to the supplied snapshot
/// + rate + deadline, and `instrument_id` is the genuine derivation. Tests tweak one field to
/// exercise a specific rule or rejection.
fn mk(
    holders: Vec<Holder>,
    rate_bps: u32,
    deadline: u64,
    collateral: i128,
    payments: Vec<Payment>,
    positions: Vec<Position>,
) -> Inputs {
    let terms = Terms { coupon_rate_bps: rate_bps };
    let sroot = snapshot_root(&holders);
    let proot = position_root(&positions);
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: sroot,
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, deadline)],
    };
    let type_id_xdr = vec![0xCC, 1, 2, 3, 4];
    let rules_version = 1u32;
    let instrument_id = derive_instrument_id(&type_id_xdr, rules_version, &config_hash(&config));
    Inputs {
        type_id_xdr,
        rules_version,
        config,
        instrument_id,
        epoch: 1,
        deadline,
        terms,
        collateral,
        snapshot: holders,
        payments,
        positions,
        position_root: proot,
    }
}

fn alloc(buyer_n: u8, amount: i128) -> Allocation {
    Allocation { buyer: buyer_xdr(buyer_n), amount }
}

// ---------------- published-rule coverage ----------------

#[test]
fn full_miss_pays_full_cover() {
    let inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    let (allocs, j) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![alloc(100, 800)]);
    assert_eq!(j.total_payout, 800);
    assert_eq!(j.instrument_id, inp.instrument_id);
}

#[test]
fn partial_default_pays_pro_rata() {
    let holders: Vec<Holder> = (1..=10).map(|n| holder(n, 1000)).collect();
    let payments: Vec<Payment> = (1..=7).map(|n| pay(n, 100, 499)).collect();
    let inp = mk(holders, 1000, 500, 1000, payments, vec![pos(100, 1000)]);
    let (allocs, _) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![alloc(100, 300)]);
}

#[test]
fn short_pay_is_partial_shortfall() {
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 60, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&inp).unwrap().0, vec![alloc(100, 40)]);
}

#[test]
fn fully_paid_is_unprovable() {
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 100, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&inp), Err(SettleError::NoDefault));
}

#[test]
fn payment_at_deadline_qualifies_after_does_not() {
    let at_deadline = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 100, 500)], vec![pos(100, 100)]);
    assert_eq!(settle(&at_deadline), Err(SettleError::NoDefault));
    let late = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 100, 501)], vec![pos(100, 100)]);
    assert_eq!(settle(&late).unwrap().0, vec![alloc(100, 100)]);
}

#[test]
fn clawed_back_payment_counts_as_shortfall() {
    let mut p = pay(1, 100, 400);
    p.clawed_back = true;
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![p], vec![pos(100, 100)]);
    assert_eq!(settle(&inp).unwrap().0, vec![alloc(100, 100)]);
}

#[test]
fn frozen_holder_counts_as_shortfall() {
    let mut h = holder(1, 1000);
    h.frozen = true;
    let inp = mk(vec![h], 1000, 500, 100, vec![pay(1, 100, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&inp).unwrap().0, vec![alloc(100, 100)]);
}

#[test]
fn no_trustline_holder_excluded_from_owed() {
    let h1 = holder(1, 1000);
    let mut h2 = holder(2, 1000);
    h2.has_trustline = false;
    let inp = mk(vec![h1, h2], 1000, 500, 100, vec![], vec![pos(100, 100)]);
    assert_eq!(settle(&inp).unwrap().0, vec![alloc(100, 100)]); // Σowed=100, not 200
}

#[test]
fn payout_never_exceeds_cover() {
    let holders: Vec<Holder> = (1..=10).map(|n| holder(n, 1000)).collect();
    let inp = mk(holders, 1000, 500, 1000, vec![], vec![pos(100, 250)]);
    assert_eq!(settle(&inp).unwrap().0[0].amount, 250);
}

#[test]
fn total_payout_exceeding_collateral_is_insolvent() {
    let inp = mk(vec![holder(1, 10_000)], 1000, 500, 500, vec![], vec![pos(100, 1000)]);
    assert_eq!(settle(&inp), Err(SettleError::Insolvent));
}

#[test]
fn negative_committed_balance_rejected() {
    let inp = mk(vec![holder(1, -1000), holder(2, 1000)], 1000, 500, 1000, vec![], vec![pos(100, 100)]);
    assert_eq!(settle(&inp), Err(SettleError::BadInput));
}

#[test]
fn negative_payment_amount_rejected() {
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 1000, vec![pay(1, -50, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&inp), Err(SettleError::BadInput));
}

#[test]
fn non_positive_cover_rejected() {
    let inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 0)]);
    assert_eq!(settle(&inp), Err(SettleError::BadInput));
}

#[test]
fn mixed_payment_representations_aggregate() {
    let paid = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 40, 300), pay(1, 60, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&paid), Err(SettleError::NoDefault));
    let short = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 40, 300), pay(1, 50, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&short).unwrap().0, vec![alloc(100, 10)]);
}

#[test]
fn journal_serializes_to_116_byte_layout() {
    let inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    let (_, j) = settle(&inp).unwrap();
    let b = j.to_bytes();
    assert_eq!(b.len(), 116);
    assert_eq!(&b[0..32], &j.instrument_id);
    assert_eq!(&b[32..36], &j.epoch.to_be_bytes());
    assert_eq!(&b[36..44], &j.deadline.to_be_bytes());
    assert_eq!(&b[44..76], &j.position_root);
    assert_eq!(&b[76..108], &j.allocation_root);
    assert_eq!(&b[108..116], &j.total_payout.to_be_bytes());
}

#[test]
fn commitment_and_root_are_deterministic() {
    let c1 = commitment(&buyer_xdr(7), 600, &[7; 32]);
    assert_eq!(c1, commitment(&buyer_xdr(7), 600, &[7; 32]));
    assert_ne!(c1, commitment(&buyer_xdr(7), 601, &[7; 32]));
    assert_eq!(position_root(&[pos(7, 600)]), sha256_fold(&[0u8; 32], &c1));
}

// ---------------- soundness binding (review M1/M2) ----------------

#[test]
fn tampered_instrument_id_rejected() {
    let mut inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    inp.instrument_id = [0xFF; 32];
    assert_eq!(settle(&inp), Err(SettleError::InstrumentMismatch));
}

#[test]
fn fabricated_config_changes_instrument_id() {
    // Swap the committed snapshot_root but keep the real instrument_id -> derivation mismatch.
    // This is the M1 exploit: a fabricated snapshot can't be passed off under the real instrument.
    let mut inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    inp.config.snapshot_root = [0xAB; 32];
    assert_eq!(settle(&inp), Err(SettleError::InstrumentMismatch));
}

#[test]
fn snapshot_not_matching_committed_root_rejected() {
    // Config + instrument_id valid, but the supplied snapshot doesn't hash to the commitment.
    let mut inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    inp.snapshot = vec![holder(1, 999_999)]; // different from what config.snapshot_root commits
    assert_eq!(settle(&inp), Err(SettleError::SnapshotMismatch));
}

#[test]
fn tampered_coupon_rate_rejected() {
    // Inflating the rate to manufacture a default fails: terms no longer open terms_hash.
    let mut inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    inp.terms.coupon_rate_bps = 9999;
    assert_eq!(settle(&inp), Err(SettleError::TermsMismatch));
}

#[test]
fn deadline_not_matching_committed_schedule_rejected() {
    let mut inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    inp.deadline = 999; // committed schedule has epoch 1 -> 500
    assert_eq!(settle(&inp), Err(SettleError::DeadlineMismatch));
}

#[test]
fn tampered_position_root_rejected() {
    let mut inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    inp.position_root = [0xBB; 32];
    assert_eq!(settle(&inp), Err(SettleError::PositionMismatch));
}
