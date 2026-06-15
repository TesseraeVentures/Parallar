#![no_std]
//! YieldVault — a premium-aware reserve vault (TECH_SPEC §3.2 / §5A; G11/G12).
//!
//! A NEW instrument-family version; the deployed generic vault stays frozen (Law #2). It makes
//! BOTH sides' economics real:
//!   • buyers PAY a premium when they buy cover (`buy_protection`), or premium arrives from routed
//!     coupons (`receive_premium`, called by the YieldRouter);
//!   • underwriters EARN that premium pro-rata to their collateral, via a rewards-per-share accrual
//!     (`acc_premium_per_collateral`), claimable any time;
//!   • the protocol takes a base fee on premium (~12%, §5A).
//!
//! ARCHITECTURAL LAW #1 holds: the reserve moves to a payee SOLELY via `pay_allocations`, callable
//! only by the bound settlement contract (the proof-gated default path) — UNCHANGED. Premium is a
//! separate flow that can never authorize a payout; it is distributed to sellers / the protocol,
//! never to buyers. The reserve (`total_collateral`) and the premium pools are accounted
//! separately though they share the token balance, so neither path can spend the other's funds.
//!
//! Solvency carries a liquidity haircut (`total_cover ≤ (1−h)·collateral`, §3.2) for NAV-floating
//! reserves; h=0 for XLM. The float-strategy adapter (G12) slots in without touching this surface.

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, token, Address, Bytes, BytesN, Env, Vec};

/// Fixed-point scale for the rewards-per-share accumulator (premium per unit collateral).
const ACC_SCALE: i128 = 1_000_000_000_000;
const BPS: i128 = 10_000;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Settlement,
    CollateralToken,
    Admin,
    Router,            // the bound YieldRouter (transparent protected-share-class path)
    PremiumBps,
    ProtocolFeeBps,
    HaircutBps,
    TotalCollateral,
    TotalCover,        // SEALED cover (from buy_protection commitments)
    RoutedCover,       // cover registered by the router (wrapped balances; public)
    PositionRoot,
    Frozen,
    AccPremiumPerColl, // i128, scaled by ACC_SCALE
    ProtocolFeeAccrued,
    Seller(Address),         // collateral
    SellerDebt(Address),     // acc-per-coll snapshot at last settle (scaled)
    SellerClaimable(Address),// settled, unclaimed premium
}

#[contractevent(data_format = "single-value")]
pub struct AllocationsPaid {
    #[topic]
    pub epoch: u32,
    pub total: i128,
}

#[contractevent(data_format = "single-value")]
pub struct PremiumDistributed {
    #[topic]
    pub to_sellers: i128,
    pub protocol_fee: i128,
}

#[contract]
pub struct YieldVault;

fn fold(env: &Env, prev: &BytesN<32>, commitment: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_array(env, &prev.to_array()));
    buf.append(&Bytes::from_array(env, &commitment.to_array()));
    env.crypto().sha256(&buf).to_bytes()
}
fn is_frozen(env: &Env) -> bool {
    env.storage().instance().get(&DataKey::Frozen).unwrap_or(false)
}
fn get_i128(env: &Env, k: &DataKey) -> i128 {
    env.storage().instance().get(k).unwrap_or(0)
}

/// Distribute `amount` of premium: a protocol-fee cut accrues to the protocol; the rest raises the
/// per-collateral accumulator so every seller earns pro-rata to their stake. Premium NEVER touches
/// `total_collateral` (the reserve) — it is yield on top, claimable separately.
fn distribute_premium(env: &Env, amount: i128) {
    let s = env.storage().instance();
    let total_coll = get_i128(env, &DataKey::TotalCollateral);
    assert!(total_coll > 0, "no collateral to distribute premium to");
    let fee_bps: i128 = s.get(&DataKey::ProtocolFeeBps).unwrap();
    let protocol_cut = amount * fee_bps / BPS;
    let sellers_cut = amount - protocol_cut;
    let acc = get_i128(env, &DataKey::AccPremiumPerColl);
    s.set(&DataKey::AccPremiumPerColl, &(acc + sellers_cut * ACC_SCALE / total_coll));
    s.set(&DataKey::ProtocolFeeAccrued, &(get_i128(env, &DataKey::ProtocolFeeAccrued) + protocol_cut));
    PremiumDistributed { to_sellers: sellers_cut, protocol_fee: protocol_cut }.publish(env);
}

/// Settle a seller's accrued premium into their claimable bucket and reset their debt — call
/// before any change to their collateral (deposit/withdraw) or before a claim.
fn settle_seller(env: &Env, seller: &Address) {
    let s = env.storage().instance();
    let coll = get_i128(env, &DataKey::Seller(seller.clone()));
    let acc = get_i128(env, &DataKey::AccPremiumPerColl);
    let debt = get_i128(env, &DataKey::SellerDebt(seller.clone()));
    let pending = coll * acc / ACC_SCALE - debt;
    if pending > 0 {
        let claimable = get_i128(env, &DataKey::SellerClaimable(seller.clone()));
        s.set(&DataKey::SellerClaimable(seller.clone()), &(claimable + pending));
    }
    s.set(&DataKey::SellerDebt(seller.clone()), &(coll * acc / ACC_SCALE));
}

#[contractimpl]
impl YieldVault {
    pub fn init(
        env: Env,
        settlement: Address,
        collateral_token: Address,
        admin: Address,
        premium_bps: u32,
        protocol_fee_bps: u32,
        haircut_bps: u32,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::Settlement) {
            panic!("already initialized");
        }
        assert!(protocol_fee_bps <= 10_000 && haircut_bps < 10_000, "bad bps");
        s.set(&DataKey::Settlement, &settlement);
        s.set(&DataKey::CollateralToken, &collateral_token);
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::PremiumBps, &(premium_bps as i128));
        s.set(&DataKey::ProtocolFeeBps, &(protocol_fee_bps as i128));
        s.set(&DataKey::HaircutBps, &(haircut_bps as i128));
        s.set(&DataKey::TotalCollateral, &0i128);
        s.set(&DataKey::TotalCover, &0i128);
        s.set(&DataKey::RoutedCover, &0i128);
        s.set(&DataKey::PositionRoot, &BytesN::from_array(&env, &[0u8; 32]));
        s.set(&DataKey::Frozen, &false);
        s.set(&DataKey::AccPremiumPerColl, &0i128);
        s.set(&DataKey::ProtocolFeeAccrued, &0i128);
        s.extend_ttl(50, 100);
    }

    /// Underwriter deposits collateral. Settles any pending premium first so the new stake doesn't
    /// distort past accrual.
    pub fn deposit(env: Env, seller: Address, amount: i128) {
        seller.require_auth();
        assert!(amount > 0, "amount must be positive");
        let s = env.storage().instance();
        settle_seller(&env, &seller);
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        token::Client::new(&env, &token_addr).transfer(&seller, &env.current_contract_address(), &amount);
        s.set(&DataKey::TotalCollateral, &(get_i128(&env, &DataKey::TotalCollateral) + amount));
        let coll = get_i128(&env, &DataKey::Seller(seller.clone())) + amount;
        s.set(&DataKey::Seller(seller.clone()), &coll);
        let acc = get_i128(&env, &DataKey::AccPremiumPerColl);
        s.set(&DataKey::SellerDebt(seller), &(coll * acc / ACC_SCALE));
        s.extend_ttl(50, 100);
    }

    /// Buyer buys cover and PAYS the premium (cover × premium_bps), distributed to sellers + the
    /// protocol. Stores the opaque commitment, folds position_root, enforces the solvency floor.
    pub fn buy_protection(env: Env, buyer: Address, commitment: BytesN<32>, cover: i128) {
        buyer.require_auth();
        assert!(cover > 0, "cover must be positive");
        assert!(!is_frozen(&env), "settlement window: vault frozen");
        let s = env.storage().instance();
        let total_coll = get_i128(&env, &DataKey::TotalCollateral);
        let cur_cover = get_i128(&env, &DataKey::TotalCover);
        let routed = get_i128(&env, &DataKey::RoutedCover);
        let haircut: i128 = s.get(&DataKey::HaircutBps).unwrap();
        // sealed + routed + new cover ≤ (1 − h)·collateral
        assert!((cur_cover + routed + cover) * BPS <= total_coll * (BPS - haircut), "insolvent: cover would exceed (1-haircut)·collateral");

        let premium_bps: i128 = s.get(&DataKey::PremiumBps).unwrap();
        let premium = cover * premium_bps / BPS;
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        if premium > 0 {
            token::Client::new(&env, &token_addr).transfer(&buyer, &env.current_contract_address(), &premium);
            distribute_premium(&env, premium);
        }
        let root: BytesN<32> = s.get(&DataKey::PositionRoot).unwrap();
        s.set(&DataKey::PositionRoot, &fold(&env, &root, &commitment));
        s.set(&DataKey::TotalCover, &(cur_cover + cover));
        s.extend_ttl(50, 100);
    }

    /// Premium arriving from routed coupons. The bound ROUTER transfers the tokens to this vault
    /// FIRST (it is the current contract for that transfer), then calls this to distribute them.
    /// Router-only: only the bound router may account premium, and it always transfers beforehand.
    pub fn receive_premium(env: Env, amount: i128) {
        assert!(amount > 0, "amount must be positive");
        let s = env.storage().instance();
        let router: Address = s.get(&DataKey::Router).expect("router not set");
        router.require_auth();
        distribute_premium(&env, amount);
        s.extend_ttl(50, 100);
    }

    /// Underwriter claims accrued premium.
    pub fn claim_premium(env: Env, seller: Address) {
        seller.require_auth();
        settle_seller(&env, &seller);
        let s = env.storage().instance();
        let claimable = get_i128(&env, &DataKey::SellerClaimable(seller.clone()));
        assert!(claimable > 0, "nothing to claim");
        s.set(&DataKey::SellerClaimable(seller.clone()), &0i128);
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        token::Client::new(&env, &token_addr).transfer(&env.current_contract_address(), &seller, &claimable);
        s.extend_ttl(50, 100);
    }

    /// Protocol claims its accrued base fee.
    pub fn claim_protocol_fee(env: Env) {
        let s = env.storage().instance();
        let admin: Address = s.get(&DataKey::Admin).unwrap();
        admin.require_auth();
        let accrued = get_i128(&env, &DataKey::ProtocolFeeAccrued);
        assert!(accrued > 0, "nothing to claim");
        s.set(&DataKey::ProtocolFeeAccrued, &0i128);
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        token::Client::new(&env, &token_addr).transfer(&env.current_contract_address(), &admin, &accrued);
        s.extend_ttl(50, 100);
    }

    /// Seller withdraws collateral. Settles premium first; cannot drop the reserve below the
    /// haircut-adjusted cover floor; frozen during settlement windows.
    pub fn withdraw(env: Env, seller: Address, amount: i128) {
        seller.require_auth();
        assert!(amount > 0, "amount must be positive");
        assert!(!is_frozen(&env), "settlement window: withdrawals frozen");
        let s = env.storage().instance();
        settle_seller(&env, &seller);
        let coll = get_i128(&env, &DataKey::Seller(seller.clone()));
        assert!(coll >= amount, "insufficient seller balance");
        let total_coll = get_i128(&env, &DataKey::TotalCollateral);
        let cover = get_i128(&env, &DataKey::TotalCover) + get_i128(&env, &DataKey::RoutedCover);
        let haircut: i128 = s.get(&DataKey::HaircutBps).unwrap();
        assert!((total_coll - amount) * (BPS - haircut) >= cover * BPS, "would drop reserve below the cover floor");
        let token_addr: Address = s.get(&DataKey::CollateralToken).unwrap();
        token::Client::new(&env, &token_addr).transfer(&env.current_contract_address(), &seller, &amount);
        let new_coll = coll - amount;
        s.set(&DataKey::Seller(seller.clone()), &new_coll);
        s.set(&DataKey::TotalCollateral, &(total_coll - amount));
        let acc = get_i128(&env, &DataKey::AccPremiumPerColl);
        s.set(&DataKey::SellerDebt(seller), &(new_coll * acc / ACC_SCALE));
        s.extend_ttl(50, 100);
    }

    /// Execute payouts — the SOLE path the reserve leaves to buyers, callable only by the bound
    /// settlement (Law #1). UNCHANGED from the generic vault; pays from `total_collateral` only,
    /// never from the premium pools.
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
        let total = get_i128(&env, &DataKey::TotalCollateral);
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

    pub fn set_window(env: Env, open: bool) {
        let s = env.storage().instance();
        let settlement: Address = s.get(&DataKey::Settlement).unwrap();
        settlement.require_auth();
        s.set(&DataKey::Frozen, &open);
    }

    /// Bind the YieldRouter (the transparent protected-share-class path). Admin, set once.
    pub fn set_router(env: Env, router: Address) {
        let s = env.storage().instance();
        let admin: Address = s.get(&DataKey::Admin).unwrap();
        admin.require_auth();
        assert!(!s.has(&DataKey::Router), "router already set");
        s.set(&DataKey::Router, &router);
        s.extend_ttl(50, 100);
    }

    /// The bound router registers its aggregate wrapped cover, enforcing the shared solvency floor
    /// (sealed + routed ≤ (1−h)·collateral). Only the bound router may call (it advances its own
    /// total as holders wrap/unwrap).
    pub fn set_routed_cover(env: Env, new_routed: i128) {
        assert!(new_routed >= 0, "routed cover must be non-negative");
        let s = env.storage().instance();
        let router: Address = s.get(&DataKey::Router).expect("router not set");
        router.require_auth();
        let total_coll = get_i128(&env, &DataKey::TotalCollateral);
        let sealed = get_i128(&env, &DataKey::TotalCover);
        let haircut: i128 = s.get(&DataKey::HaircutBps).unwrap();
        assert!((sealed + new_routed) * BPS <= total_coll * (BPS - haircut), "insolvent: routed cover would exceed (1-haircut)·collateral");
        s.set(&DataKey::RoutedCover, &new_routed);
        s.extend_ttl(50, 100);
    }

    // --- getters ---
    pub fn total_collateral(env: Env) -> i128 { get_i128(&env, &DataKey::TotalCollateral) }
    /// Effective cover = sealed (commitment path) + routed (protected-share-class path).
    pub fn total_cover(env: Env) -> i128 {
        get_i128(&env, &DataKey::TotalCover) + get_i128(&env, &DataKey::RoutedCover)
    }
    pub fn routed_cover(env: Env) -> i128 { get_i128(&env, &DataKey::RoutedCover) }
    pub fn position_root(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::PositionRoot).unwrap()
    }
    pub fn seller_balance(env: Env, seller: Address) -> i128 { get_i128(&env, &DataKey::Seller(seller)) }
    pub fn protocol_fee_accrued(env: Env) -> i128 { get_i128(&env, &DataKey::ProtocolFeeAccrued) }
    /// Premium a seller could claim right now (settled claimable + unsettled pending).
    pub fn pending_premium(env: Env, seller: Address) -> i128 {
        let coll = get_i128(&env, &DataKey::Seller(seller.clone()));
        let acc = get_i128(&env, &DataKey::AccPremiumPerColl);
        let debt = get_i128(&env, &DataKey::SellerDebt(seller.clone()));
        let claimable = get_i128(&env, &DataKey::SellerClaimable(seller));
        claimable + (coll * acc / ACC_SCALE - debt)
    }
    pub fn is_frozen(env: Env) -> bool { is_frozen(&env) }
}

mod test;
