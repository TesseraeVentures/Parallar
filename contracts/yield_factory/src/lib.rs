#![no_std]
//! YieldFactory — the tiered protected-share-class factory (TECH_SPEC §5A, PRODUCTION_GAP G11).
//!
//! A NEW factory version; the deployed credit factory stays frozen (Law #2). It does two things:
//!   1. Registers standardised RISK TIERS — each a `(min_premium_bps, max_premium_bps, haircut)`
//!      band + label. A tier is a risk profile underwriters understand and can express appetite
//!      for ("back investment-grade corporates", "back high-yield"), so similar-risk bonds are
//!      backed under one standard rather than bespoke terms per bond.
//!   2. Deploys a full protected family — `yield_vault` + settlement + `yield_router`, cross-bound
//!      in one transaction via the Soroban deployer pattern — for ANY bond/asset, with the
//!      instrument's premium RISK-PRICED within its tier's band (validated here), so a riskier
//!      bond carries a higher premium and therefore a lower net coupon.
//!
//! Settlement is unchanged: on a default the standard proof-gated certificate pays from the
//! reserve (Law #1). The router is purely upstream; the four core surfaces are untouched.

use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    String, Symbol, Val, Vec,
};

/// A standardised risk profile. Premium for any instrument in the tier must fall in the band;
/// the haircut is applied to that tier's reserves.
#[contracttype]
#[derive(Clone)]
pub struct RiskTier {
    pub min_premium_bps: u32,
    pub max_premium_bps: u32,
    pub haircut_bps: u32,
    pub label: String,
}

/// Per-bond deploy parameters. `premium_bps` is the risk-priced premium (within the tier band).
///
/// TRUST DELTA vs the base `ParallarFactory` (PRODUCTION_GAP G11): the base factory DERIVES
/// `instrument_id = H(type_id ‖ rules_version ‖ config_hash)`, binding the on-chain config to the
/// id the guest re-derives. This tiered factory accepts `instrument_id` as an admin-supplied
/// field. The guest's M1/M2 config re-derivation remains the soundness backstop — a mismatched
/// config still cannot produce a valid proof, so this is NOT a Law #1 break — but on-chain
/// id↔config canonicity is not guaranteed here. Deriving it (mirroring the base factory) is a
/// follow-up; until then the registry multisig is the gate.
#[contracttype]
#[derive(Clone)]
pub struct ProtectedConfig {
    pub instrument_id: BytesN<32>,
    pub image_id: BytesN<32>, // settlement guest image_id (e.g. credit_v1)
    pub bond_token: Address,
    pub coupon_token: Address, // the reserve / payout / premium / coupon asset
    pub premium_bps: u32,
    pub protocol_fee_bps: u32,
    pub dist_fee_bps: u32,
    pub epoch_deadlines: Vec<(u32, u64)>,
}

#[contracttype]
#[derive(Clone)]
pub struct ProtectedInstrument {
    pub tier: Symbol,
    pub vault: Address,
    pub settlement: Address,
    pub router: Address,
    pub premium_bps: u32,
}

/// Per-bond deploy parameters for a TRANCHED family. `weights[rank]` is tranche `rank`'s premium
/// share (rank 0 = junior/first-loss). The premium is still risk-priced within the tier band; the
/// tranches layer first-loss seniority *within* the instrument's reserve.
#[contracttype]
#[derive(Clone)]
pub struct TranchedConfig {
    pub instrument_id: BytesN<32>,
    pub image_id: BytesN<32>,
    pub collateral_token: Address, // reserve / payout / premium asset
    pub premium_bps: u32,
    pub protocol_fee_bps: u32,
    pub weights: Vec<u32>,
    pub epoch_deadlines: Vec<(u32, u64)>,
}

#[contracttype]
#[derive(Clone)]
pub struct TranchedInstrument {
    pub tier: Symbol,
    pub vault: Address,
    pub settlement: Address,
    pub premium_bps: u32,
    pub num_tranches: u32,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Verifier,
    VaultWasm,
    SettlementWasm,
    RouterWasm,
    TranchedWasm,
    Tier(Symbol),
    TierCount(Symbol),
    Instrument(BytesN<32>),
    TranchedInst(BytesN<32>),
}

#[contractevent(data_format = "map")]
pub struct ProtectedDeployed {
    #[topic]
    pub instrument_id: BytesN<32>,
    pub tier: Symbol,
    pub premium_bps: u32,
}

#[contractevent(data_format = "map")]
pub struct TranchedDeployed {
    #[topic]
    pub instrument_id: BytesN<32>,
    pub tier: Symbol,
    pub premium_bps: u32,
    pub num_tranches: u32,
}

// init clients for the freshly deployed family
#[contractclient(name = "VaultInitClient")]
pub trait VaultInit {
    fn init(env: Env, settlement: Address, collateral_token: Address, admin: Address, premium_bps: u32, protocol_fee_bps: u32, haircut_bps: u32);
    fn set_router(env: Env, router: Address);
}
#[contractclient(name = "SettlementInitClient")]
pub trait SettlementInit {
    fn init(env: Env, image_id: BytesN<32>, instrument_id: BytesN<32>, vault: Address, deadlines: Vec<(u32, u64)>, verifier_router: Address);
}
#[contractclient(name = "RouterInitClient")]
pub trait RouterInit {
    fn init(env: Env, bond_token: Address, coupon_token: Address, vault: Address, admin: Address, premium_bps: u32, dist_fee_bps: u32);
}
#[contractclient(name = "TranchedVaultInitClient")]
pub trait TranchedVaultInit {
    fn init(env: Env, settlement: Address, collateral_token: Address, admin: Address, premium_bps: u32, protocol_fee_bps: u32, weights: Vec<u32>);
}

#[contract]
pub struct YieldFactory;

fn salt(env: &Env, instrument_id: &BytesN<32>, tag: u8) -> BytesN<32> {
    let mut buf = soroban_sdk::Bytes::new(env);
    buf.append(&soroban_sdk::Bytes::from_array(env, &instrument_id.to_array()));
    buf.push_back(tag);
    env.crypto().sha256(&buf).to_bytes()
}

#[contractimpl]
impl YieldFactory {
    /// `verifier_router` is the network RISC Zero verifier; the three wasm hashes are the frozen
    /// family WASM (yield_vault / settlement / yield_router) this factory deploys.
    pub fn __constructor(
        env: Env,
        admin: Address,
        verifier_router: Address,
        vault_wasm: BytesN<32>,
        settlement_wasm: BytesN<32>,
        router_wasm: BytesN<32>,
    ) {
        let s = env.storage().instance();
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::Verifier, &verifier_router);
        s.set(&DataKey::VaultWasm, &vault_wasm);
        s.set(&DataKey::SettlementWasm, &settlement_wasm);
        s.set(&DataKey::RouterWasm, &router_wasm);
    }

    /// Register a standardised risk tier. Admin; a tier is immutable once registered (re-pricing
    /// a tier would silently re-rate live instruments — register a new tier instead).
    pub fn register_tier(env: Env, tier_id: Symbol, min_premium_bps: u32, max_premium_bps: u32, haircut_bps: u32, label: String) {
        Self::admin(env.clone()).require_auth();
        assert!(min_premium_bps <= max_premium_bps && max_premium_bps <= 10_000 && haircut_bps < 10_000, "bad tier band");
        let key = DataKey::Tier(tier_id.clone());
        assert!(!env.storage().persistent().has(&key), "tier already registered");
        env.storage().persistent().set(&key, &RiskTier { min_premium_bps, max_premium_bps, haircut_bps, label });
    }

    /// Deploy + cross-bind a protected family (vault + settlement + router) for a bond, into a
    /// tier, with a risk-priced premium validated against the tier band. One transaction. Admin.
    pub fn deploy_protected(env: Env, tier_id: Symbol, cfg: ProtectedConfig) -> (Address, Address, Address) {
        Self::admin(env.clone()).require_auth();
        let tier: RiskTier = env.storage().persistent().get(&DataKey::Tier(tier_id.clone())).expect("tier not registered");
        assert!(
            cfg.premium_bps >= tier.min_premium_bps && cfg.premium_bps <= tier.max_premium_bps,
            "premium outside the tier's risk band"
        );
        let s = env.storage().instance();
        let admin: Address = s.get(&DataKey::Admin).unwrap();
        let verifier: Address = s.get(&DataKey::Verifier).unwrap();
        let vault_wasm: BytesN<32> = s.get(&DataKey::VaultWasm).unwrap();
        let settlement_wasm: BytesN<32> = s.get(&DataKey::SettlementWasm).unwrap();
        let router_wasm: BytesN<32> = s.get(&DataKey::RouterWasm).unwrap();

        let here = env.current_contract_address();
        let no_args: Vec<Val> = Vec::new(&env);
        let vault = env.deployer().with_address(here.clone(), salt(&env, &cfg.instrument_id, 0)).deploy_v2(vault_wasm, no_args.clone());
        let settlement = env.deployer().with_address(here.clone(), salt(&env, &cfg.instrument_id, 1)).deploy_v2(settlement_wasm, no_args.clone());
        let router = env.deployer().with_address(here, salt(&env, &cfg.instrument_id, 2)).deploy_v2(router_wasm, no_args);

        // cross-bind, atomically in this tx: vault ↔ settlement, vault ↔ router, premium/haircut set.
        VaultInitClient::new(&env, &vault).init(&settlement, &cfg.coupon_token, &admin, &cfg.premium_bps, &cfg.protocol_fee_bps, &tier.haircut_bps);
        SettlementInitClient::new(&env, &settlement).init(&cfg.image_id, &cfg.instrument_id, &vault, &cfg.epoch_deadlines, &verifier);
        RouterInitClient::new(&env, &router).init(&cfg.bond_token, &cfg.coupon_token, &vault, &admin, &cfg.premium_bps, &cfg.dist_fee_bps);
        VaultInitClient::new(&env, &vault).set_router(&router);

        env.storage().persistent().set(
            &DataKey::Instrument(cfg.instrument_id.clone()),
            &ProtectedInstrument { tier: tier_id.clone(), vault: vault.clone(), settlement: settlement.clone(), router: router.clone(), premium_bps: cfg.premium_bps },
        );
        let count: u32 = env.storage().persistent().get(&DataKey::TierCount(tier_id.clone())).unwrap_or(0);
        env.storage().persistent().set(&DataKey::TierCount(tier_id.clone()), &(count + 1));
        ProtectedDeployed { instrument_id: cfg.instrument_id, tier: tier_id, premium_bps: cfg.premium_bps }.publish(&env);
        (vault, settlement, router)
    }

    /// Register the tranched-vault WASM this factory deploys (admin; one-time, not in the
    /// constructor so the protected-only constructor surface stays unchanged).
    pub fn set_tranched_wasm(env: Env, tranched_wasm: BytesN<32>) {
        Self::admin(env.clone()).require_auth();
        env.storage().instance().set(&DataKey::TranchedWasm, &tranched_wasm);
    }

    /// Deploy + cross-bind a TRANCHED family (tranched_vault + settlement) for a bond, into a tier,
    /// with a risk-priced premium and a tranche structure (`cfg.weights`, rank 0 = junior). One
    /// transaction. Admin. The same settlement contract binds transparently — it calls the
    /// identical `pay_allocations(epoch, allocations)`, which the tranched vault absorbs junior-first.
    pub fn deploy_tranched(env: Env, tier_id: Symbol, cfg: TranchedConfig) -> (Address, Address) {
        Self::admin(env.clone()).require_auth();
        let tier: RiskTier = env.storage().persistent().get(&DataKey::Tier(tier_id.clone())).expect("tier not registered");
        assert!(
            cfg.premium_bps >= tier.min_premium_bps && cfg.premium_bps <= tier.max_premium_bps,
            "premium outside the tier's risk band"
        );
        assert!(!cfg.weights.is_empty(), "tranched family needs at least one tranche");
        let s = env.storage().instance();
        let admin: Address = s.get(&DataKey::Admin).unwrap();
        let verifier: Address = s.get(&DataKey::Verifier).unwrap();
        let settlement_wasm: BytesN<32> = s.get(&DataKey::SettlementWasm).unwrap();
        let tranched_wasm: BytesN<32> = s.get(&DataKey::TranchedWasm).expect("tranched wasm not set");

        let here = env.current_contract_address();
        let no_args: Vec<Val> = Vec::new(&env);
        // tags 3/4 keep tranched salts distinct from the protected family's 0/1/2.
        let vault = env.deployer().with_address(here.clone(), salt(&env, &cfg.instrument_id, 3)).deploy_v2(tranched_wasm, no_args.clone());
        let settlement = env.deployer().with_address(here, salt(&env, &cfg.instrument_id, 4)).deploy_v2(settlement_wasm, no_args);

        let num_tranches = cfg.weights.len();
        TranchedVaultInitClient::new(&env, &vault).init(&settlement, &cfg.collateral_token, &admin, &cfg.premium_bps, &cfg.protocol_fee_bps, &cfg.weights);
        SettlementInitClient::new(&env, &settlement).init(&cfg.image_id, &cfg.instrument_id, &vault, &cfg.epoch_deadlines, &verifier);

        env.storage().persistent().set(
            &DataKey::TranchedInst(cfg.instrument_id.clone()),
            &TranchedInstrument { tier: tier_id.clone(), vault: vault.clone(), settlement: settlement.clone(), premium_bps: cfg.premium_bps, num_tranches },
        );
        let count: u32 = env.storage().persistent().get(&DataKey::TierCount(tier_id.clone())).unwrap_or(0);
        env.storage().persistent().set(&DataKey::TierCount(tier_id.clone()), &(count + 1));
        TranchedDeployed { instrument_id: cfg.instrument_id, tier: tier_id, premium_bps: cfg.premium_bps, num_tranches }.publish(&env);
        (vault, settlement)
    }

    pub fn get_tranched(env: Env, instrument_id: BytesN<32>) -> TranchedInstrument {
        env.storage().persistent().get(&DataKey::TranchedInst(instrument_id)).unwrap()
    }

    pub fn get_tier(env: Env, tier_id: Symbol) -> RiskTier {
        env.storage().persistent().get(&DataKey::Tier(tier_id)).unwrap()
    }
    pub fn get_protected(env: Env, instrument_id: BytesN<32>) -> ProtectedInstrument {
        env.storage().persistent().get(&DataKey::Instrument(instrument_id)).unwrap()
    }
    pub fn tier_count(env: Env, tier_id: Symbol) -> u32 {
        env.storage().persistent().get(&DataKey::TierCount(tier_id)).unwrap_or(0)
    }
    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }
}

mod test;
