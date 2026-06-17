//! Confidential-cover keeper / sequencer — the PRODUCTION_GAP G3 coordination layer that makes
//! `confidential_vault` usable in practice.
//!
//! The confidential vault keeps the running cover total as a Poseidon COMMITMENT, not a number.
//! Every confidential buy/withdraw carries a `solvency_v1` proof against the CURRENT aggregate
//! opening `(total, salt)` — so *someone* must hold that opening, order purchases (the opening is
//! shared mutable state — two buys can't both prove against the same `prev`), and advance it. This
//! is that someone: a **single-writer sequencer over one instrument's hidden aggregate**.
//!
//! Trust model: the keeper is a trusted OPERATOR role. It learns cover deltas to maintain the
//! aggregate — private to the *market / chain*, not to the operator. That is the documented G3
//! coordination model; a buyer-builds-the-proof variant (keeper supplies only the opening) is a
//! later refinement. Per the Product Vision: "private to the market, verifiable to the regulator."
//!
//! This module is the PURE construction + state machine (native-testable, no zkVM). The actual
//! Groth16 proof is produced by `prove_solvency_buy` / `prove_solvency_withdraw` on the returned
//! inputs (x86 / Rosetta); the CLI wires the two together over a persisted state file.

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// The keeper's persisted state for ONE confidential instrument: the opening of the vault's stored
/// cover commitment. INVARIANT: `commit_total(total, salt)` equals the vault's on-chain
/// `CoverCommitment`. A fresh instrument starts at `genesis(salt0)` where the vault was init'd with
/// `initial_cover_commitment = commit_total(0, salt0)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperState {
    /// the hidden running aggregate cover the commitment opens to
    pub total: i128,
    /// the opening salt of the current commitment
    pub salt: [u8; 32],
    /// premium rate (basis points) — for computing the buyer's upfront premium
    pub premium_bps: u32,
}

impl KeeperState {
    /// Genesis state matching a vault deployed with `initial_cover_commitment = commit_total(0, salt0)`.
    pub fn genesis(salt0: [u8; 32], premium_bps: u32) -> Self {
        Self { total: 0, salt: salt0, premium_bps }
    }

    /// The commitment the vault currently stores — must match the on-chain `CoverCommitment`.
    pub fn commitment(&self) -> [u8; 32] {
        solvency_v1::commit_total(self.total, &self.salt)
    }
}

/// The result of planning a confidential BUY: the solvency inputs to prove, the advanced keeper
/// state to persist on success, and the opening the BUYER must save `(cover, position_salt)` to
/// settle later.
#[derive(Clone, Debug)]
pub struct BuyPlan {
    pub inputs: solvency_v1::SolvencyInputs,
    pub next: KeeperState,
    pub position_salt: [u8; 32],
    pub premium: i128,
}

/// Plan a confidential purchase: advance the hidden aggregate by `cover` and bind the buyer's
/// position. Pure + native-testable — the caller proves `plan.inputs` with `prove_solvency_buy`
/// (x86) to get the seal+journal the buyer submits to `buy_protection_proven`, then persists
/// `plan.next` ONLY on a successful on-chain submission.
///
/// `new_agg_salt` advances the aggregate commitment; `position_salt` is the buyer's own opening.
/// Both are caller-supplied so the function stays deterministic (the CLI draws them from the OS RNG).
pub fn plan_buy(
    state: &KeeperState,
    buyer_xdr: Vec<u8>,
    cover: i128,
    collateral: i128,
    new_agg_salt: [u8; 32],
    position_salt: [u8; 32],
) -> anyhow::Result<BuyPlan> {
    anyhow::ensure!(cover > 0, "cover must be positive");
    anyhow::ensure!(collateral >= 0, "collateral must be non-negative");
    let new_total = state.total.checked_add(cover).context("cover would overflow the aggregate")?;
    anyhow::ensure!(
        new_total <= collateral,
        "insolvent: new aggregate {new_total} would exceed collateral {collateral} (deposit more first)"
    );
    let inputs = solvency_v1::SolvencyInputs {
        collateral,
        prev_cover_commitment: state.commitment(),
        new_cover_commitment: solvency_v1::commit_total(new_total, &new_agg_salt),
        position_commitment: settle_credit_v1::commitment(&buyer_xdr, cover, &position_salt),
        old_total: state.total,
        old_salt: state.salt,
        new_salt: new_agg_salt,
        cover,
        buyer: buyer_xdr,
        salt: position_salt,
    };
    let premium = cover.saturating_mul(state.premium_bps as i128) / 10_000;
    Ok(BuyPlan {
        inputs,
        next: KeeperState { total: new_total, salt: new_agg_salt, premium_bps: state.premium_bps },
        position_salt,
        premium,
    })
}

/// Plan a confidential WITHDRAW: prove the (unchanged) hidden aggregate still fits under the
/// post-withdrawal collateral. The keeper state does NOT change — a withdrawal shrinks collateral,
/// not cover. `collateral_after = vault.total_collateral - amount`.
pub fn plan_withdraw(
    state: &KeeperState,
    collateral_after: i128,
) -> anyhow::Result<solvency_v1::WithdrawInputs> {
    anyhow::ensure!(collateral_after >= 0, "collateral_after must be non-negative");
    anyhow::ensure!(
        state.total <= collateral_after,
        "withdrawal would break solvency: hidden aggregate {} exceeds collateral_after {collateral_after}",
        state.total
    );
    Ok(solvency_v1::WithdrawInputs {
        collateral_after,
        cover_commitment: state.commitment(),
        total: state.total,
        salt: state.salt,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    fn buyer() -> Vec<u8> {
        vec![0x12u8; 40]
    }

    #[test]
    fn buy_advances_aggregate_and_proves_solvent() {
        let st = KeeperState::genesis([1u8; 32], 200);
        let plan = plan_buy(&st, buyer(), 600, 1000, [2u8; 32], [7u8; 32]).unwrap();
        // the construction is accepted by the native guest check (so a real proof will exist)
        let journal = solvency_v1::check(&plan.inputs).expect("solvent purchase must pass the guest check");
        // the advanced state's commitment is exactly the new commitment the vault will store
        assert_eq!(journal.new_cover_commitment, plan.next.commitment());
        assert_eq!(plan.next.total, 600);
        // prev links to the genesis commitment the vault was init'd with
        assert_eq!(journal.prev_cover_commitment, st.commitment());
        // premium = cover × bps / 10_000
        assert_eq!(plan.premium, 12);
    }

    #[test]
    fn second_buy_chains_off_the_first() {
        let st = KeeperState::genesis([1u8; 32], 0);
        let p1 = plan_buy(&st, buyer(), 400, 1000, [2u8; 32], [8u8; 32]).unwrap();
        solvency_v1::check(&p1.inputs).unwrap();
        let p2 = plan_buy(&p1.next, buyer(), 300, 1000, [3u8; 32], [9u8; 32]).unwrap();
        let j2 = solvency_v1::check(&p2.inputs).expect("chained purchase must pass");
        // p2 proves against p1's advanced commitment — the sequencer invariant
        assert_eq!(j2.prev_cover_commitment, p1.next.commitment());
        assert_eq!(p2.next.total, 700);
    }

    #[test]
    fn insolvent_buy_is_rejected_before_proving() {
        let st = KeeperState::genesis([1u8; 32], 200);
        // cover 1200 > collateral 1000 — the keeper refuses to even build the inputs
        let err = plan_buy(&st, buyer(), 1200, 1000, [2u8; 32], [7u8; 32]).unwrap_err();
        assert!(err.to_string().contains("insolvent"), "got: {err}");
    }

    #[test]
    fn withdraw_proves_against_unchanged_aggregate() {
        let st = KeeperState::genesis([1u8; 32], 200);
        let bought = plan_buy(&st, buyer(), 600, 1000, [2u8; 32], [7u8; 32]).unwrap().next;
        // collateral 1000 → withdraw 200 → collateral_after 800; aggregate 600 ≤ 800 ✓
        let w = plan_withdraw(&bought, 800).unwrap();
        let wj = solvency_v1::check_withdraw(&w).expect("solvent withdrawal must pass");
        assert_eq!(wj.cover_commitment, bought.commitment());
        // withdrawing down to 500 would strand the 600 aggregate — rejected
        assert!(plan_withdraw(&bought, 500).is_err());
    }
}
