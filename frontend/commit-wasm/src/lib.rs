//! WASM bridge exposing the guest's EXACT position commitment to the browser dApp.
//!
//! `parallar_commit` calls `settle_credit_v1::commitment` directly, so the browser computes
//! `H(buyer ‖ cover ‖ salt)` byte-for-byte identically to what the settlement guest reproduces
//! at settlement time. A cover position bought in the UI is therefore genuinely settleable and
//! genuinely private — "your position stays private" is honored by construction, not faked.
//!
//! Manual C-ABI exports + a bump of `parallar_alloc` so JS marshals bytes through wasm memory
//! without wasm-bindgen.

use core::slice;

// getrandom is pulled in transitively (ark-std) but the deterministic commitment never needs
// randomness. Register a no-op CUSTOM backend so the wasm carries NO external RNG import.
getrandom::register_custom_getrandom!(unreachable_rng);
fn unreachable_rng(_buf: &mut [u8]) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}

/// Allocate `n` bytes in wasm linear memory and leak them; JS writes inputs / reads the output
/// here. (A demo bridge — no free; the page reloads cheaply.)
#[no_mangle]
pub extern "C" fn parallar_alloc(n: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(n);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

/// Compute the Poseidon position commitment.
/// - `buyer_ptr`/`buyer_len`: the buyer's canonical Address XDR bytes.
/// - `cover_lo`/`cover_hi`: the i128 cover split into two u64 halves (the wasm ABI has no i128).
/// - `salt_ptr`: 32 salt bytes.
/// - `out_ptr`: receives the 32-byte commitment.
#[no_mangle]
pub extern "C" fn parallar_commit(
    buyer_ptr: *const u8,
    buyer_len: usize,
    cover_lo: u64,
    cover_hi: u64,
    salt_ptr: *const u8,
    out_ptr: *mut u8,
) {
    let buyer = unsafe { slice::from_raw_parts(buyer_ptr, buyer_len) };
    let salt = unsafe { slice::from_raw_parts(salt_ptr, 32) };
    let mut salt32 = [0u8; 32];
    salt32.copy_from_slice(salt);
    let cover = (((cover_hi as u128) << 64) | (cover_lo as u128)) as i128;

    let c = settle_credit_v1::commitment(buyer, cover, &salt32);
    unsafe { core::ptr::copy_nonoverlapping(c.as_ptr(), out_ptr, 32) };
}
