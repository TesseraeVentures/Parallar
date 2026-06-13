use super::*;

fn id(n: u8) -> [u8; 32] {
    [n; 32]
}
fn holder(n: u8, balance: i128) -> Holder {
    Holder { id: id(n), balance, has_trustline: true, frozen: false }
}
fn pay(holder_n: u8, amount: i128, at: u64) -> Payment {
    Payment { holder: id(holder_n), amount, paid_at: at, clawed_back: false }
}
fn pos(buyer_n: u8, cover: i128) -> Position {
    Position { buyer: id(buyer_n), cover, salt: [buyer_n; 32] }
}
fn mk(
    holders: Vec<Holder>,
    rate_bps: u32,
    deadline: u64,
    collateral: i128,
    payments: Vec<Payment>,
    positions: Vec<Position>,
) -> Inputs {
    Inputs {
        instrument_id: [9u8; 32],
        epoch: 1,
        deadline,
        coupon_rate_bps: rate_bps,
        collateral,
        snapshot_root: snapshot_root(&holders),
        snapshot: holders,
        payments,
        position_root: position_root(&positions),
        positions,
    }
}

#[test]
fn full_miss_pays_full_cover() {
    // 1 holder owed 1000 (10% of 10_000), unpaid. Buyer cover 800.
    let inp = mk(vec![holder(1, 10_000)], 1000, 500, 1000, vec![], vec![pos(100, 800)]);
    let (allocs, j) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 800 }]);
    assert_eq!(j.total_payout, 800);
    assert_eq!(j.position_root, position_root(&inp.positions));
}

#[test]
fn partial_default_pays_pro_rata() {
    // 10 holders owed 100 each (Σowed=1000); 7 paid, 3 short (Σshort=300). Cover 1000 -> 300.
    let holders: Vec<Holder> = (1..=10).map(|n| holder(n, 1000)).collect();
    let payments: Vec<Payment> = (1..=7).map(|n| pay(n, 100, 499)).collect();
    let inp = mk(holders, 1000, 500, 1000, payments, vec![pos(100, 1000)]);
    let (allocs, j) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 300 }]);
    assert_eq!(j.total_payout, 300);
}

#[test]
fn short_pay_is_partial_shortfall() {
    // owed 100, paid 60 -> short 40.
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 60, 400)], vec![pos(100, 100)]);
    let (allocs, _) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 40 }]);
}

#[test]
fn fully_paid_is_unprovable() {
    // owed 100, paid 100 -> Σshort 0 -> NoDefault (no proof can exist).
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 100, 400)], vec![pos(100, 100)]);
    assert_eq!(settle(&inp), Err(SettleError::NoDefault));
}

#[test]
fn payment_at_deadline_qualifies_after_does_not() {
    // at == deadline -> qualifies (fully paid -> NoDefault)
    let paid_at_deadline = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 100, 500)], vec![pos(100, 100)]);
    assert_eq!(settle(&paid_at_deadline), Err(SettleError::NoDefault));

    // at == deadline+1 -> does NOT qualify -> short 100 -> default
    let paid_late = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![pay(1, 100, 501)], vec![pos(100, 100)]);
    let (allocs, _) = settle(&paid_late).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 100 }]);
}

#[test]
fn clawed_back_payment_counts_as_shortfall() {
    let mut p = pay(1, 100, 400);
    p.clawed_back = true; // issuer paid then clawed back -> does not qualify
    let inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![p], vec![pos(100, 100)]);
    let (allocs, _) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 100 }]);
}

#[test]
fn frozen_holder_counts_as_shortfall() {
    // frozen holder: owed is in Σowed, but paid is forced to 0 even with a payment record.
    let mut h = holder(1, 1000);
    h.frozen = true;
    let inp = mk(vec![h], 1000, 500, 100, vec![pay(1, 100, 400)], vec![pos(100, 100)]);
    let (allocs, _) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 100 }]); // full owed = shortfall
}

#[test]
fn no_trustline_holder_excluded_from_owed() {
    // h1 owed 100 and short; h2 has NO trustline (balance 1000) -> excluded from Σowed.
    let h1 = holder(1, 1000);
    let mut h2 = holder(2, 1000);
    h2.has_trustline = false;
    // Σowed = 100 (only h1). Σshort = 100. Cover 100 -> payout = 100*100/100 = 100.
    // If h2 were included Σowed would be 200 and payout would be 50 — so 100 proves exclusion.
    let inp = mk(vec![h1, h2], 1000, 500, 100, vec![], vec![pos(100, 100)]);
    let (allocs, _) = settle(&inp).unwrap();
    assert_eq!(allocs, vec![Allocation { buyer: id(100), amount: 100 }]);
}

#[test]
fn payout_never_exceeds_cover() {
    // Σshort ≤ Σowed always, so pro-rata ≤ cover; the cap is a defensive bound. Verify ≤ cover.
    let holders: Vec<Holder> = (1..=10).map(|n| holder(n, 1000)).collect();
    let inp = mk(holders, 1000, 500, 1000, vec![], vec![pos(100, 250)]); // full miss, cover 250
    let (allocs, _) = settle(&inp).unwrap();
    assert_eq!(allocs[0].amount, 250); // 250 * 1000/1000 = 250, == cover, not exceeded
}

#[test]
fn total_payout_exceeding_collateral_is_insolvent() {
    // full miss, cover 1000, but collateral only 500 -> Σ payouts > collateral -> Insolvent.
    let inp = mk(vec![holder(1, 10_000)], 1000, 500, 500, vec![], vec![pos(100, 1000)]);
    assert_eq!(settle(&inp), Err(SettleError::Insolvent));
}

#[test]
fn tampered_snapshot_root_rejected() {
    let mut inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![], vec![pos(100, 100)]);
    inp.snapshot_root = [0xAA; 32];
    assert_eq!(settle(&inp), Err(SettleError::SnapshotMismatch));
}

#[test]
fn tampered_position_root_rejected() {
    let mut inp = mk(vec![holder(1, 1000)], 1000, 500, 100, vec![], vec![pos(100, 100)]);
    inp.position_root = [0xBB; 32];
    assert_eq!(settle(&inp), Err(SettleError::PositionMismatch));
}

#[test]
fn commitment_and_root_are_deterministic() {
    let c1 = commitment(&id(7), 600, &[7; 32]);
    let c2 = commitment(&id(7), 600, &[7; 32]);
    assert_eq!(c1, c2);
    assert_ne!(c1, commitment(&id(7), 601, &[7; 32])); // cover changes the commitment
    // position_root over a single position == sha256(zeros ‖ commitment)
    assert_eq!(position_root(&[pos(7, 600)]), {
        let c = commitment(&id(7), 600, &[7; 32]);
        let mut h = Sha256::new();
        h.update([0u8; 32]);
        h.update(c);
        let out: [u8; 32] = h.finalize().into();
        out
    });
}
