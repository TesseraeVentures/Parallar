//! One-off generator for a TESTNET-settleable weather_v1 (instance #2) scenario. Emits a
//! witness (for `parallar-prover prove --guest weather`) AND the matching on-chain values (the
//! InstrumentConfig for deploy_instrument, the position commitment for buy_protection, the payout
//! for settle) so the deployed instrument_id equals the proof's journal instrument_id and the
//! vault's position_root equals the journal's. Run with the deployed testnet addresses in env:
//!
//!   SCENARIO_XLM=C… SCENARIO_BUYER=G… SCENARIO_REFERENCE=G… \
//!     cargo test -p parallar-prover-host --test gen_weather_scenario -- --ignored --nocapture
//!
//! Mirrors gen_scenario.rs (credit) exactly in shape — the deploy/prove/settle handoff is the
//! same; only the determination (rainfall shortfall vs coupon shortfall) differs.

use parallar_prover_host::{address_xdr, symbol_xdr};
use settle_weather_v1::{
    commitment, config_hash, derive_instrument_id, position_root, settle, snapshot_root,
    terms_hash, ConfigFields, Inputs, Observation, Position, Terms, WeatherParams,
};
use soroban_sdk::{Address, Env, String as SString, Symbol};

#[test]
#[ignore = "one-off scenario generator; needs SCENARIO_* env vars"]
fn gen_testnet_weather_scenario() {
    let xlm = std::env::var("SCENARIO_XLM").expect("SCENARIO_XLM");
    let buyer_s = std::env::var("SCENARIO_BUYER").expect("SCENARIO_BUYER");
    let reference_s = std::env::var("SCENARIO_REFERENCE").expect("SCENARIO_REFERENCE");
    let out =
        std::env::var("SCENARIO_OUT").unwrap_or_else(|_| "/tmp/parallar_weather_scenario".into());
    std::fs::create_dir_all(&out).unwrap();

    let env = Env::default();
    let addr = |s: &str| Address::from_string(&SString::from_str(&env, s));
    let collateral_token = addr(&xlm);
    let buyer = addr(&buyer_s);
    let reference = addr(&reference_s);

    // A drought breach: window [t0, deadline]; trigger 500mm, exhaust 100mm (span 400).
    // Observed 300mm in-window -> severity (500-300)/400 = 0.5 -> buyer cover 800 pays 400.
    let deadline: u64 = 1_700_000_000; // past ledger timestamp (settle requires now > deadline)
    let window_start: u64 = 1_699_000_000;
    let station_id = [9u8; 32];
    let params = WeatherParams { station_id, window_start, window_end: deadline };
    let terms = Terms { trigger_mm: 500, exhaust_mm: 100 };
    let salt = [7u8; 32];
    let cover: i128 = 800;
    let collateral: i128 = 1000;
    let positions = vec![Position { buyer: address_xdr(&env, &buyer), cover, salt }];
    // three attested in-window readings summing to 300mm
    let observations = vec![
        Observation { station: station_id, mm: 100, observed_at: window_start + 10_000 },
        Observation { station: station_id, mm: 120, observed_at: window_start + 200_000 },
        Observation { station: station_id, mm: 80, observed_at: deadline - 5_000 },
    ];

    let config = ConfigFields {
        reference_asset_xdr: address_xdr(&env, &reference),
        terms_hash: terms_hash(&terms),
        schedule_root: [0x55; 32],
        snapshot_root: snapshot_root(&params),
        collateral_token_xdr: address_xdr(&env, &collateral_token),
        premium_bps: 150,
        epoch_deadlines: vec![(1, deadline)],
    };
    let type_id_xdr = symbol_xdr(&env, &Symbol::new(&env, "weather_v1"));
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
        params,
        collateral,
        observations,
        positions,
        position_root: proot,
    };

    // sanity: native settle succeeds and pays the buyer the pro-rata amount
    let (allocs, journal) = settle(&inputs).expect("native weather settle");
    assert_eq!(journal.instrument_id, instrument_id);
    assert_eq!(allocs.len(), 1);
    assert_eq!(journal.total_payout, 400, "0.5 severity on 800 cover");

    let commit = commitment(&address_xdr(&env, &buyer), cover, &salt);

    std::fs::write(format!("{out}/witness.json"), serde_json::to_string_pretty(&inputs).unwrap())
        .unwrap();
    let scenario = serde_json::json!({
        "type_id": "weather_v1",
        "image_id": "d31246e6d19379cfecbc23434e8c4aba0571e12cb6374b286ad3e9598db4a9bb",
        "instrument_id": hex::encode(instrument_id),
        "config": {
            "reference_asset": reference_s,
            "terms_hash": hex::encode(config.terms_hash),
            "schedule_root": hex::encode(config.schedule_root),
            "snapshot_root": hex::encode(config.snapshot_root),
            "collateral_token": xlm,
            "premium_bps": 150,
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
    std::fs::write(format!("{out}/scenario.json"), serde_json::to_string_pretty(&scenario).unwrap())
        .unwrap();
    println!(
        "weather scenario → {out} | instrument_id={} position_root={} payout={}",
        hex::encode(instrument_id),
        hex::encode(proot),
        journal.total_payout
    );
}
