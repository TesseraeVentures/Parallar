//! Witness generators for the post-R52 guest types (G1 attested, G4 record-date, G3 confidential).
//! Each emits a prove-ready witness JSON with DETERMINISTIC, synthetic-but-valid inputs, so a
//! founder can produce a real Groth16 proof on the x86 box with one command and no extra setup:
//!
//!   cargo test -p parallar-prover-host --test gen_new_scenarios -- --ignored --nocapture
//!   parallar-prover prove --guest credit-v3 --inputs /tmp/parallar_credit_v3/witness.json --out p.json
//!
//! For a TESTNET-SETTLEABLE scenario (real G-address buyers, the real issuer key), adapt these the
//! way `gen_scenario.rs` does (SCENARIO_* env addresses) — see docs/RUNBOOK.md. These are the
//! prove/benchmark inputs; the deploy side is in the deploy_* scripts.

fn out_dir(name: &str) -> String {
    let out = std::env::var("SCENARIO_OUT").unwrap_or_else(|_| format!("/tmp/parallar_{name}"));
    std::fs::create_dir_all(&out).unwrap();
    out
}

/// credit_v2 (G1, attested payments). Issuer demo key = [42;32]; production uses the real issuer key.
#[test]
#[ignore = "one-off witness generator"]
fn gen_credit_v2_witness() {
    use ed25519_dalek::{Signer, SigningKey};
    use settle_credit_v2 as v2;
    let out = out_dir("credit_v2");
    let sk = SigningKey::from_bytes(&[42u8; 32]);
    let snapshot = vec![
        v2::Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false },
        v2::Holder { id: [2; 32], balance: 10_000, has_trustline: true, frozen: false },
    ];
    let payments = vec![v2::Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }]; // h1 unpaid -> default
    let terms = v2::Terms { coupon_rate_bps: 1000, issuer_pubkey: sk.verifying_key().to_bytes() };
    let config = v2::ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: v2::terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: v2::snapshot_root(&snapshot),
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let positions = vec![v2::Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let inputs = v2::Inputs {
        instrument_id: v2::derive_instrument_id(&type_id_xdr, 1, &v2::config_hash(&config)),
        type_id_xdr,
        rules_version: 1,
        config,
        epoch: 1,
        deadline: 500,
        terms,
        collateral: 2000,
        attestation: sk.sign(&v2::payments_digest(&payments)).to_bytes().to_vec(),
        snapshot,
        payments,
        positions: positions.clone(),
        position_root: v2::position_root(&positions),
    };
    v2::settle(&inputs).expect("native credit_v2 settle must succeed");
    std::fs::write(format!("{out}/witness.json"), serde_json::to_string_pretty(&inputs).unwrap()).unwrap();
    println!("credit_v2 witness -> {out}/witness.json | prove: parallar-prover prove --guest credit-v2 --inputs {out}/witness.json --out proof.json");
}

/// credit_v3 (G4, record-date). The issuer signs (epoch ‖ snapshot ‖ payments) for THIS epoch.
#[test]
#[ignore = "one-off witness generator"]
fn gen_credit_v3_witness() {
    use ed25519_dalek::{Signer, SigningKey};
    use settle_credit_v3 as v3;
    let out = out_dir("credit_v3");
    let sk = SigningKey::from_bytes(&[42u8; 32]);
    let snapshot = vec![
        v3::Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false },
        v3::Holder { id: [2; 32], balance: 10_000, has_trustline: true, frozen: false },
    ];
    let payments = vec![v3::Payment { holder: [2; 32], amount: 1000, paid_at: 400, clawed_back: false }];
    let terms = v3::Terms { coupon_rate_bps: 1000, issuer_pubkey: sk.verifying_key().to_bytes() };
    let config = v3::ConfigFields {
        reference_asset_xdr: vec![0xAA, 1, 2, 3],
        terms_hash: v3::terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: [0x33; 32], // arbitrary — credit_v3 attests the per-epoch snapshot instead
        collateral_token_xdr: vec![0xBB, 4, 5, 6],
        premium_bps: 200,
        epoch_deadlines: vec![(1u32, 500u64)],
    };
    let type_id_xdr = vec![0xCCu8, 1, 2, 3, 4];
    let positions = vec![v3::Position { buyer: vec![0x12u8; 40], cover: 800, salt: [7; 32] }];
    let inputs = v3::Inputs {
        instrument_id: v3::derive_instrument_id(&type_id_xdr, 1, &v3::config_hash(&config)),
        type_id_xdr,
        rules_version: 1,
        config,
        epoch: 1,
        deadline: 500,
        terms,
        collateral: 2000,
        attestation: sk.sign(&v3::record_date_msg(1, &snapshot, &payments)).to_bytes().to_vec(),
        snapshot,
        payments,
        positions: positions.clone(),
        position_root: v3::position_root(&positions),
    };
    v3::settle(&inputs).expect("native credit_v3 settle must succeed");
    std::fs::write(format!("{out}/witness.json"), serde_json::to_string_pretty(&inputs).unwrap()).unwrap();
    println!("credit_v3 witness -> {out}/witness.json | prove: parallar-prover prove --guest credit-v3 --inputs {out}/witness.json --out proof.json");
}

/// solvency_v1 (G3, confidential cover): a BUY witness and a WITHDRAW witness, plus the vault's
/// initial cover_commitment (the confidential_vault init arg) so the on-chain `prev` matches.
#[test]
#[ignore = "one-off witness generator"]
fn gen_solvency_witnesses() {
    use solvency_v1 as sv;
    let out = out_dir("solvency");
    let old_salt = [1u8; 32];
    let new_salt = [2u8; 32];
    let salt = [7u8; 32];
    let buyer = vec![0x12u8; 40];

    // BUY: aggregate 0 -> 600 under collateral 1000, binding a cover-600 position commitment.
    let buy = sv::SolvencyInputs {
        collateral: 1000,
        prev_cover_commitment: sv::commit_total(0, &old_salt),
        new_cover_commitment: sv::commit_total(600, &new_salt),
        position_commitment: settle_credit_v1::commitment(&buyer, 600, &salt),
        old_total: 0,
        old_salt,
        new_salt,
        cover: 600,
        buyer,
        salt,
    };
    sv::check(&buy).expect("native solvency buy must succeed");
    std::fs::write(format!("{out}/buy_witness.json"), serde_json::to_string_pretty(&sv::SolvencyRequest::Buy(buy)).unwrap()).unwrap();

    // WITHDRAW: the committed aggregate (600) still fits under post-withdrawal collateral 700.
    let wsalt = [9u8; 32];
    let wd = sv::WithdrawInputs { collateral_after: 700, cover_commitment: sv::commit_total(600, &wsalt), total: 600, salt: wsalt };
    sv::check_withdraw(&wd).expect("native solvency withdraw must succeed");
    std::fs::write(format!("{out}/withdraw_witness.json"), serde_json::to_string_pretty(&sv::SolvencyRequest::Withdraw(wd)).unwrap()).unwrap();

    println!("solvency witnesses -> {out}/{{buy,withdraw}}_witness.json");
    println!("  confidential_vault init arg initial_cover_commitment = commit_total(0,[1;32]) = {}", hex::encode(sv::commit_total(0, &old_salt)));
    println!("  prove: parallar-prover prove-solvency --inputs {out}/buy_witness.json --out buy_proof.json");
}
