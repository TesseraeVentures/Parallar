//! Property tests (proptest) over the guest determination rules — fuzz randomized books and
//! assert the invariants ALWAYS hold. Lives in its own crate (not in the guests) so the guest
//! ELFs / image_ids stay minimal + reproducible. Uses only the guests' public APIs.

use proptest::prelude::*;

// ─────────────────────────────────── credit_v1 ───────────────────────────────────
mod credit {
    use super::*;
    use settle_credit_v1::{
        config_hash, derive_instrument_id, position_root, settle, snapshot_root, terms_hash,
        ConfigFields, Holder, Inputs, Payment, Position, SettleError, Terms,
    };

    fn build(balances: &[i128], rate: u32, paid: &[i128], covers: &[i128], collateral: i128) -> Inputs {
        let deadline = 500u64;
        let snapshot: Vec<Holder> = balances.iter().enumerate().map(|(i, &b)| {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
            Holder { id, balance: b, has_trustline: true, frozen: false }
        }).collect();
        let payments: Vec<Payment> = paid.iter().enumerate().filter(|(_, &p)| p > 0).map(|(i, &p)| {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
            Payment { holder: id, amount: p, paid_at: deadline, clawed_back: false }
        }).collect();
        let positions: Vec<Position> = covers.iter().enumerate()
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
        Inputs { type_id_xdr, rules_version: 1, config, instrument_id, epoch: 1, deadline, terms,
                 collateral, snapshot, payments, positions, position_root }
    }

    proptest! {
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
            match settle(&build(&balances, rate, &paid, &covers, collateral)) {
                Ok((allocs, j)) => {
                    let sum: i128 = allocs.iter().map(|a| a.amount).sum();
                    prop_assert_eq!(sum as u64, j.total_payout);
                    prop_assert!(sum <= collateral);
                    prop_assert!(sum <= covers.iter().sum());
                    prop_assert!(allocs.iter().all(|a| a.amount > 0));
                }
                Err(SettleError::NoDefault) | Err(SettleError::Insolvent) | Err(SettleError::Overflow) => {}
                Err(e) => prop_assert!(false, "unexpected error {:?}", e),
            }
        }

        #[test]
        fn fully_paid_is_always_nodefault(
            balances in prop::collection::vec(1i128..1_000_000i128, 1..6),
            rate in 1u32..=10_000u32,
            covers in prop::collection::vec(1i128..1_000i128, 1..3),
        ) {
            let paid: Vec<i128> = balances.iter().map(|&b| b.saturating_mul(rate as i128) / 10_000).collect();
            prop_assert_eq!(settle(&build(&balances, rate, &paid, &covers, 100_000_000)), Err(SettleError::NoDefault));
        }

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

        #[test]
        fn tampered_position_root_always_rejected(
            balances in prop::collection::vec(1i128..1_000_000i128, 1..5),
            rate in 1u32..=10_000u32,
            covers in prop::collection::vec(1i128..10_000i128, 1..3),
            tweak in any::<[u8; 32]>(),
        ) {
            let paid: Vec<i128> = balances.iter().map(|_| 0i128).collect();
            let mut inputs = build(&balances, rate, &paid, &covers, 100_000_000);
            prop_assume!(tweak != inputs.position_root);
            inputs.position_root = tweak;
            prop_assert_eq!(settle(&inputs), Err(SettleError::PositionMismatch));
        }
    }
}

// ─────────────────────────────────── weather_v1 ───────────────────────────────────
mod weather {
    use super::*;
    use settle_weather_v1::{
        config_hash, derive_instrument_id, position_root, settle, snapshot_root, terms_hash,
        ConfigFields, Inputs, Observation, Position, SettleError, Terms, WeatherParams,
    };

    fn build(readings: &[u32], trigger: u32, exhaust: u32, covers: &[i128], collateral: i128) -> Inputs {
        let params = WeatherParams { station_id: [9; 32], window_start: 100, window_end: 1_000_000 };
        let terms = Terms { trigger_mm: trigger, exhaust_mm: exhaust };
        let observations: Vec<Observation> = readings.iter().enumerate()
            .map(|(i, &mm)| Observation { station: [9; 32], mm, observed_at: 200 + i as u64 }).collect();
        let positions: Vec<Position> = covers.iter().enumerate()
            .map(|(i, &c)| Position { buyer: vec![(i as u8).wrapping_add(1); 40], cover: c, salt: [i as u8; 32] }).collect();
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
        Inputs { type_id_xdr, rules_version: 1, config, instrument_id, epoch: 1, deadline: 1_000_000,
                 terms, params, collateral, observations, positions, position_root }
    }

    proptest! {
        #[test]
        fn weather_payouts_bounded(
            readings in prop::collection::vec(0u32..10_000u32, 0..8),
            exhaust in 0u32..5_000u32,
            span in 1u32..5_000u32,
            covers in prop::collection::vec(1i128..1_000_000i128, 1..4),
            collateral in 1i128..100_000_000i128,
        ) {
            match settle(&build(&readings, exhaust + span, exhaust, &covers, collateral)) {
                Ok((allocs, j)) => {
                    let sum: i128 = allocs.iter().map(|a| a.amount).sum();
                    prop_assert_eq!(sum as u64, j.total_payout);
                    prop_assert!(sum <= collateral);
                    prop_assert!(sum <= covers.iter().sum());
                }
                Err(SettleError::NoBreach) | Err(SettleError::Insolvent) | Err(SettleError::Overflow) => {}
                Err(e) => prop_assert!(false, "unexpected error {:?}", e),
            }
        }

        #[test]
        fn weather_no_breach_when_rainfall_meets_trigger(
            readings in prop::collection::vec(0u32..10_000u32, 1..8),
        ) {
            let observed: u64 = readings.iter().map(|&r| r as u64).sum();
            prop_assume!(observed >= 1);
            prop_assert_eq!(settle(&build(&readings, observed as u32, 0, &[800], 100_000_000)), Err(SettleError::NoBreach));
        }
    }
}

// ─────────────────────────────────── credit_v2 (attested) ───────────────────────────────────
mod credit_v2 {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use settle_credit_v2::{
        config_hash, derive_instrument_id, payments_digest, position_root, settle, snapshot_root,
        terms_hash, ConfigFields, Holder, Inputs, Payment, Position, SettleError, Terms,
    };

    fn build(rate: u32, payments: Vec<Payment>, sign_over: &[Payment], signer: &SigningKey) -> Inputs {
        let snapshot = vec![
            Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false },
            Holder { id: [2; 32], balance: 10_000, has_trustline: true, frozen: false },
        ];
        let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
        let terms = Terms { coupon_rate_bps: rate, issuer_pubkey: signer.verifying_key().to_bytes() };
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
        Inputs { type_id_xdr, rules_version: 1, config, instrument_id, epoch: 1, deadline: 500, terms,
                 collateral: 2000, snapshot, payments, attestation,
                 positions: positions.clone(), position_root: position_root(&positions) }
    }

    proptest! {
        // Correctly-attested data reaches determination (never AttestationInvalid).
        #[test]
        fn attested_data_passes_the_gate(
            amounts in prop::collection::vec(0i128..2_000i128, 0..4),
        ) {
            let sk = SigningKey::from_bytes(&[42u8; 32]);
            let payments: Vec<Payment> = amounts.iter().enumerate()
                .map(|(i, &a)| Payment { holder: [(i % 2 + 1) as u8; 32], amount: a, paid_at: 400, clawed_back: false })
                .collect();
            match settle(&build(1000, payments.clone(), &payments, &sk)) {
                Ok(_) | Err(SettleError::NoDefault) | Err(SettleError::Insolvent) => {}
                Err(e) => prop_assert!(false, "attested data must not be rejected: {:?}", e),
            }
        }

        // ANY mismatch between the presented payments and the signed payments is rejected.
        #[test]
        fn unattested_payments_always_rejected(
            signed in prop::collection::vec(0i128..2_000i128, 1..4),
            extra in 1i128..2_000i128,
        ) {
            let sk = SigningKey::from_bytes(&[42u8; 32]);
            let signed_p: Vec<Payment> = signed.iter().enumerate()
                .map(|(i, &a)| Payment { holder: [(i % 2 + 1) as u8; 32], amount: a, paid_at: 400, clawed_back: false })
                .collect();
            let mut presented = signed_p.clone();
            presented.push(Payment { holder: [1; 32], amount: extra, paid_at: 401, clawed_back: false });
            prop_assert_eq!(settle(&build(1000, presented, &signed_p, &sk)), Err(SettleError::AttestationInvalid));
        }
    }
}
