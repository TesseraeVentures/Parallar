#![no_std]
//! ParallarFactory / Registry (R7, TECH_SPEC §3.0).
//!
//! Registers instrument *types* (a guest `image_id` + version + the generic vault/
//! settlement WASM hashes) and deploys instrument *instances* — a vault + settlement
//! pair, deployed and cross-bound in ONE transaction via the Soroban deployer pattern.
//!
//! Versioning rule (final): a new guest version is a NEW type entry; a registered type
//! is immutable, and a deployed instance is pinned to its image_id forever. The asset-
//! policy gate refuses collateral that isn't claw/freeze-proof (§3.0 step 5, §10.1).

use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, xdr::ToXdr, Address,
    Bytes, BytesN, Env, String, Symbol, Val, Vec,
};

#[contracttype]
#[derive(Clone)]
pub struct InstrumentType {
    pub image_id: BytesN<32>,
    pub rules_version: u32,
    pub rules_uri: String,
    pub vault_wasm: BytesN<32>,
    pub settlement_wasm: BytesN<32>,
}

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

#[contracttype]
#[derive(Clone)]
pub struct Instrument {
    pub type_id: Symbol,
    pub vault: Address,
    pub settlement: Address,
    pub config_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Verifier,
    Type(Symbol),
    Instrument(BytesN<32>),
    Eligible(Address),
}

#[contractevent(data_format = "single-value")]
pub struct InstrumentDeployed {
    #[topic]
    pub instrument_id: BytesN<32>,
    pub type_id: Symbol,
}

// Cross-init clients for the freshly deployed generic pair.
#[contractclient(name = "VaultInitClient")]
pub trait VaultInit {
    fn init(env: Env, settlement: Address, collateral_token: Address);
}
#[contractclient(name = "SettlementInitClient")]
pub trait SettlementInit {
    fn init(env: Env, image_id: BytesN<32>, instrument_id: BytesN<32>, vault: Address, deadlines: Vec<(u32, u64)>, verifier_router: Address);
}

#[contract]
pub struct ParallarFactory;

/// config_hash = sha256 over a FLAT canonical encoding of the config fields, so the
/// off-chain settlement guest can reproduce it byte-for-byte (the struct-XDR form was not
/// guest-reproducible). The two `Address` fields use their canonical XDR — the only
/// irreducible piece — which the guest receives as host-provided bytes, bound through
/// `instrument_id`. Layout: reference_asset_xdr ‖ terms_hash(32) ‖ schedule_root(32) ‖
/// snapshot_root(32) ‖ collateral_token_xdr ‖ premium_bps(4 BE) ‖ deadlines_len(4 BE) ‖
/// {epoch(4 BE) ‖ deadline(8 BE)}*.
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

/// instrument_id = H(type_id ‖ rules_version ‖ config_hash) — what guests bind proofs to.
pub fn derive_instrument_id(env: &Env, type_id: &Symbol, rules_version: u32, config_hash: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&type_id.clone().to_xdr(env));
    buf.append(&Bytes::from_array(env, &rules_version.to_be_bytes()));
    buf.append(&Bytes::from_array(env, &config_hash.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}

fn salt(env: &Env, instrument_id: &BytesN<32>, tag: u8) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_array(env, &instrument_id.to_array()));
    buf.push_back(tag);
    env.crypto().sha256(&buf).to_bytes()
}

#[contractimpl]
impl ParallarFactory {
    /// `verifier_router` is the network's RISC Zero verifier router (the Nethermind stack).
    /// It is system-level infrastructure shared by every instrument this factory deploys —
    /// not a per-type field (the registry `InstrumentType` surface stays frozen, Law #2) —
    /// and is plumbed into each settlement instance at deploy time.
    pub fn __constructor(env: Env, admin: Address, verifier_router: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Verifier, &verifier_router);
    }

    /// Register an instrument type. Admin-gated; a registered type is immutable (a new
    /// guest version is a new type_id, never an in-place edit).
    pub fn register_type(env: Env, type_id: Symbol, t: InstrumentType) {
        Self::admin(env.clone()).require_auth();
        let key = DataKey::Type(type_id);
        if env.storage().persistent().has(&key) {
            panic!("type already registered");
        }
        env.storage().persistent().set(&key, &t);
    }

    /// Admin curates the eligible collateral list — the MVP form of the §10.1 clawback/
    /// freeze gate (production reads `AUTH_CLAWBACK_ENABLED` on-chain).
    pub fn set_collateral_eligible(env: Env, token: Address, eligible: bool) {
        Self::admin(env.clone()).require_auth();
        env.storage().persistent().set(&DataKey::Eligible(token), &eligible);
    }

    /// Deploy + cross-bind a vault/settlement pair for a registered type, in one tx.
    pub fn deploy_instrument(env: Env, type_id: Symbol, config: InstrumentConfig) -> (BytesN<32>, Address, Address) {
        let t: InstrumentType = env
            .storage()
            .persistent()
            .get(&DataKey::Type(type_id.clone()))
            .expect("type not registered");

        // asset-policy gate (§3.0 step 5 / §10.1): collateral must be claw/freeze-proof
        let eligible: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Eligible(config.collateral_token.clone()))
            .unwrap_or(false);
        assert!(eligible, "collateral asset not eligible (clawback/freeze gate)");

        let config_hash = hash_config(&env, &config);
        let instrument_id = derive_instrument_id(&env, &type_id, t.rules_version, &config_hash);

        // deploy the generic pair (no constructor args), then cross-init — atomic in this tx
        let here = env.current_contract_address();
        let no_args: Vec<Val> = Vec::new(&env);
        let vault_addr = env
            .deployer()
            .with_address(here.clone(), salt(&env, &instrument_id, 0))
            .deploy_v2(t.vault_wasm.clone(), no_args.clone());
        let settlement_addr = env
            .deployer()
            .with_address(here, salt(&env, &instrument_id, 1))
            .deploy_v2(t.settlement_wasm.clone(), no_args);

        let verifier: Address = env.storage().instance().get(&DataKey::Verifier).unwrap();
        VaultInitClient::new(&env, &vault_addr).init(&settlement_addr, &config.collateral_token);
        SettlementInitClient::new(&env, &settlement_addr).init(
            &t.image_id,
            &instrument_id,
            &vault_addr,
            &config.epoch_deadlines,
            &verifier,
        );

        let instrument = Instrument {
            type_id: type_id.clone(),
            vault: vault_addr.clone(),
            settlement: settlement_addr.clone(),
            config_hash,
        };
        env.storage().persistent().set(&DataKey::Instrument(instrument_id.clone()), &instrument);
        InstrumentDeployed { instrument_id: instrument_id.clone(), type_id }.publish(&env);

        (instrument_id, vault_addr, settlement_addr)
    }

    pub fn get_type(env: Env, type_id: Symbol) -> InstrumentType {
        env.storage().persistent().get(&DataKey::Type(type_id)).unwrap()
    }
    pub fn get_instrument(env: Env, instrument_id: BytesN<32>) -> Instrument {
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
