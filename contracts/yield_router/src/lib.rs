#![no_std]
//! YieldRouter — the protected share class (TECH_SPEC §5A, PRODUCTION_GAP G11).
//!
//! Upstream of the frozen core; the vault/settlement/journal/guests are untouched. A holder
//! `wrap`s a bond to mint pBOND (cover = the wrapped balance, registered with the vault for the
//! shared solvency floor). `route_coupon` runs the §5A waterfall on the gross coupon received for
//! the wrapped bonds:
//!   premium = wrapped × premium_bps;
//!   distribution_fee = premium × dist_fee_bps   (the router's MGA-equivalent take — §5A (b));
//!   to the vault = premium − distribution_fee   (→ underwriters pro-rata + the vault's base fee);
//!   net = gross − premium                       (→ pBOND holders, pro-rata to pBOND).
//! `unwrap` burns pBOND and returns the bond (cover lapses). pBOND is a transferable receipt so it
//! can serve as enhanced collateral in external lending markets (G13). Settlement is unchanged:
//! on a default the standard certificate pays wrapped holders from the reserve (Law #1 holds).
//!
//! Premium-in-arrears (§5A tension 2): premium is deducted from each coupon; a defaulted epoch
//! routes no premium (settlement pays the holders instead). The one-epoch-escrow alternative is
//! documented in PRODUCTION_GAP G11. Visibility (tension 1): pBOND cover is public by design —
//! this is the transparent product line; the sealed line stays on the vault's commitment path.

use soroban_sdk::{contract, contractclient, contractevent, contractimpl, contracttype, token, Address, Env};

const ACC_SCALE: i128 = 1_000_000_000_000;
const BPS: i128 = 10_000;

/// The bound vault: the router registers cover for solvency and forwards premium to it.
#[contractclient(name = "VaultClient")]
pub trait VaultInterface {
    fn set_routed_cover(env: Env, new_routed: i128);
    fn receive_premium(env: Env, amount: i128);
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    BondToken,
    CouponToken,
    Vault,
    Admin,
    PremiumBps,
    DistFeeBps,
    TotalSupply,            // total pBOND = routed cover
    AccNetPerPbond,         // net-coupon-per-pbond accumulator (scaled)
    DistFeeAccrued,
    Balance(Address),       // pBOND balance
    Debt(Address),          // acc snapshot at last settle (scaled)
    Claimable(Address),     // settled, unclaimed net coupon
}

#[contractevent(data_format = "map")]
pub struct CouponRouted {
    #[topic]
    pub epoch: u32,
    pub to_vault: i128,
    pub to_holders: i128,
    pub dist_fee: i128,
}

#[contract]
pub struct YieldRouter;

fn g(env: &Env, k: &DataKey) -> i128 {
    env.storage().instance().get(k).unwrap_or(0)
}

/// Settle a holder's accrued net coupon into their claimable bucket; call before any pBOND balance
/// change (wrap/unwrap/transfer) or a claim.
fn settle_holder(env: &Env, who: &Address) {
    let s = env.storage().instance();
    let bal = g(env, &DataKey::Balance(who.clone()));
    let acc = g(env, &DataKey::AccNetPerPbond);
    let debt = g(env, &DataKey::Debt(who.clone()));
    let pending = bal * acc / ACC_SCALE - debt;
    if pending > 0 {
        let c = g(env, &DataKey::Claimable(who.clone()));
        s.set(&DataKey::Claimable(who.clone()), &(c + pending));
    }
    s.set(&DataKey::Debt(who.clone()), &(bal * acc / ACC_SCALE));
}

#[contractimpl]
impl YieldRouter {
    pub fn init(
        env: Env,
        bond_token: Address,
        coupon_token: Address,
        vault: Address,
        admin: Address,
        premium_bps: u32,
        dist_fee_bps: u32,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::Vault) {
            panic!("already initialized");
        }
        assert!(dist_fee_bps <= 10_000, "bad bps");
        s.set(&DataKey::BondToken, &bond_token);
        s.set(&DataKey::CouponToken, &coupon_token);
        s.set(&DataKey::Vault, &vault);
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::PremiumBps, &(premium_bps as i128));
        s.set(&DataKey::DistFeeBps, &(dist_fee_bps as i128));
        s.set(&DataKey::TotalSupply, &0i128);
        s.set(&DataKey::AccNetPerPbond, &0i128);
        s.set(&DataKey::DistFeeAccrued, &0i128);
        s.extend_ttl(50, 100);
    }

    /// Wrap `amount` of the bond → mint pBOND 1:1; cover auto-sizes to the wrapped balance and is
    /// registered with the vault (which enforces the shared solvency floor — reverts if it would
    /// breach it, so wrapping past the reserve is impossible).
    pub fn wrap(env: Env, holder: Address, amount: i128) {
        holder.require_auth();
        assert!(amount > 0, "amount must be positive");
        let s = env.storage().instance();
        settle_holder(&env, &holder);
        let bond: Address = s.get(&DataKey::BondToken).unwrap();
        token::Client::new(&env, &bond).transfer(&holder, &env.current_contract_address(), &amount);

        let supply = g(&env, &DataKey::TotalSupply) + amount;
        let bal = g(&env, &DataKey::Balance(holder.clone())) + amount;
        s.set(&DataKey::TotalSupply, &supply);
        s.set(&DataKey::Balance(holder.clone()), &bal);
        let acc = g(&env, &DataKey::AccNetPerPbond);
        s.set(&DataKey::Debt(holder), &(bal * acc / ACC_SCALE));

        let vault: Address = s.get(&DataKey::Vault).unwrap();
        VaultClient::new(&env, &vault).set_routed_cover(&supply); // solvency floor enforced here
        s.extend_ttl(50, 100);
    }

    /// Burn `amount` pBOND, return the bond; cover lapses (the router lowers its routed total).
    pub fn unwrap(env: Env, holder: Address, amount: i128) {
        holder.require_auth();
        assert!(amount > 0, "amount must be positive");
        let s = env.storage().instance();
        settle_holder(&env, &holder);
        let bal = g(&env, &DataKey::Balance(holder.clone()));
        assert!(bal >= amount, "insufficient pBOND");
        let supply = g(&env, &DataKey::TotalSupply) - amount;
        let new_bal = bal - amount;
        s.set(&DataKey::TotalSupply, &supply);
        s.set(&DataKey::Balance(holder.clone()), &new_bal);
        let acc = g(&env, &DataKey::AccNetPerPbond);
        s.set(&DataKey::Debt(holder.clone()), &(new_bal * acc / ACC_SCALE));

        let bond: Address = s.get(&DataKey::BondToken).unwrap();
        token::Client::new(&env, &bond).transfer(&env.current_contract_address(), &holder, &amount);
        let vault: Address = s.get(&DataKey::Vault).unwrap();
        VaultClient::new(&env, &vault).set_routed_cover(&supply);
        s.extend_ttl(50, 100);
    }

    /// Route an epoch's gross coupon (paid by `coupon_payer` to the router as the escrowed
    /// bondholder): premium → the vault (minus the router's distribution fee), NET → pBOND holders.
    pub fn route_coupon(env: Env, coupon_payer: Address, epoch: u32, gross: i128) {
        coupon_payer.require_auth();
        assert!(gross > 0, "gross must be positive");
        let s = env.storage().instance();
        let supply = g(&env, &DataKey::TotalSupply);
        assert!(supply > 0, "nothing wrapped");

        let coupon: Address = s.get(&DataKey::CouponToken).unwrap();
        token::Client::new(&env, &coupon).transfer(&coupon_payer, &env.current_contract_address(), &gross);

        let premium_bps: i128 = s.get(&DataKey::PremiumBps).unwrap();
        let premium = supply * premium_bps / BPS;
        let premium = if premium > gross { gross } else { premium }; // short coupon: route what arrived
        let dist_bps: i128 = s.get(&DataKey::DistFeeBps).unwrap();
        let dist_fee = premium * dist_bps / BPS;
        let to_vault = premium - dist_fee;
        let net = gross - premium;

        if to_vault > 0 {
            let vault: Address = s.get(&DataKey::Vault).unwrap();
            // transfer the premium to the vault FROM the router (implicit current-contract auth),
            // then have the vault account + distribute it (router-only).
            token::Client::new(&env, &coupon).transfer(&env.current_contract_address(), &vault, &to_vault);
            VaultClient::new(&env, &vault).receive_premium(&to_vault);
        }
        if dist_fee > 0 {
            s.set(&DataKey::DistFeeAccrued, &(g(&env, &DataKey::DistFeeAccrued) + dist_fee));
        }
        if net > 0 {
            let acc = g(&env, &DataKey::AccNetPerPbond);
            s.set(&DataKey::AccNetPerPbond, &(acc + net * ACC_SCALE / supply));
        }
        s.extend_ttl(50, 100);
        CouponRouted { epoch, to_vault, to_holders: net, dist_fee }.publish(&env);
    }

    /// pBOND holder claims their accrued net coupon (in the coupon asset).
    pub fn claim_coupon(env: Env, holder: Address) {
        holder.require_auth();
        settle_holder(&env, &holder);
        let s = env.storage().instance();
        let claimable = g(&env, &DataKey::Claimable(holder.clone()));
        assert!(claimable > 0, "nothing to claim");
        s.set(&DataKey::Claimable(holder.clone()), &0i128);
        let coupon: Address = s.get(&DataKey::CouponToken).unwrap();
        token::Client::new(&env, &coupon).transfer(&env.current_contract_address(), &holder, &claimable);
        s.extend_ttl(50, 100);
    }

    /// Protocol claims the router's accrued distribution fee.
    pub fn claim_dist_fee(env: Env) {
        let s = env.storage().instance();
        let admin: Address = s.get(&DataKey::Admin).unwrap();
        admin.require_auth();
        let accrued = g(&env, &DataKey::DistFeeAccrued);
        assert!(accrued > 0, "nothing to claim");
        s.set(&DataKey::DistFeeAccrued, &0i128);
        let coupon: Address = s.get(&DataKey::CouponToken).unwrap();
        token::Client::new(&env, &coupon).transfer(&env.current_contract_address(), &admin, &accrued);
        s.extend_ttl(50, 100);
    }

    /// Transfer pBOND (composability — pBOND as collateral in external lending, G13). Settles both
    /// parties' accrued coupons first so the move doesn't distort accrual.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0, "amount must be positive");
        let s = env.storage().instance();
        settle_holder(&env, &from);
        settle_holder(&env, &to);
        let fb = g(&env, &DataKey::Balance(from.clone()));
        assert!(fb >= amount, "insufficient pBOND");
        let acc = g(&env, &DataKey::AccNetPerPbond);
        let nfb = fb - amount;
        let ntb = g(&env, &DataKey::Balance(to.clone())) + amount;
        s.set(&DataKey::Balance(from.clone()), &nfb);
        s.set(&DataKey::Balance(to.clone()), &ntb);
        s.set(&DataKey::Debt(from), &(nfb * acc / ACC_SCALE));
        s.set(&DataKey::Debt(to), &(ntb * acc / ACC_SCALE));
        s.extend_ttl(50, 100);
    }

    // --- getters ---
    pub fn pbond_balance(env: Env, who: Address) -> i128 { g(&env, &DataKey::Balance(who)) }
    pub fn total_supply(env: Env) -> i128 { g(&env, &DataKey::TotalSupply) }
    pub fn dist_fee_accrued(env: Env) -> i128 { g(&env, &DataKey::DistFeeAccrued) }
    pub fn pending_coupon(env: Env, who: Address) -> i128 {
        let bal = g(&env, &DataKey::Balance(who.clone()));
        let acc = g(&env, &DataKey::AccNetPerPbond);
        let debt = g(&env, &DataKey::Debt(who.clone()));
        g(&env, &DataKey::Claimable(who)) + (bal * acc / ACC_SCALE - debt)
    }
}

mod test;
