#![no_std]
//! VaultContract — generic, identical WASM for every instrument type (TECH_SPEC §3.2).
//!
//! Holds PUBLIC seller collateral + PUBLIC aggregates. Buyer positions live ONLY as
//! Poseidon commitments `H(buyer ‖ cover ‖ salt)` (computed off-chain); the contract
//! stores the opaque 32-byte commitment and folds it into a `position_root`
//! accumulator (sha256 chain over the Poseidon leaves — the guest reproduces it).
//!
//! ARCHITECTURAL LAW: payouts move solely via `pay_allocations`, callable only by the
//! bound settlement contract — there is no admin/override path. No per-buyer cover is
//! ever stored or exposed (only the opaque commitment + the public aggregate).
//!
//! Solvency: MVP **Option B** — a running PUBLIC aggregate `total_cover`; an individual
//! cover is revealed transiently in the buy tx, never persisted per-buyer. Replaceable
//! by Option C (purchase-time proof) behind this same interface (decision June 17).

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, token, Address, Bytes, BytesN, Env, Vec};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Settlement,
    CollateralToken,
    TotalCollateral,
    TotalCover,
    PositionRoot,
    Frozen,
    Seller(Address),
}

/// Emitted when the bound settlement contract executes payouts (ledger history).
#[contractevent(data_format = "single-value")]
pub struct AllocationsPaid {
    #[topic]
    pub epoch: u32,
    pub total: i128,
}

#[contract]
pub struct VaultContract;

fn fold(env: &Env, prev: &BytesN<32>, commitment: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_array(env, &prev.to_array()));
    buf.append(&Bytes::from_array(env, &commitment.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}

fn is_frozen(env: &Env) -> bool {
    env.storage().instance().get(&DataKey::Frozen).unwrap_or(false)
}

#[contractimpl]
impl VaultContract {
    /// Cross-binding set at factory deploy: `settlement` is the only address that may
    /// move collateral out via `pay_allocations`.
    pub fn init(env: Env, settlement: Address, collateral_token: Address) {
        let s = env.storage().instance();
        if s.has(&DataKey::Settlement) {
            panic!("already initialized");
        }
        s.set(&DataKey::Settlement, &settlement);
        s.set(&DataKey::CollateralToken, &collateral_token);
        s.set(&DataKey::TotalCollateral, &0i128);
        s.set(&DataKey::TotalCover, &0i128);
        s.set(&DataKey::PositionRoot, &BytesN::from_array(&env, &[0u8; 32]));
        s.set(&DataKey::Frozen, &false);
        s.extend_ttl(50, 100);
    }

    /// Seller deposits collateral (public).
    pub fn deposit(env: Env, seller: Address, amount: i128) {
        seller.require_auth();
        assert!(amount > 0, "amount must be positive");
        let s = env.storage().instance();
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        token::Client::new(&env, &token_addr).transfer(&seller, &env.current_contract_address(), &amount);
        let total: i128 = s.get(&DataKey::TotalCollateral).unwrap();
        s.set(&DataKey::TotalCollateral, &(total + amount));
        let key = DataKey::Seller(seller);
        let bal: i128 = s.get(&key).unwrap_or(0);
        s.set(&key, &(bal + amount));
        s.extend_ttl(50, 100);
    }

    /// Buyer purchases protection: store the opaque commitment, fold it into
    /// `position_root`, and add `cover` to the public aggregate (Option B).
    pub fn buy_protection(env: Env, buyer: Address, commitment: BytesN<32>, cover: i128) {
        buyer.require_auth();
        assert!(cover > 0, "cover must be positive");
        assert!(!is_frozen(&env), "settlement window: vault frozen");
        let s = env.storage().instance();
        let total: i128 = s.get(&DataKey::TotalCollateral).unwrap();
        let cur_cover: i128 = s.get(&DataKey::TotalCover).unwrap();
        assert!(cur_cover + cover <= total, "insolvent: cover would exceed collateral");
        let root: BytesN<32> = s.get(&DataKey::PositionRoot).unwrap();
        s.set(&DataKey::PositionRoot, &fold(&env, &root, &commitment));
        s.set(&DataKey::TotalCover, &(cur_cover + cover));
        s.extend_ttl(50, 100);
    }

    /// Seller withdraws collateral. Frozen during settlement windows; cannot drop total
    /// collateral below outstanding cover.
    pub fn withdraw(env: Env, seller: Address, amount: i128) {
        seller.require_auth();
        assert!(amount > 0, "amount must be positive");
        assert!(!is_frozen(&env), "settlement window: withdrawals frozen");
        let s = env.storage().instance();
        let key = DataKey::Seller(seller.clone());
        let bal: i128 = s.get(&key).unwrap_or(0);
        assert!(bal >= amount, "insufficient seller balance");
        let total: i128 = s.get(&DataKey::TotalCollateral).unwrap();
        let cover: i128 = s.get(&DataKey::TotalCover).unwrap();
        assert!(total - amount >= cover, "would drop collateral below outstanding cover");
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        token::Client::new(&env, &token_addr).transfer(&env.current_contract_address(), &seller, &amount);
        s.set(&key, &(bal - amount));
        s.set(&DataKey::TotalCollateral, &(total - amount));
        s.extend_ttl(50, 100);
    }

    /// Execute payouts — the SOLE path collateral leaves to buyers. Only the bound
    /// settlement contract may authorize this (payouts only via verified settlement).
    pub fn pay_allocations(env: Env, epoch: u32, allocations: Vec<(Address, i128)>) {
        let s = env.storage().instance();
        let settlement: Address = s.get(&DataKey::Settlement).unwrap();
        settlement.require_auth();

        let mut sum: i128 = 0;
        for pair in allocations.iter() {
            let (_, amt) = pair;
            assert!(amt > 0, "allocation must be positive");
            sum += amt;
        }
        let total: i128 = s.get(&DataKey::TotalCollateral).unwrap();
        assert!(sum <= total, "allocations exceed collateral");

        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        let client = token::Client::new(&env, &token_addr);
        for pair in allocations.iter() {
            let (to, amt) = pair;
            client.transfer(&env.current_contract_address(), &to, &amt);
        }
        s.set(&DataKey::TotalCollateral, &(total - sum));
        s.extend_ttl(50, 100);
        AllocationsPaid { epoch, total: sum }.publish(&env);
    }

    /// Settlement opens/closes the withdrawal-freeze window.
    pub fn set_window(env: Env, open: bool) {
        let s = env.storage().instance();
        let settlement: Address = s.get(&DataKey::Settlement).unwrap();
        settlement.require_auth();
        s.set(&DataKey::Frozen, &open);
    }

    // --- getters (aggregates public; NO per-buyer cover exists to expose) ---
    pub fn settlement(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Settlement).unwrap()
    }
    pub fn collateral_token(env: Env) -> Address {
        env.storage().instance().get(&DataKey::CollateralToken).unwrap()
    }
    pub fn total_collateral(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalCollateral).unwrap()
    }
    pub fn total_cover(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalCover).unwrap()
    }
    pub fn position_root(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::PositionRoot).unwrap()
    }
    pub fn seller_balance(env: Env, seller: Address) -> i128 {
        env.storage().instance().get(&DataKey::Seller(seller)).unwrap_or(0)
    }
    pub fn is_frozen(env: Env) -> bool {
        is_frozen(&env)
    }
}

mod test;
