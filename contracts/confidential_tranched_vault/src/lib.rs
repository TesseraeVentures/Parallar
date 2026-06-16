#![no_std]
//! ConfidentialTranchedVault — confidential cover over a first-loss tranche structure.
//!
//! Two built primitives composed into one instrument-family version:
//!   • CONFIDENTIAL cover (from `confidential_vault` / `solvency_v1`): the running AGGREGATE cover is
//!     a Poseidon COMMITMENT, never a public total. Each purchase carries a `solvency_v1` proof that
//!     the new aggregate <= total collateral (cover + totals hidden); each withdrawal carries a proof
//!     that the aggregate still fits under the post-withdrawal collateral.
//!   • FIRST-LOSS TRANCHES (from `tranched_vault`): underwriters commit to a tranche by seniority
//!     rank (0 = junior / first-loss). A default is absorbed JUNIOR-FIRST across tranches; each
//!     tranche uses a share model so its loss is shared pro-rata. Premium is split across tranches
//!     by configured weight (junior earns the most), then pro-rata to shares within a tranche.
//!
//! The solvency bound is on the TOTAL reserve (Σ tranche collateral); the tranches only order loss
//! absorption, so a single `solvency_v1` proof serves the tranched reserve unchanged.
//!
//! LAW #1 intact: `pay_allocations` is settlement-only + proof-gated; the first-loss ordering is a
//! pure accounting waterfall. The solvency proofs gate buy/withdraw but NEVER authorize a payout.
//! HONEST SCOPE: per-buyer position committed + aggregate book hidden (proven adequate, no public
//! getter); the DECLARED premium is public but the chain never computes premium = cover*bps, so it
//! does not reveal the cover (adequacy priced by the keeper that supplies the aggregate opening).

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, token, Address, Bytes,
    BytesN, Env, Vec,
};

const ACC_SCALE: i128 = 1_000_000_000_000;
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
    BadTranche = 3,
    BadAmount = 4,
    WindowClosed = 5,
    BadJournalLen = 6,
    PrevCommitmentMismatch = 7,
    Insolvent = 8,
    CollateralAfterMismatch = 9,
    InsufficientShares = 10,
    NothingToClaim = 11,
    BadConfig = 12,
    ReserveTooSmall = 13,
}

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
    Weights,     // Vec<u32>, index = rank (0 = junior)
    TotalWeight, // u32
    NumTranches, // u32
    Frozen,
    // persistent accounting
    CoverCommitment, // BytesN<32> — hidden running aggregate
    PositionRoot,    // BytesN<32>
    TotalCollateral, // i128 — Σ tranche collateral, net of absorbed losses
    ProtocolFeeAccrued,
    TrancheColl(u32),
    TrancheShares(u32),
    AccPremPerShare(u32),
    Shares(u32, Address),
    PremDebt(u32, Address),
    PremClaimable(u32, Address),
}

#[contract]
pub struct ConfidentialTranchedVault;

#[contractimpl]
impl ConfidentialTranchedVault {
    /// `weights[rank]` is tranche `rank`'s premium share (rank 0 = junior/first-loss).
    /// `initial_cover_commitment` must be `commit_total(0, salt0)` (computed off-chain).
    pub fn init(
        env: Env,
        settlement: Address,
        collateral_token: Address,
        admin: Address,
        solvency_image_id: BytesN<32>,
        verifier: Address,
        protocol_fee_bps: u32,
        weights: Vec<u32>,
        initial_cover_commitment: BytesN<32>,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::Settlement) {
            panic_with(&env, Error::AlreadyInitialized);
        }
        if weights.is_empty() || protocol_fee_bps >= BPS as u32 {
            panic_with(&env, Error::BadConfig);
        }
        let mut total_weight: u32 = 0;
        for w in weights.iter() {
            total_weight = total_weight.checked_add(w).expect("weight overflow");
        }
        if total_weight == 0 {
            panic_with(&env, Error::BadConfig);
        }
        s.set(&DataKey::Settlement, &settlement);
        s.set(&DataKey::CollateralToken, &collateral_token);
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::SolvencyImageId, &solvency_image_id);
        s.set(&DataKey::Verifier, &verifier);
        s.set(&DataKey::ProtocolFeeBps, &protocol_fee_bps);
        s.set(&DataKey::NumTranches, &(weights.len()));
        s.set(&DataKey::TotalWeight, &total_weight);
        s.set(&DataKey::Weights, &weights);
        s.set(&DataKey::Frozen, &false);
        let p = env.storage().persistent();
        p.set(&DataKey::CoverCommitment, &initial_cover_commitment);
        p.extend_ttl(&DataKey::CoverCommitment, TTL_THRESHOLD, TTL_EXTEND);
        p.set(&DataKey::PositionRoot, &BytesN::from_array(&env, &[0u8; 32]));
        p.extend_ttl(&DataKey::PositionRoot, TTL_THRESHOLD, TTL_EXTEND);
        s.extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
    }

    /// Underwriter commits collateral to a tranche; shares minted against the tranche's current
    /// collateral-per-share (so later losses are borne pro-rata).
    pub fn deposit(env: Env, seller: Address, tranche: u32, amount: i128) {
        seller.require_auth();
        require_open(&env);
        if amount <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        check_tranche(&env, tranche);
        settle_premium(&env, tranche, &seller);

        token_client(&env).transfer(&seller, &env.current_contract_address(), &amount);
        let coll = get_i128(&env, &DataKey::TrancheColl(tranche));
        let tshares = get_i128(&env, &DataKey::TrancheShares(tranche));
        let minted = if tshares == 0 || coll == 0 { amount } else { amount * tshares / coll };
        set_i128(&env, &DataKey::TrancheColl(tranche), coll + amount);
        set_i128(&env, &DataKey::TrancheShares(tranche), tshares + minted);
        let s_shares = get_i128(&env, &DataKey::Shares(tranche, seller.clone())) + minted;
        set_i128(&env, &DataKey::Shares(tranche, seller.clone()), s_shares);
        bump_total_collateral(&env, amount);
        let acc = get_i128(&env, &DataKey::AccPremPerShare(tranche));
        set_i128(&env, &DataKey::PremDebt(tranche, seller), s_shares * acc / ACC_SCALE);
        bump_instance(&env);
    }

    /// Buy confidential cover: a `solvency_v1` purchase proof attests the new aggregate <= total
    /// reserve (cover hidden) and binds the buyer's position commitment. The vault advances its
    /// stored commitment, folds the position root, and splits the declared premium across tranches
    /// by weight. The cover never appears on-chain.
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

        let stored: BytesN<32> = env.storage().persistent().get(&DataKey::CoverCommitment).unwrap();
        if prev != stored {
            panic_with(&env, Error::PrevCommitmentMismatch);
        }
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

    /// Burn shares in a tranche and withdraw the (loss-adjusted) collateral, gated by a `solvency_v1`
    /// withdrawal proof that the hidden aggregate still fits under the post-withdrawal total reserve.
    pub fn withdraw_proven(env: Env, seller: Address, tranche: u32, shares_to_burn: i128, seal: Bytes, journal: Bytes) -> i128 {
        seller.require_auth();
        check_tranche(&env, tranche);
        if shares_to_burn <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        settle_premium(&env, tranche, &seller);
        let s_shares = get_i128(&env, &DataKey::Shares(tranche, seller.clone()));
        if shares_to_burn > s_shares {
            panic_with(&env, Error::InsufficientShares);
        }
        let coll = get_i128(&env, &DataKey::TrancheColl(tranche));
        let tshares = get_i128(&env, &DataKey::TrancheShares(tranche));
        let amount = shares_to_burn * coll / tshares;

        // the withdrawal must preserve solvency over the HIDDEN aggregate
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
        if collateral_after != total - amount {
            panic_with(&env, Error::CollateralAfterMismatch);
        }

        set_i128(&env, &DataKey::TrancheShares(tranche), tshares - shares_to_burn);
        set_i128(&env, &DataKey::TrancheColl(tranche), coll - amount);
        bump_total_collateral(&env, -amount);
        let new_shares = s_shares - shares_to_burn;
        set_i128(&env, &DataKey::Shares(tranche, seller.clone()), new_shares);
        let acc = get_i128(&env, &DataKey::AccPremPerShare(tranche));
        set_i128(&env, &DataKey::PremDebt(tranche, seller.clone()), new_shares * acc / ACC_SCALE);
        token_client(&env).transfer(&env.current_contract_address(), &seller, &amount);
        bump_instance(&env);
        amount
    }

    pub fn claim_premium(env: Env, seller: Address, tranche: u32) -> i128 {
        seller.require_auth();
        check_tranche(&env, tranche);
        settle_premium(&env, tranche, &seller);
        let key = DataKey::PremClaimable(tranche, seller.clone());
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

    /// THE settlement-only payout (Law #1). The bound settlement calls this after verifying one
    /// settlement proof; the total is absorbed JUNIOR-FIRST across tranches. The cover commitment is
    /// rebased per epoch by the keeper (a settled epoch's book is consumed).
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
        if total > get_i128(&env, &DataKey::TotalCollateral) {
            panic_with(&env, Error::ReserveTooSmall);
        }
        absorb_loss(&env, total); // junior-first
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

    // ---- getters (NO total_cover getter — the aggregate stays hidden) ----
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
    pub fn num_tranches(env: Env) -> u32 {
        get_u32(&env, &DataKey::NumTranches)
    }
    pub fn premium_weight(env: Env, tranche: u32) -> u32 {
        check_tranche(&env, tranche);
        let weights: Vec<u32> = env.storage().instance().get(&DataKey::Weights).unwrap();
        weights.get(tranche).unwrap()
    }
    pub fn tranche_collateral(env: Env, tranche: u32) -> i128 {
        get_i128(&env, &DataKey::TrancheColl(tranche))
    }
    pub fn tranche_shares(env: Env, tranche: u32) -> i128 {
        get_i128(&env, &DataKey::TrancheShares(tranche))
    }
    pub fn seller_shares(env: Env, seller: Address, tranche: u32) -> i128 {
        get_i128(&env, &DataKey::Shares(tranche, seller))
    }
    pub fn seller_value(env: Env, seller: Address, tranche: u32) -> i128 {
        let tshares = get_i128(&env, &DataKey::TrancheShares(tranche));
        if tshares == 0 {
            return 0;
        }
        let s_shares = get_i128(&env, &DataKey::Shares(tranche, seller));
        let coll = get_i128(&env, &DataKey::TrancheColl(tranche));
        s_shares * coll / tshares
    }
    pub fn pending_premium(env: Env, seller: Address, tranche: u32) -> i128 {
        let s_shares = get_i128(&env, &DataKey::Shares(tranche, seller.clone()));
        let acc = get_i128(&env, &DataKey::AccPremPerShare(tranche));
        let debt = get_i128(&env, &DataKey::PremDebt(tranche, seller.clone()));
        let claimable = get_i128(&env, &DataKey::PremClaimable(tranche, seller));
        claimable + (s_shares * acc / ACC_SCALE - debt)
    }
    pub fn total_collateral(env: Env) -> i128 {
        get_i128(&env, &DataKey::TotalCollateral)
    }
    pub fn protocol_fee_accrued(env: Env) -> i128 {
        get_i128(&env, &DataKey::ProtocolFeeAccrued)
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

/// Split premium across tranches by weight (junior largest), then pro-rata to shares within each.
fn distribute_premium(env: &Env, premium: i128) {
    let fee_bps = get_u32(env, &DataKey::ProtocolFeeBps) as i128;
    let protocol_cut = premium * fee_bps / BPS;
    let sellers_total = premium - protocol_cut;
    let total_weight = get_u32(env, &DataKey::TotalWeight) as i128;
    let n = get_u32(env, &DataKey::NumTranches);
    let weights: Vec<u32> = env.storage().instance().get(&DataKey::Weights).unwrap();

    let mut accrued: i128 = 0;
    for rank in 0..n {
        let weight = weights.get(rank).unwrap() as i128;
        let cut = sellers_total * weight / total_weight;
        let shares = get_i128(env, &DataKey::TrancheShares(rank));
        if shares > 0 && cut > 0 {
            let acc = get_i128(env, &DataKey::AccPremPerShare(rank));
            set_i128(env, &DataKey::AccPremPerShare(rank), acc + cut * ACC_SCALE / shares);
            accrued += cut;
        }
    }
    // protocol gets its base cut plus any undistributable remainder (empty tranches + rounding)
    let stranded = sellers_total - accrued;
    let fee = get_i128(env, &DataKey::ProtocolFeeAccrued);
    set_i128(env, &DataKey::ProtocolFeeAccrued, fee + protocol_cut + stranded);
}

/// Junior-first loss absorption. Shares untouched, so each absorbing tranche's collateral-per-share
/// drops and its underwriters bear the loss pro-rata.
fn absorb_loss(env: &Env, loss: i128) {
    let n = get_u32(env, &DataKey::NumTranches);
    let mut remaining = loss;
    for rank in 0..n {
        if remaining == 0 {
            break;
        }
        let coll = get_i128(env, &DataKey::TrancheColl(rank));
        let absorbed = if coll < remaining { coll } else { remaining };
        if absorbed > 0 {
            set_i128(env, &DataKey::TrancheColl(rank), coll - absorbed);
            remaining -= absorbed;
        }
    }
}

fn settle_premium(env: &Env, tranche: u32, seller: &Address) {
    let s_shares = get_i128(env, &DataKey::Shares(tranche, seller.clone()));
    let acc = get_i128(env, &DataKey::AccPremPerShare(tranche));
    let debt = get_i128(env, &DataKey::PremDebt(tranche, seller.clone()));
    let pending = s_shares * acc / ACC_SCALE - debt;
    if pending > 0 {
        let key = DataKey::PremClaimable(tranche, seller.clone());
        let c = get_i128(env, &key);
        set_i128(env, &key, c + pending);
    }
    set_i128(env, &DataKey::PremDebt(tranche, seller.clone()), s_shares * acc / ACC_SCALE);
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

fn check_tranche(env: &Env, tranche: u32) {
    if tranche >= get_u32(env, &DataKey::NumTranches) {
        panic_with(env, Error::BadTranche);
    }
}
fn require_open(env: &Env) {
    let frozen: bool = env.storage().instance().get(&DataKey::Frozen).unwrap_or(false);
    if frozen {
        panic_with(env, Error::WindowClosed);
    }
}
fn read_b32(env: &Env, j: &Bytes, off: u32) -> BytesN<32> {
    let mut a = [0u8; 32];
    for i in 0..32u32 {
        a[i as usize] = j.get(off + i).unwrap();
    }
    BytesN::from_array(env, &a)
}
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
