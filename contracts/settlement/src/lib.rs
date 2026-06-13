#![no_std]
//! SettlementContract — generic, identical WASM for every instrument type (TECH_SPEC §3.3).
//!
//! Verifies ONE proof against the type's pinned `image_id`, checks the generic journal's
//! bindings, and executes allocations through the bound vault. What is deliberately
//! ABSENT: no admin path, no pause-and-pay, no alternative authorization. Verifying the
//! proof + bindings is the sole path to a payout. Settlement is permissionless (any keeper).

use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, xdr::ToXdr, Address,
    Bytes, BytesN, Env, Vec,
};

/// Minimal cross-contract client for the bound vault.
#[contractclient(name = "VaultClient")]
pub trait VaultInterface {
    fn position_root(env: Env) -> BytesN<32>;
    fn pay_allocations(env: Env, epoch: u32, allocations: Vec<(Address, i128)>);
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    ImageId,
    InstrumentId,
    Vault,
    Deadline(u32),
    Settled(u32),
}

/// Emitted on a successful settlement (ledger history).
#[contractevent(data_format = "single-value")]
pub struct Settled {
    #[topic]
    pub epoch: u32,
    pub total_payout: u64,
}

#[contract]
pub struct SettlementContract;

// --- generic journal (§3.4): 116 bytes fixed-width, big-endian scalars ---
// instrument_id(32) | epoch(4) | deadline(8) | position_root(32) | allocation_root(32) | total_payout(8)
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

/// allocation_root = sha256 fold over (addr.to_xdr ‖ amount_be) per allocation.
/// The settlement guest reproduces this exact encoding (alignment finalized in Sprint 2).
fn hash_allocations(env: &Env, allocations: &Vec<(Address, i128)>) -> BytesN<32> {
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

#[cfg(feature = "mock-verify")]
fn verify_proof(_env: &Env, _proof: &Bytes, _journal: &Bytes, _image_id: &BytesN<32>) {
    // STUB — Sprint 1 only (CLAUDE.md stub discipline). Never demo/record on this.
}
#[cfg(not(feature = "mock-verify"))]
fn verify_proof(_env: &Env, _proof: &Bytes, _journal: &Bytes, _image_id: &BytesN<32>) {
    unimplemented!("real Groth16 verification is wired June 22 (Sprint 2)");
}

#[contractimpl]
impl SettlementContract {
    /// Cross-binding set at factory deploy. `image_id` is immutable for this instance.
    pub fn init(
        env: Env,
        image_id: BytesN<32>,
        instrument_id: BytesN<32>,
        vault: Address,
        deadlines: Vec<(u32, u64)>,
    ) {
        let s = env.storage().instance();
        if s.has(&DataKey::ImageId) {
            panic!("already initialized");
        }
        s.set(&DataKey::ImageId, &image_id);
        s.set(&DataKey::InstrumentId, &instrument_id);
        s.set(&DataKey::Vault, &vault);
        for pair in deadlines.iter() {
            let (epoch, deadline) = pair;
            s.set(&DataKey::Deadline(epoch), &deadline);
        }
        s.extend_ttl(50, 100);
    }

    /// Permissionless settlement. Verifies the proof + every binding, then pays out
    /// through the vault. One settlement per epoch.
    pub fn settle(env: Env, proof: Bytes, journal: Bytes, allocations: Vec<(Address, i128)>) {
        let s = env.storage().instance();
        let image_id: BytesN<32> = s.get(&DataKey::ImageId).unwrap();

        // 1. verify proof against the pinned image_id (stub under mock-verify)
        verify_proof(&env, &proof, &journal, &image_id);

        // 2. decode the generic journal
        assert!(journal.len() == JOURNAL_LEN, "bad journal length");
        let j_instrument = read_b32(&env, &journal, 0);
        let epoch = read_u32_be(&journal, 32);
        let deadline = read_u64_be(&journal, 36);
        let j_position_root = read_b32(&env, &journal, 44);
        let j_allocation_root = read_b32(&env, &journal, 76);
        let total_payout = read_u64_be(&journal, 108);

        // 3. bindings
        let instrument_id: BytesN<32> = s.get(&DataKey::InstrumentId).unwrap();
        assert!(j_instrument == instrument_id, "instrument_id mismatch");
        assert!(
            !s.get(&DataKey::Settled(epoch)).unwrap_or(false),
            "epoch already settled"
        );
        let stored_deadline: u64 = s.get(&DataKey::Deadline(epoch)).expect("unknown epoch");
        assert!(deadline == stored_deadline, "journal deadline != instrument deadline");
        assert!(env.ledger().timestamp() > stored_deadline, "deadline not passed");

        let vault_addr: Address = s.get(&DataKey::Vault).unwrap();
        let vault = VaultClient::new(&env, &vault_addr);
        assert!(j_position_root == vault.position_root(), "stale position_root");
        assert!(
            j_allocation_root == hash_allocations(&env, &allocations),
            "allocation_root mismatch"
        );

        // 4. mark settled + execute payouts via the vault (the sole payout path)
        s.set(&DataKey::Settled(epoch), &true);
        vault.pay_allocations(&epoch, &allocations);
        s.extend_ttl(50, 100);
        Settled { epoch, total_payout }.publish(&env);
    }

    pub fn image_id(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::ImageId).unwrap()
    }
    pub fn instrument_id(env: Env) -> BytesN<32> {
        env.storage().instance().get(&DataKey::InstrumentId).unwrap()
    }
    pub fn vault(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Vault).unwrap()
    }
    pub fn is_settled(env: Env, epoch: u32) -> bool {
        env.storage().instance().get(&DataKey::Settled(epoch)).unwrap_or(false)
    }
    pub fn deadline(env: Env, epoch: u32) -> u64 {
        env.storage().instance().get(&DataKey::Deadline(epoch)).unwrap()
    }
}

mod test;
