//! One-off generator for a TESTNET-settleable PARTIAL-default scenario with MULTIPLE holders and
//! MULTIPLE cover buyers — the richer settlement that shows the pro-rata formula doing real work
//! (the canonical live settlement #3 is a clean full default with one buyer). Emits a witness
//! (for `parallar-prover prove`) AND the matching on-chain values per buyer. Run with deployed
//! testnet addresses in env:
//!
//!   SCENARIO_XLM=C… SCENARIO_BUYER=G… SCENARIO_BUYER2=G… SCENARIO_REFERENCE=G… \
//!     cargo test -p parallar-prover-host --test gen_partial_scenario -- --ignored --nocapture
//!
//! Scenario: 3 holders each owed 1000 (balance 10000 @ 10%). Holder 1 unpaid, holder 2 half-paid
//! (500), holder 3 paid in full -> Σ owed 3000, Σ shortfall 1500, severity 0.5. Two buyers cover
//! 600 and 400 -> pro-rata payouts 300 and 200 (Σ 500) against 1000 collateral.

use parallar_prover_host::{address_xdr, symbol_xdr};
use settle_credit_v1::{
    commitment, config_hash, derive_instrument_id, position_root, settle, snapshot_root,
    terms_hash, ConfigFields, Holder, Inputs, Payment, Position, Terms,
};
use soroban_sdk::{Address, Env, String as SString, Symbol};

#[test]
#[ignore = "one-off scenario generator; needs SCENARIO_* env vars"]
fn gen_testnet_partial_scenario() {
    let xlm = std::env::var("SCENARIO_XLM").expect("SCENARIO_XLM");
    let buyer_s = std::env::var("SCENARIO_BUYER").expect("SCENARIO_BUYER");
    let buyer2_s = std::env::var("SCENARIO_BUYER2").expect("SCENARIO_BUYER2");
    let reference_s = std::env::var("SCENARIO_REFERENCE").expect("SCENARIO_REFERENCE");
    let out =
        std::env::var("SCENARIO_OUT").unwrap_or_else(|_| "/tmp/parallar_partial_scenario".into());
    std::fs::create_dir_all(&out).unwrap();

    let env = Env::default();
    let addr = |s: &str| Address::from_string(&SString::from_str(&env, s));
    let collateral_token = addr(&xlm);
    let buyer = addr(&buyer_s);
    let buyer2 = addr(&buyer2_s);
    let reference = addr(&reference_s);

    let deadline: u64 = 1_700_000_000; // past ledger timestamp (settle requires now > deadline)
    let terms = Terms { coupon_rate_bps: 1000 };
    // 3 holders, partial default
    let holders = vec![
        Holder { id: [1; 32], balance: 10_000, has_trustline: true, frozen: false },
        Holder { id: [2; 32], balance: 10_000, has_trustline: true, frozen: false },
        Holder { id: [3; 32], balance: 10_000, has_trustline: true, frozen: false },
    ];
    // holder 1 unpaid; holder 2 half (500); holder 3 full (1000)
    let payments = vec![
        Payment { holder: [2; 32], amount: 500, paid_at: deadline - 100, clawed_back: false },
        Payment { holder: [3; 32], amount: 1000, paid_at: deadline - 100, clawed_back: false },
    ];
    // two buyers, covers 600 + 400
    let salt1 = [7u8; 32];
    let salt2 = [9u8; 32];
    let positions = vec![
        Position { buyer: address_xdr(&env, &buyer), cover: 600, salt: salt1 },
        Position { buyer: address_xdr(&env, &buyer2), cover: 400, salt: salt2 },
    ];
    let collateral: i128 = 1000;

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
    let instrument_id = derive_instrument_id(&type_id_xdr, 1, &config_hash(&config));
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
        payments,
        positions,
        position_root: proot,
    };

    // sanity: native settle pays both buyers pro-rata at severity 0.5
    let (allocs, journal) = settle(&inputs).expect("native settle");
    assert_eq!(allocs.len(), 2, "both buyers paid");
    assert_eq!(journal.total_payout, 500, "300 + 200 at 0.5 severity");

    let commit1 = commitment(&address_xdr(&env, &buyer), 600, &salt1);
    let commit2 = commitment(&address_xdr(&env, &buyer2), 400, &salt2);

    std::fs::write(format!("{out}/witness.json"), serde_json::to_string_pretty(&inputs).unwrap())
        .unwrap();
    let scenario = serde_json::json!({
        "type_id": "credit_v1",
        "scenario": "3 holders, partial default (1500/3000 shortfall, severity 0.5); 2 buyers 600+400 -> 300+200",
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
        "buyers": [
            { "buyer": buyer_s, "cover": 600, "commitment": hex::encode(commit1), "payout": 300 },
            { "buyer": buyer2_s, "cover": 400, "commitment": hex::encode(commit2), "payout": 200 },
        ],
        "collateral": collateral,
        "total_payout": journal.total_payout,
        "deadline": deadline,
        "position_root": hex::encode(proot),
    });
    std::fs::write(format!("{out}/scenario.json"), serde_json::to_string_pretty(&scenario).unwrap())
        .unwrap();
    println!(
        "partial scenario → {out} | instrument_id={} payouts=300+200 severity=0.5",
        hex::encode(instrument_id)
    );
}
