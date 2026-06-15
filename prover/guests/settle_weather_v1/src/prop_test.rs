//! Property tests (proptest): fuzz the published weather_v1 rule over randomized rainfall and
//! assert the invariants always hold. `#[cfg(test)]` keeps these out of the guest ELF.
#![cfg(test)]
use super::*;
use proptest::prelude::*;

fn wbuild(readings: &[u32], trigger: u32, exhaust: u32, covers: &[i128], collateral: i128) -> Inputs {
    let params = WeatherParams { station_id: [9; 32], window_start: 100, window_end: 1_000_000 };
    let terms = Terms { trigger_mm: trigger, exhaust_mm: exhaust };
    let observations: Vec<Observation> = readings
        .iter()
        .enumerate()
        .map(|(i, &mm)| Observation { station: [9; 32], mm, observed_at: 200 + i as u64 })
        .collect();
    let positions: Vec<Position> = covers
        .iter()
        .enumerate()
        .map(|(i, &c)| Position { buyer: vec![(i as u8).wrapping_add(1); 40], cover: c, salt: [i as u8; 32] })
        .collect();
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&params),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 150,
        epoch_deadlines: vec![(1u32, 1_000_000u64)],
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
        deadline: 1_000_000,
        terms,
        params,
        collateral,
        observations,
        positions,
        position_root,
    }
}

proptest! {
    // INVARIANT: Σ payouts ≤ collateral and ≤ Σ cover; severity never pays more than the cover.
    #[test]
    fn weather_payouts_bounded(
        readings in prop::collection::vec(0u32..10_000u32, 0..8),
        exhaust in 0u32..5_000u32,
        span in 1u32..5_000u32,
        covers in prop::collection::vec(1i128..1_000_000i128, 1..4),
        collateral in 1i128..100_000_000i128,
    ) {
        let trigger = exhaust + span; // trigger strictly above exhaust: a valid band
        let inputs = wbuild(&readings, trigger, exhaust, &covers, collateral);
        match settle(&inputs) {
            Ok((allocs, j)) => {
                let sum: i128 = allocs.iter().map(|a| a.amount).sum();
                let total_cover: i128 = covers.iter().sum();
                prop_assert_eq!(sum as u64, j.total_payout);
                prop_assert!(sum <= collateral, "Σ payouts {} > collateral {}", sum, collateral);
                prop_assert!(sum <= total_cover, "Σ payouts {} > Σ cover {}", sum, total_cover);
            }
            Err(SettleError::NoBreach) | Err(SettleError::Insolvent) | Err(SettleError::Overflow) => {}
            Err(e) => prop_assert!(false, "unexpected error {:?}", e),
        }
    }

    // INVARIANT: rainfall meeting/exceeding the trigger is UNPROVABLE (no breach → no payout).
    #[test]
    fn weather_no_breach_when_rainfall_meets_trigger(
        readings in prop::collection::vec(0u32..10_000u32, 1..8),
    ) {
        let observed: u64 = readings.iter().map(|&r| r as u64).sum();
        prop_assume!(observed >= 1);
        let trigger = observed as u32; // observed == trigger ⇒ no breach
        let inputs = wbuild(&readings, trigger, 0, &[800], 100_000_000);
        prop_assert_eq!(settle(&inputs), Err(SettleError::NoBreach));
    }

    // INVARIANT: determination is deterministic.
    #[test]
    fn weather_settle_is_deterministic(
        readings in prop::collection::vec(0u32..10_000u32, 0..8),
        exhaust in 0u32..2_000u32,
        span in 1u32..3_000u32,
        covers in prop::collection::vec(1i128..100_000i128, 1..3),
    ) {
        let inputs = wbuild(&readings, exhaust + span, exhaust, &covers, 100_000_000);
        prop_assert_eq!(settle(&inputs), settle(&inputs));
    }
}
