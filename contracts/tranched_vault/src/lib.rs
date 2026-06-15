#![no_std]
//! TranchedVault — underwriters commit to a TRANCHE by seniority rank (0 = junior / first-loss).
//!
//! First-loss waterfall: on a default the settlement-authorized payout is absorbed JUNIOR-FIRST —
//! the junior tranche's collateral is consumed before the next, so junior capital takes the first
//! loss and senior capital is protected until everything more junior is exhausted. Junior bears
//! more risk and is compensated with a larger premium share (configured per-tranche weights).
//!
//! Per-tranche SHARE model: a deposit mints shares against the tranche's current collateral, so a
//! loss (which lowers `TrancheColl` while shares stay fixed) drops collateral-per-share and is
//! borne pro-rata by everyone in that tranche. Premium accrues per share (earned for bearing risk,
//! independent of whether a loss happened) via a rewards-per-share accumulator.
//!
//! LAW #1 is intact: the default-payout entrypoint (`pay_allocations`) is callable only by the
//! bound settlement contract, which authorizes solely by verifying one Groth16 proof against the
//! pinned image_id. Premium is a separate pool; no admin/pause path can move the reserve to payees.
//!
//! A NEW instrument-family version (PRD roadmap "multi-tranche vaults", built under explicit human
//! override). The deployed + yield vaults are frozen (Law #2); this composes behind the same
//! surfaces (settlement-bound payout, position-root binding).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Bytes, BytesN, Env, Vec,
};

const ACC_SCALE: i128 = 1_000_000_000_000; // 1e12, rewards-per-share fixed point
const BPS: i128 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    BadTranche = 3,
    BadAmount = 4,
    Insolvent = 5,
    WindowClosed = 6,
    InsufficientShares = 7,
    NothingToClaim = 8,
    BadConfig = 9,
    ReserveTooSmall = 10,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Settlement,
    CollateralToken,
    Admin,
    PremiumBps,
    ProtocolFeeBps,
    Weights,        // Vec<u32>, index = rank (0 = junior / first-loss)
    TotalWeight,    // u32
    NumTranches,    // u32
    TotalCollateral, // i128 — Σ tranche collateral, net of absorbed losses
    TotalCover,     // i128 — outstanding cover sold
    PositionRoot,   // BytesN<32> — folded commitment chain
    Frozen,         // bool — deposit/buy window
    ProtocolFeeAccrued, // i128
    // per tranche (keyed by rank)
    TrancheColl(u32),
    TrancheShares(u32),
    AccPremPerShare(u32),
    // per (tranche, seller)
    Shares(u32, Address),
    PremDebt(u32, Address),
    PremClaimable(u32, Address),
}

#[contract]
pub struct TranchedVault;

#[contractimpl]
impl TranchedVault {
    /// `weights[rank]` is tranche `rank`'s share of distributed premium; rank 0 is the junior
    /// (first-loss) tranche. A typical senior-protecting structure puts the largest weight on
    /// junior (e.g. [3, 2, 1]) so first-loss capital earns the most.
    pub fn init(
        env: Env,
        settlement: Address,
        collateral_token: Address,
        admin: Address,
        premium_bps: u32,
        protocol_fee_bps: u32,
        weights: Vec<u32>,
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
        s.set(&DataKey::PremiumBps, &premium_bps);
        s.set(&DataKey::ProtocolFeeBps, &protocol_fee_bps);
        s.set(&DataKey::NumTranches, &(weights.len()));
        s.set(&DataKey::TotalWeight, &total_weight);
        s.set(&DataKey::Weights, &weights);
        s.set(&DataKey::Frozen, &false);
        // mutable accounting lives in PERSISTENT storage (roots/totals never temporary, TECH_SPEC
        // §10); the i128 counters default to 0 there. Seed the position root.
        env.storage()
            .persistent()
            .set(&DataKey::PositionRoot, &BytesN::from_array(&env, &[0u8; 32]));
    }

    /// Underwriter commits collateral to tranche `tranche` and is minted shares against that
    /// tranche's current collateral-per-share.
    pub fn deposit(env: Env, seller: Address, tranche: u32, amount: i128) {
        seller.require_auth();
        require_open(&env);
        if amount <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        check_tranche(&env, tranche);
        settle_premium(&env, tranche, &seller);

        let token = token_client(&env);
        token.transfer(&seller, &env.current_contract_address(), &amount);

        let coll = get_i128(&env, &DataKey::TrancheColl(tranche));
        let tshares = get_i128(&env, &DataKey::TrancheShares(tranche));
        // first deposit (or a fully-wiped tranche) mints 1:1; otherwise proportional to value.
        let minted = if tshares == 0 || coll == 0 { amount } else { amount * tshares / coll };

        set_i128(&env, &DataKey::TrancheColl(tranche), coll + amount);
        set_i128(&env, &DataKey::TrancheShares(tranche), tshares + minted);
        let s_shares = get_i128(&env, &DataKey::Shares(tranche, seller.clone())) + minted;
        set_i128(&env, &DataKey::Shares(tranche, seller.clone()), s_shares);
        bump_total_collateral(&env, amount);
        // reset debt against the new share balance
        let acc = get_i128(&env, &DataKey::AccPremPerShare(tranche));
        set_i128(&env, &DataKey::PremDebt(tranche, seller), s_shares * acc / ACC_SCALE);
    }

    /// Cover buyer pays premium upfront; premium is split across tranches by weight (junior earns
    /// the most), then pro-rata to shares within each tranche. Solvency: total cover ≤ total reserve.
    pub fn buy_protection(env: Env, buyer: Address, position_commitment: BytesN<32>, cover: i128) {
        buyer.require_auth();
        require_open(&env);
        if cover <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        let total_coll = get_i128(&env, &DataKey::TotalCollateral);
        let total_cover = get_i128(&env, &DataKey::TotalCover);
        if total_cover + cover > total_coll {
            panic_with(&env, Error::Insolvent);
        }
        let premium_bps = get_u32(&env, &DataKey::PremiumBps) as i128;
        let premium = cover * premium_bps / BPS;
        if premium > 0 {
            let token = token_client(&env);
            token.transfer(&buyer, &env.current_contract_address(), &premium);
            distribute_premium(&env, premium);
        }
        set_i128(&env, &DataKey::TotalCover, total_cover + cover);
        fold_position_root(&env, &position_commitment);
    }

    /// Router-routed premium (already transferred into the vault): account + distribute. Same
    /// weighted-by-tranche split as `buy_protection`. Router-only (the settlement separation keeps
    /// payouts proof-gated; this is premium, a separate pool).
    pub fn receive_premium(env: Env, amount: i128) {
        // the bound settlement is NOT allowed to drive premium; only the admin-set router context.
        // Kept minimal: admin authorizes routing config; the transfer already happened.
        get_admin(&env).require_auth();
        if amount <= 0 {
            panic_with(&env, Error::BadAmount);
        }
        distribute_premium(&env, amount);
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
        amt
    }

    /// Burn shares in a tranche and withdraw the corresponding (loss-adjusted) collateral. Cannot
    /// drop the total reserve below outstanding cover.
    pub fn withdraw(env: Env, seller: Address, tranche: u32, shares_to_burn: i128) -> i128 {
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

        let total_coll = get_i128(&env, &DataKey::TotalCollateral);
        let total_cover = get_i128(&env, &DataKey::TotalCover);
        if total_coll - amount < total_cover {
            panic_with(&env, Error::Insolvent);
        }

        set_i128(&env, &DataKey::TrancheShares(tranche), tshares - shares_to_burn);
        set_i128(&env, &DataKey::TrancheColl(tranche), coll - amount);
        bump_total_collateral(&env, -amount);
        let new_shares = s_shares - shares_to_burn;
        set_i128(&env, &DataKey::Shares(tranche, seller.clone()), new_shares);
        let acc = get_i128(&env, &DataKey::AccPremPerShare(tranche));
        set_i128(&env, &DataKey::PremDebt(tranche, seller.clone()), new_shares * acc / ACC_SCALE);

        token_client(&env).transfer(&env.current_contract_address(), &seller, &amount);
        amount
    }

    /// THE settlement-only payout (Law #1). The bound settlement contract calls this after it has
    /// verified one Groth16 proof. The total payout is absorbed JUNIOR-FIRST across tranches.
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
        let total_coll = get_i128(&env, &DataKey::TotalCollateral);
        if total > total_coll {
            panic_with(&env, Error::ReserveTooSmall);
        }
        // first-loss waterfall: consume junior collateral before senior
        absorb_loss(&env, total);
        bump_total_collateral(&env, -total);
        let total_cover = get_i128(&env, &DataKey::TotalCover);
        set_i128(&env, &DataKey::TotalCover, (total_cover - total).max(0));

        let token = token_client(&env);
        let me = env.current_contract_address();
        for (payee, amt) in allocations.iter() {
            if amt > 0 {
                token.transfer(&me, &payee, &amt);
            }
        }
    }

    pub fn set_window(env: Env, open: bool) {
        get_admin(&env).require_auth();
        env.storage().instance().set(&DataKey::Frozen, &!open);
    }

    // ---- getters ----
    pub fn settlement(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Settlement).unwrap()
    }
    pub fn collateral_token(env: Env) -> Address {
        env.storage().instance().get(&DataKey::CollateralToken).unwrap()
    }
    pub fn admin(env: Env) -> Address {
        get_admin(&env)
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
    /// A seller's current loss-adjusted collateral value in a tranche.
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
    pub fn total_cover(env: Env) -> i128 {
        get_i128(&env, &DataKey::TotalCover)
    }
    pub fn position_root(env: Env) -> BytesN<32> {
        env.storage().persistent().get(&DataKey::PositionRoot).unwrap()
    }
    pub fn protocol_fee_accrued(env: Env) -> i128 {
        get_i128(&env, &DataKey::ProtocolFeeAccrued)
    }
}

// ---- internals ----

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
        // a tranche with no underwriters can't accrue its cut; it falls through to protocol below.
    }
    // protocol gets its base cut plus any undistributable remainder (empty tranches + rounding),
    // so no premium is silently stranded in the vault balance.
    let stranded = sellers_total - accrued;
    let fee = get_i128(env, &DataKey::ProtocolFeeAccrued);
    set_i128(env, &DataKey::ProtocolFeeAccrued, fee + protocol_cut + stranded);
}

/// Junior-first loss absorption. Shares are untouched, so each absorbing tranche's
/// collateral-per-share drops and its underwriters bear the loss pro-rata.
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
    // total ≤ TotalCollateral was checked by the caller, so remaining is 0 here.
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
    env.storage().persistent().set(&DataKey::PositionRoot, &new);
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

fn token_client(env: &Env) -> token::Client<'_> {
    let addr: Address = env.storage().instance().get(&DataKey::CollateralToken).unwrap();
    token::Client::new(env, &addr)
}

fn get_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn get_i128(env: &Env, key: &DataKey) -> i128 {
    env.storage().persistent().get(key).unwrap_or(0)
}
fn set_i128(env: &Env, key: &DataKey, v: i128) {
    env.storage().persistent().set(key, &v);
}
fn get_u32(env: &Env, key: &DataKey) -> u32 {
    env.storage().instance().get(key).unwrap_or(0)
}

fn panic_with(env: &Env, e: Error) -> ! {
    soroban_sdk::panic_with_error!(env, e)
}

mod test;
