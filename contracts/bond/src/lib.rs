#![no_std]
//! BondContract — instance #1 reference issuance (mock, realistic mechanics).
//!
//! Models a live Stellar corporate issuance (unnamed): a real Stellar Asset
//! Contract (SAC) coupon token paid to N holders. Terms, payment schedule, and
//! the holder snapshot are stored only as Poseidon commitments. Coupons are
//! genuine on-ledger transfers and may be partial.
//!
//! ARCHITECTURAL LAW (CLAUDE.md §"two laws"): there is **no payments getter**.
//! Whether a coupon was paid/short/missed is determined from ledger history by
//! the settlement guest — never read back from this contract. Exposing a payouts
//! view here would make the ZK proof skippable, which is forbidden.

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, token, Address, BytesN, Env, Vec};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Issuer,
    CouponToken,
    TermsCommitment,
    ScheduleRoot,
    SnapshotRoot,
}

/// Aggregate coupon-payment event (count only — never per-holder). The settlement
/// guest determines payment from the SAC transfer history, not from this event.
#[contractevent(data_format = "single-value")]
pub struct CouponPaid {
    #[topic]
    pub epoch: u32,
    pub count: u32,
}

#[contract]
pub struct BondContract;

#[contractimpl]
impl BondContract {
    /// Initialize the issuance. `*_commitment`/`*_root` are Poseidon commitments to
    /// the off-chain terms, payment schedule, and holder snapshot (fixed at issuance).
    pub fn init(
        env: Env,
        issuer: Address,
        coupon_token: Address,
        terms_commitment: BytesN<32>,
        schedule_root: BytesN<32>,
        snapshot_root: BytesN<32>,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::Issuer) {
            panic!("already initialized");
        }
        s.set(&DataKey::Issuer, &issuer);
        s.set(&DataKey::CouponToken, &coupon_token);
        s.set(&DataKey::TermsCommitment, &terms_commitment);
        s.set(&DataKey::ScheduleRoot, &schedule_root);
        s.set(&DataKey::SnapshotRoot, &snapshot_root);
        s.extend_ttl(50, 100);
    }

    /// Pay (or partially pay) the coupon for `epoch`: real SAC transfers from the
    /// issuer to each `(holder, amount)`. The issuer chooses the list, so under-/
    /// non-payment is naturally representable (omit a holder, or send less).
    ///
    /// No per-holder payment record is persisted or exposed — determination is from
    /// ledger history. An aggregate event is emitted (that IS ledger history).
    pub fn pay_coupon(env: Env, epoch: u32, payments: Vec<(Address, i128)>) {
        let s = env.storage().instance();
        let issuer: Address = s.get(&DataKey::Issuer).unwrap();
        issuer.require_auth();
        let coupon_token: Address = s.get(&DataKey::CouponToken).unwrap();
        let client = token::Client::new(&env, &coupon_token);

        for pair in payments.iter() {
            let (holder, amount) = pair;
            client.transfer(&issuer, &holder, &amount);
        }
        s.extend_ttl(50, 100);
        CouponPaid { epoch, count: payments.len() }.publish(&env);
    }

    // --- public config getters (commitments + issuance config are public by design) ---
    // NOTE: deliberately NO getter exposing who was paid for an epoch.
    pub fn issuer(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Issuer).unwrap()
    }
    pub fn coupon_token(env: Env) -> Address {
        env.storage().instance().get(&DataKey::CouponToken).unwrap()
    }
    pub fn terms_commitment(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::TermsCommitment).unwrap()
    }
    pub fn schedule_root(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::ScheduleRoot).unwrap()
    }
    pub fn snapshot_root(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::SnapshotRoot).unwrap()
    }
}

mod test;
