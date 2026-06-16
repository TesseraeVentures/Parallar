//! The RISC Zero solvency_v1 guest (Option C confidential cover, G3): read a purchase or withdrawal
//! request, prove it preserves solvency over the HIDDEN running aggregate, and commit the journal.
//! A purchase proves `new_total = old_total + cover <= collateral` and binds the buyer's position
//! commitment; a withdrawal proves the aggregate still fits under the post-withdrawal collateral.
//! The cover and the totals never enter the journal — only commitments + the public collateral do.
//! Any rejection panics, so no proof can exist for an insolvent purchase/withdrawal.

use risc0_zkvm::guest::env;
use solvency_v1::{check, check_withdraw, SolvencyRequest};

fn main() {
    let req: SolvencyRequest = env::read();
    match req {
        SolvencyRequest::Buy(inputs) => {
            let journal = check(&inputs).expect("solvency: purchase rejected (insolvent or unbound)");
            env::commit_slice(&journal.to_bytes()); // 112 bytes
        }
        SolvencyRequest::Withdraw(inputs) => {
            let journal = check_withdraw(&inputs).expect("solvency: withdrawal rejected (insolvent)");
            env::commit_slice(&journal.to_bytes()); // 48 bytes
        }
    }
}
