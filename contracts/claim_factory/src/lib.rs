#![no_std]
//! ClaimFactory (PRODUCTION_GAP G2) — deploys the CLAIMABLE instrument family.
//!
//! The base `ParallarFactory` deploys a vault + the standard settlement. This deploys a vault + the
//! `ClaimableSettlement` variant (permissionless `settle` PLUS a buyer's own `claim_direct` after a
//! keeper-grace window — the escape hatch, so a single buyer can self-settle their allocation if the
//! keeper stalls). Both paths remain proof-gated (Law #1); there is no keeper trust beyond liveness.
//!
//! A NEW factory version; the base factory and its frozen `InstrumentType` surface are untouched
//! (Law #2). It reuses the base factory's `hash_config` / `derive_instrument_id` so a claimable
//! instrument's `instrument_id` and config binding are byte-identical to a standard one — the same
//! guest proofs verify against it.

use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, xdr::ToXdr, Address,
    Bytes, BytesN, Env, Symbol, Val, Vec,
};

// The instrument config + the id derivation are byte-identical to the base ParallarFactory's, so a
// claimable instrument binds proofs exactly as a standard one (the guest re-derives the same
// instrument_id). Reimplemented here rather than depending on parallar-factory's rlib — linking that
// cdylib's contract exports into this one collides on shared export symbols (e.g. `verifier`).
#[contracttype]
#[derive(Clone)]
pub struct InstrumentConfig {
    pub reference_asset: Address,
    pub terms_hash: BytesN<32>,
    pub schedule_root: BytesN<32>,
    pub snapshot_root: BytesN<32>,
    pub collateral_token: Address,
    pub premium_bps: u32,
    pub epoch_deadlines: Vec<(u32, u64)>,
}

/// config_hash = sha256 over the SAME flat canonical encoding the base factory + the guest use.
pub fn hash_config(env: &Env, c: &InstrumentConfig) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&c.reference_asset.clone().to_xdr(env));
    buf.append(&Bytes::from_array(env, &c.terms_hash.to_array()));
    buf.append(&Bytes::from_array(env, &c.schedule_root.to_array()));
    buf.append(&Bytes::from_array(env, &c.snapshot_root.to_array()));
    buf.append(&c.collateral_token.clone().to_xdr(env));
    buf.append(&Bytes::from_array(env, &c.premium_bps.to_be_bytes()));
    buf.append(&Bytes::from_array(env, &c.epoch_deadlines.len().to_be_bytes()));
    for pair in c.epoch_deadlines.iter() {
        let (epoch, deadline) = pair;
        buf.append(&Bytes::from_array(env, &epoch.to_be_bytes()));
        buf.append(&Bytes::from_array(env, &deadline.to_be_bytes()));
    }
    env.crypto().sha256(&buf).to_bytes()
}

/// instrument_id = H(type_id ‖ rules_version ‖ config_hash) — identical to the base factory.
pub fn derive_instrument_id(env: &Env, type_id: &Symbol, rules_version: u32, config_hash: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&type_id.clone().to_xdr(env));
    buf.append(&Bytes::from_array(env, &rules_version.to_be_bytes()));
    buf.append(&Bytes::from_array(env, &config_hash.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}

/// A claimable instrument type: BOTH guest image_ids (the settle guest and the single-claim guest)
/// plus the keeper-grace window after which a buyer may self-claim.
#[contracttype]
#[derive(Clone)]
pub struct ClaimableType {
    pub settle_image_id: BytesN<32>,
    pub claim_image_id: BytesN<32>,
    pub rules_version: u32,
    pub grace: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct ClaimableInstrument {
    pub type_id: Symbol,
    pub vault: Address,
    pub settlement: Address, // the ClaimableSettlement address
    pub config_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Verifier,
    VaultWasm,
    ClaimSettlementWasm,
    Type(Symbol),
    Instrument(BytesN<32>),
    Eligible(Address),
}

#[contractevent(data_format = "single-value")]
pub struct ClaimableDeployed {
    #[topic]
    pub instrument_id: BytesN<32>,
    pub type_id: Symbol,
}

#[contractclient(name = "VaultInitClient")]
pub trait VaultInit {
    fn init(env: Env, settlement: Address, collateral_token: Address);
}
#[contractclient(name = "ClaimableInitClient")]
pub trait ClaimableInit {
    fn init(
        env: Env,
        settle_image_id: BytesN<32>,
        claim_image_id: BytesN<32>,
        instrument_id: BytesN<32>,
        vault: Address,
        deadlines: Vec<(u32, u64)>,
        verifier_router: Address,
        grace: u64,
    );
}

#[contract]
pub struct ClaimFactory;

fn salt(env: &Env, instrument_id: &BytesN<32>, tag: u8) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_array(env, &instrument_id.to_array()));
    buf.push_back(tag);
    env.crypto().sha256(&buf).to_bytes()
}

#[contractimpl]
impl ClaimFactory {
    pub fn __constructor(
        env: Env,
        admin: Address,
        verifier_router: Address,
        vault_wasm: BytesN<32>,
        claim_settlement_wasm: BytesN<32>,
    ) {
        let s = env.storage().instance();
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::Verifier, &verifier_router);
        s.set(&DataKey::VaultWasm, &vault_wasm);
        s.set(&DataKey::ClaimSettlementWasm, &claim_settlement_wasm);
    }

    /// Register a claimable type (immutable once registered, like the base registry).
    pub fn register_claimable_type(env: Env, type_id: Symbol, t: ClaimableType) {
        Self::admin(env.clone()).require_auth();
        let key = DataKey::Type(type_id);
        assert!(!env.storage().persistent().has(&key), "type already registered");
        env.storage().persistent().set(&key, &t);
    }

    pub fn set_collateral_eligible(env: Env, token: Address, eligible: bool) {
        Self::admin(env.clone()).require_auth();
        env.storage().persistent().set(&DataKey::Eligible(token), &eligible);
    }

    /// Deploy + cross-bind a vault + ClaimableSettlement for a registered claimable type, in one tx.
    pub fn deploy_claimable(env: Env, type_id: Symbol, config: InstrumentConfig) -> (BytesN<32>, Address, Address) {
        Self::admin(env.clone()).require_auth();
        let t: ClaimableType = env
            .storage()
            .persistent()
            .get(&DataKey::Type(type_id.clone()))
            .expect("type not registered");

        let eligible: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Eligible(config.collateral_token.clone()))
            .unwrap_or(false);
        assert!(eligible, "collateral asset not eligible (clawback/freeze gate)");

        let config_hash = hash_config(&env, &config);
        let instrument_id = derive_instrument_id(&env, &type_id, t.rules_version, &config_hash);

        let s = env.storage().instance();
        let verifier: Address = s.get(&DataKey::Verifier).unwrap();
        let vault_wasm: BytesN<32> = s.get(&DataKey::VaultWasm).unwrap();
        let claim_wasm: BytesN<32> = s.get(&DataKey::ClaimSettlementWasm).unwrap();

        let here = env.current_contract_address();
        let no_args: Vec<Val> = Vec::new(&env);
        // tags 0 (vault) / 2 (claim settlement) — distinct from the base factory's 0/1 standard pair.
        let vault_addr = env.deployer().with_address(here.clone(), salt(&env, &instrument_id, 0)).deploy_v2(vault_wasm, no_args.clone());
        let settlement_addr = env.deployer().with_address(here, salt(&env, &instrument_id, 2)).deploy_v2(claim_wasm, no_args);

        VaultInitClient::new(&env, &vault_addr).init(&settlement_addr, &config.collateral_token);
        ClaimableInitClient::new(&env, &settlement_addr).init(
            &t.settle_image_id,
            &t.claim_image_id,
            &instrument_id,
            &vault_addr,
            &config.epoch_deadlines,
            &verifier,
            &t.grace,
        );

        env.storage().persistent().set(
            &DataKey::Instrument(instrument_id.clone()),
            &ClaimableInstrument { type_id: type_id.clone(), vault: vault_addr.clone(), settlement: settlement_addr.clone(), config_hash },
        );
        ClaimableDeployed { instrument_id: instrument_id.clone(), type_id }.publish(&env);
        (instrument_id, vault_addr, settlement_addr)
    }

    pub fn get_claimable_type(env: Env, type_id: Symbol) -> ClaimableType {
        env.storage().persistent().get(&DataKey::Type(type_id)).unwrap()
    }
    pub fn get_instrument(env: Env, instrument_id: BytesN<32>) -> ClaimableInstrument {
        env.storage().persistent().get(&DataKey::Instrument(instrument_id)).unwrap()
    }
    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }
    pub fn verifier(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Verifier).unwrap()
    }
}

mod test;
