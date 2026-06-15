//! Scale benchmark (ignored): run the determination guests in the RISC Zero EXECUTOR over
//! growing inputs and report zkVM cycle counts. Cycles are DETERMINISTIC and hardware-
//! independent (unlike proving wall-clock), so these numbers are representative when measured on
//! ANY machine — the honest way to show why determination runs off-chain under a proof with a
//! constant-size verification on-chain: a 1,000-holder determination is far past what a Soroban
//! transaction (~100M CPU insns) could ever run inline, yet on-chain verification stays flat.
//!
//!   cargo test -p parallar-prover-host --test scale -- --ignored --nocapture

use risc0_zkvm::{default_executor, ExecutorEnv, SessionInfo};

/// Padded proving cycles: the prover commits to power-of-2 segment sizes, so this (Σ 2^po2) is
/// the work the SNARK actually proves — distinct from raw user cycles.
fn proving_cycles(info: &SessionInfo) -> u64 {
    info.segments.iter().map(|s| 1u64 << s.po2).sum()
}

fn credit_witness(n: usize) -> settle_credit_v1::Inputs {
    use settle_credit_v1::{
        config_hash, derive_instrument_id, position_root, snapshot_root, terms_hash, ConfigFields,
        Holder, Inputs, Position, Terms,
    };
    let mut holders = Vec::with_capacity(n);
    for i in 0..n {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
        holders.push(Holder { id, balance: 10_000, has_trustline: true, frozen: false });
    }
    let terms = Terms { coupon_rate_bps: 1000 };
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&holders),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
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
        deadline: 500,
        terms,
        collateral: 1000,
        snapshot: holders,
        payments: vec![],
        positions,
        position_root,
    }
}

fn weather_witness(n_obs: usize) -> settle_weather_v1::Inputs {
    use settle_weather_v1::{
        config_hash, derive_instrument_id, position_root, snapshot_root, terms_hash, ConfigFields,
        Inputs, Observation, Position, Terms, WeatherParams,
    };
    let params = WeatherParams { station_id: [9; 32], window_start: 100, window_end: 10_000_000 };
    let terms = Terms { trigger_mm: 2000, exhaust_mm: 100 }; // observed (≤ n_obs mm) stays < trigger
    let mut observations = Vec::with_capacity(n_obs);
    for i in 0..n_obs {
        observations.push(Observation { station: [9; 32], mm: 1, observed_at: 200 + i as u64 });
    }
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&params),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 150,
        epoch_deadlines: vec![(1u32, 500u64)],
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
        deadline: 500,
        terms,
        params,
        collateral: 1000,
        observations,
        positions,
        position_root,
    }
}

#[test]
#[ignore = "scale benchmark: zkVM executor cycle counts over 10/100/1000 holders"]
fn credit_determination_scale() {
    println!("\ncredit_v1 — determination at scale (zkVM executor; cycles are hardware-independent)");
    println!("holders   user_cycles   proving_cycles(sum 2^po2)");
    println!("-------   -----------   -------------------------");
    for n in [10usize, 100, 1000] {
        let inputs = credit_witness(n);
        let _ = settle_credit_v1::settle(&inputs).expect("native settle");
        let env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let info = default_executor()
            .execute(env, parallar_methods::SETTLE_CREDIT_V1_GUEST_ELF)
            .unwrap();
        println!("{n:>7}   {:>11}   {:>17}", info.cycles(), proving_cycles(&info));
    }
    println!("on-chain verify is FLAT at ~35M CPU insns regardless of holder count.");
}

#[test]
#[ignore = "scale benchmark: zkVM executor cycle counts over 10/100/1000 observations"]
fn weather_determination_scale() {
    println!("\nweather_v1 — determination at scale (zkVM executor; same generic core)");
    println!("observations   user_cycles   proving_cycles(sum 2^po2)");
    println!("------------   -----------   -------------------------");
    for n in [10usize, 100, 1000] {
        let inputs = weather_witness(n);
        let _ = settle_weather_v1::settle(&inputs).expect("native settle");
        let env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let info = default_executor()
            .execute(env, parallar_methods::SETTLE_WEATHER_V1_GUEST_ELF)
            .unwrap();
        println!("{n:>12}   {:>11}   {:>17}", info.cycles(), proving_cycles(&info));
    }
}
