#![no_std]
//! ClaimableSettlement — a v-next settlement variant (PRODUCTION_GAP G2 escape hatch).
//!
//! A NEW instrument-family version, deployed alongside (never replacing) the frozen generic
//! settlement. It keeps the full keeper path `settle` AND adds `claim_direct`: after a
//! keeper-grace window, ANY single buyer can be paid against a single-allocation proof — so no
//! keeper can withhold a payout. Two verification keys: the SETTLE guest's image_id (full set)
//! and the CLAIM guest's image_id (one buyer). Both paths are gated solely by a verified Groth16
//! proof (Law #1); there is no admin/pause path.
//!
//! Double-pay is impossible by construction: `settle` and `claim_direct` are MUTUALLY EXCLUSIVE
//! per epoch (a full settle blocks later claims; any claim blocks a later full settle), each
//! claimant can claim at most once, and the vault's Σ payouts ≤ collateral check bounds the rest.

use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, xdr::ToXdr, Address,
    Bytes, BytesN, Env, Vec,
};

#[contractclient(name = "VaultClient")]
pub trait VaultInterface {
    fn position_root(env: Env) -> BytesN<32>;
    fn pay_allocations(env: Env, epoch: u32, allocations: Vec<(Address, i128)>);
}

#[contractclient(name = "VerifierRouterClient")]
pub trait VerifierRouterInterface {
    fn verify(env: Env, seal: Bytes, image_id: BytesN<32>, journal: BytesN<32>);
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    SettleImageId,
    ClaimImageId,
    InstrumentId,
    Vault,
    Verifier,
    Grace,
    Deadline(u32),
    Settled(u32),
    HasClaims(u32),
    Claimed(u32, Address),
}

#[contractevent(data_format = "single-value")]
pub struct Settled {
    #[topic]
    pub epoch: u32,
    pub total_payout: u64,
}

#[contractevent(data_format = "single-value")]
pub struct Claimed {
    #[topic]
    pub epoch: u32,
    pub amount: i128,
}

#[contract]
pub struct ClaimableSettlement;

const JOURNAL_LEN: u32 = 116;

fn read_u32_be(j: &Bytes, off: u32) -> u32 {
    let mut v: u32 = 0;
    for i in 0..4u32 {
        v = (v << 8) | (j.get(off + i).unwrap() as u32);
    }
    v
}
fn read_u64_be(j: &Bytes, off: u32) -> u64 {
    let mut v: u64 = 0;
    for i in 0..8u32 {
        v = (v << 8) | (j.get(off + i).unwrap() as u64);
    }
    v
}
fn read_b32(env: &Env, j: &Bytes, off: u32) -> BytesN<32> {
    let mut a = [0u8; 32];
    for i in 0..32u32 {
        a[i as usize] = j.get(off + i).unwrap();
    }
    BytesN::from_array(env, &a)
}

/// allocation_root = sha256 fold over (addr.to_xdr ‖ amount_be) — identical to the generic
/// settlement, so the SAME guest/host `allocation_root` encoding applies.
pub fn hash_allocations(env: &Env, allocations: &Vec<(Address, i128)>) -> BytesN<32> {
    let mut acc = BytesN::from_array(env, &[0u8; 32]);
    for pair in allocations.iter() {
        let (addr, amt) = pair;
        let mut buf = Bytes::new(env);
        buf.append(&Bytes::from_array(env, &acc.to_array()));
        buf.append(&addr.to_xdr(env));
        buf.append(&Bytes::from_array(env, &amt.to_be_bytes()));
        acc = env.crypto().sha256(&buf).to_bytes();
    }
    acc
}

/// Decode + verify everything common to both paths: the proof (against `image_id`), the journal
/// length + bindings (instrument, deadline). Returns (epoch, deadline, position_root,
/// allocation_root, total_payout). Traps on any failure — that trap is the gate.
fn verify_and_decode(env: &Env, image_id: &BytesN<32>, proof: &Bytes, journal: &Bytes) -> (u32, u64, BytesN<32>, BytesN<32>, u64) {
    let s = env.storage().instance();
    let verifier: Address = s.get(&DataKey::Verifier).unwrap();
    let journal_digest = env.crypto().sha256(journal).to_bytes();
    VerifierRouterClient::new(env, &verifier).verify(proof, image_id, &journal_digest);

    assert!(journal.len() == JOURNAL_LEN, "bad journal length");
    let j_instrument = read_b32(env, journal, 0);
    let epoch = read_u32_be(journal, 32);
    let deadline = read_u64_be(journal, 36);
    let j_position_root = read_b32(env, journal, 44);
    let j_allocation_root = read_b32(env, journal, 76);
    let total_payout = read_u64_be(journal, 108);

    let instrument_id: BytesN<32> = s.get(&DataKey::InstrumentId).unwrap();
    assert!(j_instrument == instrument_id, "instrument_id mismatch");
    let stored_deadline: u64 = s.get(&DataKey::Deadline(epoch)).expect("unknown epoch");
    assert!(deadline == stored_deadline, "journal deadline != instrument deadline");

    let vault_addr: Address = s.get(&DataKey::Vault).unwrap();
    assert!(j_position_root == VaultClient::new(env, &vault_addr).position_root(), "stale position_root");
    (epoch, stored_deadline, j_position_root, j_allocation_root, total_payout)
}

#[contractimpl]
impl ClaimableSettlement {
    /// Cross-binding set at deploy. `settle_image_id` gates the full keeper path; `claim_image_id`
    /// gates the single-buyer escape hatch; `grace` is the seconds after the deadline that the
    /// keeper has exclusively before direct claims open.
    pub fn init(
        env: Env,
        settle_image_id: BytesN<32>,
        claim_image_id: BytesN<32>,
        instrument_id: BytesN<32>,
        vault: Address,
        deadlines: Vec<(u32, u64)>,
        verifier_router: Address,
        grace: u64,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::SettleImageId) {
            panic!("already initialized");
        }
        s.set(&DataKey::SettleImageId, &settle_image_id);
        s.set(&DataKey::ClaimImageId, &claim_image_id);
        s.set(&DataKey::InstrumentId, &instrument_id);
        s.set(&DataKey::Vault, &vault);
        s.set(&DataKey::Verifier, &verifier_router);
        s.set(&DataKey::Grace, &grace);
        for pair in deadlines.iter() {
            let (epoch, deadline) = pair;
            s.set(&DataKey::Deadline(epoch), &deadline);
        }
        s.extend_ttl(50, 100);
    }

    /// Full keeper settlement — verifies against the SETTLE image_id and pays the whole set.
    /// Permissionless. Blocked once any direct claim has been made for the epoch (mutual exclusion).
    pub fn settle(env: Env, proof: Bytes, journal: Bytes, allocations: Vec<(Address, i128)>) {
        let s = env.storage().instance();
        let image_id: BytesN<32> = s.get(&DataKey::SettleImageId).unwrap();
        let (epoch, deadline, _proot, j_allocation_root, total_payout) =
            verify_and_decode(&env, &image_id, &proof, &journal);

        assert!(!s.get(&DataKey::Settled(epoch)).unwrap_or(false), "epoch already settled");
        assert!(!s.get(&DataKey::HasClaims(epoch)).unwrap_or(false), "epoch has direct claims; settle per-claimant");
        assert!(env.ledger().timestamp() > deadline, "deadline not passed");
        assert!(j_allocation_root == hash_allocations(&env, &allocations), "allocation_root mismatch");

        s.set(&DataKey::Settled(epoch), &true);
        let vault_addr: Address = s.get(&DataKey::Vault).unwrap();
        VaultClient::new(&env, &vault_addr).pay_allocations(&epoch, &allocations);
        s.extend_ttl(50, 100);
        Settled { epoch, total_payout }.publish(&env);
    }

    /// Escape hatch — after the keeper-grace window, pay a SINGLE buyer against a single-allocation
    /// proof (verified against the CLAIM image_id). Per-claimant dedup; blocked once fully settled;
    /// blocks a later full settle. The vault's Σ ≤ collateral check bounds cumulative claims.
    pub fn claim_direct(env: Env, proof: Bytes, journal: Bytes, claimant: Address, amount: i128) {
        let s = env.storage().instance();
        let image_id: BytesN<32> = s.get(&DataKey::ClaimImageId).unwrap();
        let (epoch, deadline, _proot, j_allocation_root, total_payout) =
            verify_and_decode(&env, &image_id, &proof, &journal);

        assert!(!s.get(&DataKey::Settled(epoch)).unwrap_or(false), "epoch already fully settled");
        let grace: u64 = s.get(&DataKey::Grace).unwrap();
        assert!(env.ledger().timestamp() > deadline + grace, "keeper-grace window not passed");
        assert!(!s.get(&DataKey::Claimed(epoch, claimant.clone())).unwrap_or(false), "already claimed");

        // the proof must attest exactly this one allocation: (claimant, amount), total == amount
        assert!(amount > 0, "claim amount must be positive");
        assert!(total_payout == amount as u64, "journal total != claim amount");
        let mut one = Vec::new(&env);
        one.push_back((claimant.clone(), amount));
        assert!(j_allocation_root == hash_allocations(&env, &one), "allocation_root != single claim");

        s.set(&DataKey::Claimed(epoch, claimant.clone()), &true);
        s.set(&DataKey::HasClaims(epoch), &true);
        let vault_addr: Address = s.get(&DataKey::Vault).unwrap();
        VaultClient::new(&env, &vault_addr).pay_allocations(&epoch, &one); // vault bounds Σ ≤ collateral
        s.extend_ttl(50, 100);
        Claimed { epoch, amount }.publish(&env);
    }

    pub fn settle_image_id(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::SettleImageId).unwrap()
    }
    pub fn claim_image_id(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::ClaimImageId).unwrap()
    }
    pub fn is_settled(env: Env, epoch: u32) -> bool {
        env.storage().instance().get(&DataKey::Settled(epoch)).unwrap_or(false)
    }
    pub fn has_claimed(env: Env, epoch: u32, who: Address) -> bool {
        env.storage().instance().get(&DataKey::Claimed(epoch, who)).unwrap_or(false)
    }
    pub fn grace(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::Grace).unwrap()
    }
}

mod test;
