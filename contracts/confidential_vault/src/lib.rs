#![no_std]
//! ConfidentialVault — Option C confidential cover (PRODUCTION_GAP G3), the on-chain consumer of
//! the `solvency_v1` guest.
//!
//! The vault keeps the running AGGREGATE cover as a Poseidon COMMITMENT (`cover_commitment`), never
//! a public total. Each purchase carries a `solvency_v1` proof that the new aggregate <= collateral
//! (the cover and the running totals stay hidden) and that the same hidden cover is the one bound in
//! the buyer's position commitment; each withdrawal carries a proof that the aggregate still fits
//! under the post-withdrawal collateral. The vault verifies ONE Groth16 proof against the pinned
//! solvency image_id, checks the proof's `prev` commitment equals what it stored, and advances to
//! `new`. Per-purchase cover and the running book never appear in plaintext on-chain.
//!
//! HONEST SCOPE: the per-buyer position is committed (private) and the AGGREGATE book is a
//! commitment proven adequate — there is no public `total_cover` getter. The DECLARED premium is a
//! public token transfer; because the cover is hidden, the chain does NOT compute premium = cover *
//! bps, so the premium does not reveal the cover. Premium adequacy is priced and authorized by the
//! same keeper/sequencer that orders purchases and supplies the running-aggregate opening for the
//! proof (the coordination model documented in `solvency_v1`).
//!
//! LAW #1 intact: the default-payout entrypoint (`pay_allocations`) is settlement-only + proof-
//! gated. The solvency proofs gate buy/withdraw but NEVER authorize a payout — no admin/pause path
//! can move the reserve to payees. The four frozen surfaces (factory/bond/vault/settlement, the
//! 116-byte generic journal, the registry interface, the guest plug-in contract) are untouched;
//! the solvency journal (112 / 48 bytes) is a separate surface verified by this vault, not the
//! settlement contract.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, token, Address, Bytes,
    BytesN, Env, Vec,
};

const ACC_SCALE: i128 = 1_000_000_000_000; // 1e12, rewards-per-share fixed point
const BPS: i128 = 10_000;
const TTL_THRESHOLD: u32 = 50;
const TTL_EXTEND: u32 = 100;
const BUY_JOURNAL_LEN: u32 = 112; // prev(32) ‖ new(32) ‖ position(32) ‖ collateral(16 BE)
const WITHDRAW_JOURNAL_LEN: u32 = 48; // cover_commitment(32) ‖ collateral_after(16 BE)

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    BadAmount = 3,
    WindowClosed = 4,
    BadJournalLen = 5,
    PrevCommitmentMismatch = 6,
    Insolvent = 7,
    CollateralAfterMismatch = 8,
    InsufficientBalance = 9,
    NothingToClaim = 10,
    BadConfig = 11,
}

/// The network RISC Zero verifier router: verifies one Groth16 proof against `image_id` over the
/// journal's sha256 digest, trapping on a bad proof/selector. Same interface the settlement uses.
#[contractclient(name = "VerifierRouterClient")]
pub trait VerifierRouterInterface {
    fn verify(env: Env, seal: Bytes, image_id: BytesN<32>, journal: BytesN<32>);
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Settlement,
    CollateralToken,
    Admin,
    SolvencyImageId,
    Verifier,
    ProtocolFeeBps,
    // persistent accounting
    CoverCommitment, // BytesN<32> — the hidden running aggregate
    PositionRoot,    // BytesN<32>
    TotalCollateral, // i128
    ProtocolFeeAccrued,
    AccPremiumPerColl,
    Frozen,
    Seller(Address),
    SellerDebt(Address),
    SellerClaimable(Address),
}

#[contract]
pub struct ConfidentialVault;

#[contractimpl]
impl ConfidentialVault {
    /// `initial_cover_commitment` must be `commit_total(0, salt0)` (computed off-chain — the vault
    /// never runs Poseidon; it stores commitments and verifies proofs that bind them).
    pub fn init(
        env: Env,
        settlement: Address,
        collateral_token: Address,
        admin: Address,
        solvency_image_id: BytesN<32>,
        verifier: Address,
        protocol_fee_bps: u32,
        initial_cover_commitment: BytesN<32>,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::Settlement) {
            panic_with(&env, Error::AlreadyInitialized);
        }
        if protocol_fee_bps >= BPS as u32 {
            panic_with(&env, Error::BadConfig);
        }
        s.set(&DataKey::Settlement, &settlement);
        s.set(&DataKey::CollateralToken, &collateral_token);
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::SolvencyImageId, &solvency_image_id);
        s.set(&DataKey::Verifier, &verifier);
        s.set(&DataKey::ProtocolFeeBps, &protocol_fee_bps);
        s.set(&DataKey::Frozen, &false);
        let p = env.storage().persistent();
        p.set(&DataKey::CoverCommitment, &initial_cover_commitment);
        p.extend_ttl(&DataKey::CoverCommitment, TTL_THRESHOLD, TTL_EXTEND);
        p.set(&DataKey::PositionRoot, &BytesN::from_array(&env, &[0u8; 32]));
        p.extend_ttl(&DataKey::PositionRoot, TTL_THRESHOLD, TTL_EXTEND);
        s.extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
    }

    /// Underwriter deposits public collateral and accrues premium pro-rata to it.
    pub fn deposit(env: Env, seller: Address, amount: i128) {
        seller.require_auth();
        require_open(&env);
        if amount <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        settle_premium(&env, &seller);
        token_client(&env).transfer(&seller, &env.current_contract_address(), &amount);
        let bal = get_i128(&env, &DataKey::Seller(seller.clone())) + amount;
        set_i128(&env, &DataKey::Seller(seller.clone()), bal);
        bump_total_collateral(&env, amount);
        reset_debt(&env, &seller, bal);
        bump_instance(&env);
    }

    /// Buy confidential cover: a `solvency_v1` purchase proof attests the new aggregate <= collateral
    /// (cover hidden) and binds the buyer's position commitment. The vault advances its stored
    /// commitment, folds the position root, and collects the declared premium. The cover never
    /// appears on-chain.
    pub fn buy_protection_proven(env: Env, buyer: Address, seal: Bytes, journal: Bytes, premium: i128) {
        buyer.require_auth();
        require_open(&env);
        if premium < 0 {
            panic_with(&env, Error::BadAmount);
        }
        verify_solvency(&env, &seal, &journal);
        if journal.len() != BUY_JOURNAL_LEN {
            panic_with(&env, Error::BadJournalLen);
        }
        let prev = read_b32(&env, &journal, 0);
        let new = read_b32(&env, &journal, 32);
        let position = read_b32(&env, &journal, 64);
        let collateral = read_i128_be(&journal, 96);

        // the proof must advance from the aggregate we currently store ...
        let stored: BytesN<32> = env.storage().persistent().get(&DataKey::CoverCommitment).unwrap();
        if prev != stored {
            panic_with(&env, Error::PrevCommitmentMismatch);
        }
        // ... and the solvency bound it proved (new_total <= collateral) must hold against the REAL
        // reserve: collateral <= total_collateral ⇒ new_total <= total_collateral.
        if collateral > get_i128(&env, &DataKey::TotalCollateral) {
            panic_with(&env, Error::Insolvent);
        }

        let p = env.storage().persistent();
        p.set(&DataKey::CoverCommitment, &new);
        p.extend_ttl(&DataKey::CoverCommitment, TTL_THRESHOLD, TTL_EXTEND);
        fold_position_root(&env, &position);

        if premium > 0 {
            token_client(&env).transfer(&buyer, &env.current_contract_address(), &premium);
            distribute_premium(&env, premium);
        }
        bump_instance(&env);
    }

    /// Withdraw collateral. A `solvency_v1` withdrawal proof attests the hidden aggregate still fits
    /// under the post-withdrawal collateral, so the reserve can never drop below the outstanding
    /// (committed) book.
    pub fn withdraw_proven(env: Env, seller: Address, amount: i128, seal: Bytes, journal: Bytes) {
        seller.require_auth();
        if amount <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        settle_premium(&env, &seller);
        verify_solvency(&env, &seal, &journal);
        if journal.len() != WITHDRAW_JOURNAL_LEN {
            panic_with(&env, Error::BadJournalLen);
        }
        let cover_commitment = read_b32(&env, &journal, 0);
        let collateral_after = read_i128_be(&journal, 32);

        let stored: BytesN<32> = env.storage().persistent().get(&DataKey::CoverCommitment).unwrap();
        if cover_commitment != stored {
            panic_with(&env, Error::PrevCommitmentMismatch);
        }
        let total = get_i128(&env, &DataKey::TotalCollateral);
        // the proof checked aggregate <= collateral_after; it must be the REAL post-withdrawal reserve
        if collateral_after != total - amount {
            panic_with(&env, Error::CollateralAfterMismatch);
        }
        let bal = get_i128(&env, &DataKey::Seller(seller.clone()));
        if amount > bal {
            panic_with(&env, Error::InsufficientBalance);
        }
        set_i128(&env, &DataKey::Seller(seller.clone()), bal - amount);
        bump_total_collateral(&env, -amount);
        reset_debt(&env, &seller, bal - amount);
        token_client(&env).transfer(&env.current_contract_address(), &seller, &amount);
        bump_instance(&env);
    }

    pub fn claim_premium(env: Env, seller: Address) -> i128 {
        seller.require_auth();
        settle_premium(&env, &seller);
        let key = DataKey::SellerClaimable(seller.clone());
        let amt = get_i128(&env, &key);
        if amt <= 0 {
            panic_with(&env, Error::NothingToClaim);
        }
        set_i128(&env, &key, 0);
        token_client(&env).transfer(&env.current_contract_address(), &seller, &amt);
        bump_instance(&env);
        amt
    }

    pub fn claim_protocol_fee(env: Env) -> i128 {
        let admin = get_admin(&env);
        admin.require_auth();
        let amt = get_i128(&env, &DataKey::ProtocolFeeAccrued);
        if amt <= 0 {
            panic_with(&env, Error::NothingToClaim);
        }
        set_i128(&env, &DataKey::ProtocolFeeAccrued, 0);
        token_client(&env).transfer(&env.current_contract_address(), &admin, &amt);
        bump_instance(&env);
        amt
    }

    /// THE settlement-only payout (Law #1). The bound settlement contract calls this after it has
    /// verified one settlement Groth16 proof. Pays from the reserve; never from premium. The cover
    /// commitment is rebased per epoch by the keeper (a settled epoch's book is consumed) — the
    /// payout path deliberately does not touch it.
    pub fn pay_allocations(env: Env, _epoch: u32, allocations: Vec<(Address, i128)>) {
        let settlement: Address = env
            .storage()
            .instance()
            .get(&DataKey::Settlement)
            .unwrap_or_else(|| panic_with(&env, Error::NotInitialized));
        settlement.require_auth();

        let mut total: i128 = 0;
        for (_, amt) in allocations.iter() {
            total += amt;
        }
        let reserve = get_i128(&env, &DataKey::TotalCollateral);
        if total > reserve {
            panic_with(&env, Error::Insolvent);
        }
        bump_total_collateral(&env, -total);
        let token = token_client(&env);
        let me = env.current_contract_address();
        for (payee, amt) in allocations.iter() {
            if amt > 0 {
                token.transfer(&me, &payee, &amt);
            }
        }
        bump_instance(&env);
    }

    pub fn set_window(env: Env, open: bool) {
        get_admin(&env).require_auth();
        env.storage().instance().set(&DataKey::Frozen, &!open);
        bump_instance(&env);
    }

    // ---- getters (NOTE: deliberately NO total_cover getter — the aggregate stays hidden) ----
    pub fn settlement(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Settlement).unwrap()
    }
    pub fn collateral_token(env: Env) -> Address {
        env.storage().instance().get(&DataKey::CollateralToken).unwrap()
    }
    pub fn admin(env: Env) -> Address {
        get_admin(&env)
    }
    pub fn solvency_image_id(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::SolvencyImageId).unwrap()
    }
    pub fn verifier(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Verifier).unwrap()
    }
    pub fn cover_commitment(env: Env) -> BytesN<32> {
        env.storage().persistent().get(&DataKey::CoverCommitment).unwrap()
    }
    pub fn position_root(env: Env) -> BytesN<32> {
        env.storage().persistent().get(&DataKey::PositionRoot).unwrap()
    }
    pub fn total_collateral(env: Env) -> i128 {
        get_i128(&env, &DataKey::TotalCollateral)
    }
    pub fn seller_balance(env: Env, seller: Address) -> i128 {
        get_i128(&env, &DataKey::Seller(seller))
    }
    pub fn pending_premium(env: Env, seller: Address) -> i128 {
        let coll = get_i128(&env, &DataKey::Seller(seller.clone()));
        let acc = get_i128(&env, &DataKey::AccPremiumPerColl);
        let debt = get_i128(&env, &DataKey::SellerDebt(seller.clone()));
        let claimable = get_i128(&env, &DataKey::SellerClaimable(seller));
        claimable + (coll * acc / ACC_SCALE - debt)
    }
    pub fn protocol_fee_accrued(env: Env) -> i128 {
        get_i128(&env, &DataKey::ProtocolFeeAccrued)
    }
    pub fn is_frozen(env: Env) -> bool {
        env.storage().instance().get(&DataKey::Frozen).unwrap_or(false)
    }
}

// ---- internals ----

fn verify_solvency(env: &Env, seal: &Bytes, journal: &Bytes) {
    let s = env.storage().instance();
    let image_id: BytesN<32> = s.get(&DataKey::SolvencyImageId).unwrap();
    let verifier: Address = s.get(&DataKey::Verifier).unwrap();
    let digest = env.crypto().sha256(journal).to_bytes();
    VerifierRouterClient::new(env, &verifier).verify(seal, &image_id, &digest);
}

fn distribute_premium(env: &Env, premium: i128) {
    let fee_bps = get_u32(env, &DataKey::ProtocolFeeBps) as i128;
    let protocol_cut = premium * fee_bps / BPS;
    let sellers_cut = premium - protocol_cut;
    let total_coll = get_i128(env, &DataKey::TotalCollateral);
    let fee = get_i128(env, &DataKey::ProtocolFeeAccrued);
    if total_coll > 0 && sellers_cut > 0 {
        let acc = get_i128(env, &DataKey::AccPremiumPerColl);
        set_i128(env, &DataKey::AccPremiumPerColl, acc + sellers_cut * ACC_SCALE / total_coll);
        set_i128(env, &DataKey::ProtocolFeeAccrued, fee + protocol_cut);
    } else {
        // no underwriters to distribute to — the whole premium accrues to the protocol
        set_i128(env, &DataKey::ProtocolFeeAccrued, fee + premium);
    }
}

fn settle_premium(env: &Env, seller: &Address) {
    let coll = get_i128(env, &DataKey::Seller(seller.clone()));
    let acc = get_i128(env, &DataKey::AccPremiumPerColl);
    let debt = get_i128(env, &DataKey::SellerDebt(seller.clone()));
    let pending = coll * acc / ACC_SCALE - debt;
    if pending > 0 {
        let key = DataKey::SellerClaimable(seller.clone());
        let c = get_i128(env, &key);
        set_i128(env, &key, c + pending);
    }
    set_i128(env, &DataKey::SellerDebt(seller.clone()), coll * acc / ACC_SCALE);
}

fn reset_debt(env: &Env, seller: &Address, bal: i128) {
    let acc = get_i128(env, &DataKey::AccPremiumPerColl);
    set_i128(env, &DataKey::SellerDebt(seller.clone()), bal * acc / ACC_SCALE);
}

fn fold_position_root(env: &Env, commitment: &BytesN<32>) {
    let old: BytesN<32> = env.storage().persistent().get(&DataKey::PositionRoot).unwrap();
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_array(env, &old.to_array()));
    buf.append(&Bytes::from_array(env, &commitment.to_array()));
    let new = env.crypto().sha256(&buf).to_bytes();
    let p = env.storage().persistent();
    p.set(&DataKey::PositionRoot, &new);
    p.extend_ttl(&DataKey::PositionRoot, TTL_THRESHOLD, TTL_EXTEND);
}

fn bump_total_collateral(env: &Env, delta: i128) {
    let t = get_i128(env, &DataKey::TotalCollateral);
    set_i128(env, &DataKey::TotalCollateral, t + delta);
}

fn require_open(env: &Env) {
    let frozen: bool = env.storage().instance().get(&DataKey::Frozen).unwrap_or(false);
    if frozen {
        panic_with(env, Error::WindowClosed);
    }
}

// read a 32-byte field from the journal Bytes at `off`
fn read_b32(env: &Env, j: &Bytes, off: u32) -> BytesN<32> {
    let mut a = [0u8; 32];
    for i in 0..32u32 {
        a[i as usize] = j.get(off + i).unwrap();
    }
    BytesN::from_array(env, &a)
}
// read a 16-byte big-endian i128 (collateral values are non-negative)
fn read_i128_be(j: &Bytes, off: u32) -> i128 {
    let mut v: i128 = 0;
    for i in 0..16u32 {
        v = (v << 8) | (j.get(off + i).unwrap() as i128);
    }
    v
}

fn token_client(env: &Env) -> token::Client<'_> {
    let addr: Address = env.storage().instance().get(&DataKey::CollateralToken).unwrap();
    token::Client::new(env, &addr)
}
fn get_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}
fn bump_instance(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
}
fn get_i128(env: &Env, key: &DataKey) -> i128 {
    env.storage().persistent().get(key).unwrap_or(0)
}
fn set_i128(env: &Env, key: &DataKey, v: i128) {
    let p = env.storage().persistent();
    p.set(key, &v);
    p.extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND);
}
fn get_u32(env: &Env, key: &DataKey) -> u32 {
    env.storage().instance().get(key).unwrap_or(0)
}
fn panic_with(env: &Env, e: Error) -> ! {
    soroban_sdk::panic_with_error!(env, e)
}

mod test;
