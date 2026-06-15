//! One-off generator for a TESTNET-settleable scenario. Emits a witness (for `prove`) AND the
//! matching on-chain values (the InstrumentConfig for deploy_instrument, the position commitment
//! for buy_protection, the payout for settle) so the deployed instrument_id equals the proof's
//! journal instrument_id and the vault's position_root equals the journal's. Run with the
//! deployed testnet addresses in env:
//!
//!   SCENARIO_XLM=C… SCENARIO_BUYER=G… SCENARIO_REFERENCE=G… \
//!     cargo test -p parallar-prover-host --test gen_scenario -- --ignored --nocapture

use parallar_prover_host::{address_xdr, symbol_xdr};
use settle_credit_v1::{
    commitment, config_hash, derive_instrument_id, position_root, settle, snapshot_root,
    terms_hash, ConfigFields, Holder, Inputs, Position, Terms,
};
use soroban_sdk::{Address, Env, String as SString, Symbol};

#[test]
#[ignore = "one-off scenario generator; needs SCENARIO_* env vars"]
fn gen_testnet_scenario() {
    let xlm = std::env::var("SCENARIO_XLM").expect("SCENARIO_XLM");
    let buyer_s = std::env::var("SCENARIO_BUYER").expect("SCENARIO_BUYER");
    let reference_s = std::env::var("SCENARIO_REFERENCE").expect("SCENARIO_REFERENCE");
    let out = std::env::var("SCENARIO_OUT").unwrap_or_else(|_| "/tmp/parallar_scenario".into());
    std::fs::create_dir_all(&out).unwrap();

    let env = Env::default();
    let addr = |s: &str| Address::from_string(&SString::from_str(&env, s));
    let collateral_token = addr(&xlm);
    let buyer = addr(&buyer_s);
    let reference = addr(&reference_s);

    // 1 holder owed 1000 (balance 10000 @ 10%), unpaid -> full default; buyer cover 800.
    let holders = vec![Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false }];
    let terms = Terms { coupon_rate_bps: 1000 };
    let salt = [7u8; 32];
    let cover: i128 = 800;
    let collateral: i128 = 1000;
    let deadline: u64 = 1_700_000_000; // a past ledger timestamp (settle requires now > deadline)
    let positions = vec![Position { buyer: address_xdr(&env, &buyer), cover, salt }];

    let config = ConfigFields {
        reference_asset_xdr: address_xdr(&env, &reference),
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&holders),
        collateral_token_xdr: address_xdr(&env, &collateral_token),
        premium_bps: 200,
        epoch_deadlines: vec![(1, deadline)],
    };
    let type_id_xdr = symbol_xdr(&env, &Symbol::new(&env, "credit_v1"));
    let ch = config_hash(&config);
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &ch);
    let proot = position_root(&positions);

    let inputs = Inputs {
        type_id_xdr,
        rules_version: 1,
        config: config.clone(),
        instrument_id,
        epoch: 1,
        deadline,
        terms,
        collateral,
        snapshot: holders,
        payments: vec![],
        positions,
        position_root: proot,
    };

    // sanity: native settle succeeds and pays the buyer
    let (allocs, journal) = settle(&inputs).expect("native settle");
    assert_eq!(journal.instrument_id, instrument_id);
    assert_eq!(allocs.len(), 1);
    assert_eq!(journal.total_payout, cover as u64);

    let commit = commitment(&address_xdr(&env, &buyer), cover, &salt);

    std::fs::write(
        format!("{out}/witness.json"),
        serde_json::to_string_pretty(&inputs).unwrap(),
    )
    .unwrap();
    let scenario = serde_json::json!({
        "instrument_id": hex::encode(instrument_id),
        "config": {
            "reference_asset": reference_s,
            "terms_hash": hex::encode(config.terms_hash),
            "schedule_root": hex::encode(config.schedule_root),
            "snapshot_root": hex::encode(config.snapshot_root),
            "collateral_token": xlm,
            "premium_bps": 200,
            "epoch_deadlines": [[1, deadline]],
        },
        "commitment": hex::encode(commit),
        "buyer": buyer_s,
        "cover": cover,
        "collateral": collateral,
        "payout": journal.total_payout,
        "deadline": deadline,
        "position_root": hex::encode(proot),
    });
    std::fs::write(
        format!("{out}/scenario.json"),
        serde_json::to_string_pretty(&scenario).unwrap(),
    )
    .unwrap();
    println!(
        "scenario → {out} | instrument_id={} position_root={} payout={}",
        hex::encode(instrument_id),
        hex::encode(proot),
        journal.total_payout
    );
}
