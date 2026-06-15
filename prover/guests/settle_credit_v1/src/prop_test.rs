//! Property tests (proptest): fuzz the published credit_v1 rule over randomized books and
//! assert the invariants ALWAYS hold — the audit-grade complement to the example-based tests.
//! These run natively; `#[cfg(test)]` keeps them out of the guest ELF (image_id unaffected).
#![cfg(test)]
use super::*;
use proptest::prelude::*;

/// Build a fully config-bound `Inputs` from raw components (all soundness bindings satisfied,
/// so settle() reaches the determination rather than failing a binding check).
fn build(balances: &[i128], rate: u32, paid: &[i128], covers: &[i128], collateral: i128) -> Inputs {
    let deadline = 500u64;
    let snapshot: Vec<Holder> = balances
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
            Holder { id, balance: b, has_trustline: true, frozen: false }
        })
        .collect();
    let payments: Vec<Payment> = paid
        .iter()
        .enumerate()
        .filter(|(_, &p)| p > 0)
        .map(|(i, &p)| {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
            Payment { holder: id, amount: p, paid_at: deadline, clawed_back: false }
        })
        .collect();
    let positions: Vec<Position> = covers
        .iter()
        .enumerate()
        .map(|(i, &c)| Position { buyer: vec![(i as u8).wrapping_add(1); 40], cover: c, salt: [i as u8; 32] })
        .collect();
    let terms = Terms { coupon_rate_bps: rate };
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&snapshot),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, deadline)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    let position_root = position_root(&positions);
    Inputs {
        type_id_xdr,
        rules_version: 1,
        config,
        instrument_id,
        epoch: 1,
        deadline,
        terms,
        collateral,
        snapshot,
        payments,
        positions,
        position_root,
    }
}

proptest! {
    // INVARIANT: Σ payouts ≤ collateral, Σ payouts ≤ Σ cover, and total_payout is the exact sum.
    // The only acceptable non-Ok outcomes are NoDefault (fully paid) / Insolvent / Overflow.
    #[test]
    fn payouts_bounded_by_collateral_and_cover(
        balances in prop::collection::vec(1i128..1_000_000i128, 1..6),
        rate in 1u32..=10_000u32,
        fracs in prop::collection::vec(0u32..=100u32, 6),
        covers in prop::collection::vec(1i128..1_000_000i128, 1..4),
        collateral in 1i128..100_000_000i128,
    ) {
        let paid: Vec<i128> = balances.iter().enumerate().map(|(i, &b)| {
            let owed = b.saturating_mul(rate as i128) / 10_000;
            owed * (fracs[i % fracs.len()] as i128) / 100
        }).collect();
        let inputs = build(&balances, rate, &paid, &covers, collateral);
        match settle(&inputs) {
            Ok((allocs, j)) => {
                let sum: i128 = allocs.iter().map(|a| a.amount).sum();
                let total_cover: i128 = covers.iter().sum();
                prop_assert_eq!(sum as u64, j.total_payout);
                prop_assert!(sum <= collateral, "Σ payouts {} > collateral {}", sum, collateral);
                prop_assert!(sum <= total_cover, "Σ payouts {} > Σ cover {}", sum, total_cover);
                prop_assert!(allocs.iter().all(|a| a.amount > 0), "no zero payouts emitted");
            }
            Err(SettleError::NoDefault) | Err(SettleError::Insolvent) | Err(SettleError::Overflow) => {}
            Err(e) => prop_assert!(false, "unexpected error {:?}", e),
        }
    }

    // INVARIANT: a fully-paid book is UNPROVABLE — every holder paid exactly its owed → NoDefault.
    #[test]
    fn fully_paid_is_always_nodefault(
        balances in prop::collection::vec(1i128..1_000_000i128, 1..6),
        rate in 1u32..=10_000u32,
        covers in prop::collection::vec(1i128..1_000i128, 1..3),
    ) {
        let paid: Vec<i128> = balances.iter().map(|&b| b.saturating_mul(rate as i128) / 10_000).collect();
        let inputs = build(&balances, rate, &paid, &covers, 100_000_000);
        prop_assert_eq!(settle(&inputs), Err(SettleError::NoDefault));
    }

    // INVARIANT: determination is deterministic — same inputs, same result.
    #[test]
    fn settle_is_deterministic(
        balances in prop::collection::vec(1i128..1_000_000i128, 1..6),
        rate in 1u32..=10_000u32,
        fracs in prop::collection::vec(0u32..=100u32, 6),
        covers in prop::collection::vec(1i128..1_000_000i128, 1..4),
    ) {
        let paid: Vec<i128> = balances.iter().enumerate()
            .map(|(i, &b)| b.saturating_mul(rate as i128) / 10_000 * (fracs[i % fracs.len()] as i128) / 100)
            .collect();
        let inputs = build(&balances, rate, &paid, &covers, 100_000_000);
        prop_assert_eq!(settle(&inputs), settle(&inputs));
    }

    // INVARIANT: a tampered position_root is ALWAYS rejected (the binding holds for any tweak).
    #[test]
    fn tampered_position_root_always_rejected(
        balances in prop::collection::vec(1i128..1_000_000i128, 1..5),
        rate in 1u32..=10_000u32,
        covers in prop::collection::vec(1i128..10_000i128, 1..3),
        tweak in any::<[u8; 32]>(),
    ) {
        let paid: Vec<i128> = balances.iter().map(|_| 0i128).collect(); // full default → reaches payout
        let mut inputs = build(&balances, rate, &paid, &covers, 100_000_000);
        prop_assume!(tweak != inputs.position_root);
        inputs.position_root = tweak;
        prop_assert_eq!(settle(&inputs), Err(SettleError::PositionMismatch));
    }
}
