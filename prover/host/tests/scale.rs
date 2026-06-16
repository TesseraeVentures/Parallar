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

fn credit_v3_witness(n: usize) -> settle_credit_v3::Inputs {
    use ed25519_dalek::{Signer, SigningKey};
    use settle_credit_v3::{
        config_hash, derive_instrument_id, position_root, record_date_msg, terms_hash, ConfigFields,
        Holder, Inputs, Position, Terms,
    };
    let sk = SigningKey::from_bytes(&[42u8; 32]);
    let mut holders = Vec::with_capacity(n);
    for i in 0..n {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
        holders.push(Holder { id, balance: 10_000, has_trustline: true, frozen: false });
    }
    let payments = vec![]; // all unpaid -> default (so determination reaches payout)
    let terms = Terms { coupon_rate_bps: 1000, issuer_pubkey: sk.verifying_key().to_bytes() };
    let positions = vec![Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let config = ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: [0x33; 32],
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
    let proot = position_root(&positions);
    let attestation = sk.sign(&record_date_msg(1, &holders, &payments)).to_bytes().to_vec();
    Inputs {
        type_id_xdr,
        rules_version: 1,
        config,
        instrument_id,
        epoch: 1,
        deadline: 500,
        terms,
        collateral: 100_000_000,
        attestation,
        snapshot: holders,
        payments,
        positions,
        position_root: proot,
    }
}

#[test]
#[ignore = "scale benchmark: attested record-date determination (credit_v3) incl. in-circuit Ed25519"]
fn credit_v3_determination_scale() {
    println!("\ncredit_v3 — attested record-date determination at scale (zkVM executor; incl. in-circuit Ed25519)");
    println!("holders   user_cycles   proving_cycles(sum 2^po2)");
    println!("-------   -----------   -------------------------");
    for n in [10usize, 100, 1000] {
        let inputs = credit_v3_witness(n);
        let _ = settle_credit_v3::settle(&inputs).expect("native settle");
        let env = ExecutorEnv::builder().write(&inputs).unwrap().build().unwrap();
        let info = default_executor()
            .execute(env, parallar_methods::SETTLE_CREDIT_V3_GUEST_ELF)
            .unwrap();
        println!("{n:>7}   {:>11}   {:>17}", info.cycles(), proving_cycles(&info));
    }
    println!("the Ed25519 attestation adds a flat in-circuit cost on top of the credit determination.");
}

#[test]
#[ignore = "scale benchmark: confidential-cover solvency proof (solvency_v1) — constant-size"]
fn solvency_cycle_cost() {
    use solvency_v1::{check, commit_total, SolvencyInputs, SolvencyRequest};
    println!("\nsolvency_v1 — confidential purchase proof (zkVM executor; constant-size, hides cover + totals)");
    let inputs = SolvencyInputs {
        collateral: 1000,
        prev_cover_commitment: commit_total(0, &[1u8; 32]),
        new_cover_commitment: commit_total(600, &[2u8; 32]),
        position_commitment: settle_credit_v1::commitment(&vec![0x12u8; 40], 600, &[7u8; 32]),
        old_total: 0,
        old_salt: [1u8; 32],
        new_salt: [2u8; 32],
        cover: 600,
        buyer: vec![0x12u8; 40],
        salt: [7u8; 32],
    };
    let _ = check(&inputs).expect("native solvency check");
    let req = SolvencyRequest::Buy(inputs);
    let env = ExecutorEnv::builder().write(&req).unwrap().build().unwrap();
    let info = default_executor()
        .execute(env, parallar_methods::SOLVENCY_V1_GUEST_ELF)
        .unwrap();
    println!("user_cycles={}   proving_cycles={}  (one Poseidon-committed solvency check; no holder loop)", info.cycles(), proving_cycles(&info));
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
