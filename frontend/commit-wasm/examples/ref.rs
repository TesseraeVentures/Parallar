//! Native reference for the parity check: prints the guest commitment for a fixed input, so the
//! wasm output (via the browser bridge / Node) can be confirmed byte-identical.
//!   cargo run --example ref
fn main() {
    let c = settle_credit_v1::commitment(&[0x12u8; 40], 800, &[7u8; 32]);
    let hex: String = c.iter().map(|b| format!("{:02x}", b)).collect();
    println!("{hex}");
}
