# Verify Parallar yourself

Every claim Parallar makes is checkable from public data and this repo — you do not have to
trust us. Run:

```bash
./scripts/verify.sh
```

It walks the steps below. Here is what each one proves.

## 1 · The proof artifact is self-consistent
The committed `prover/host/tests/fixtures/real_proof.json` is a **real 260-byte selector-wrapped
Groth16 proof** plus the 116-byte generic journal it attests. The script decodes the journal into
its fields (instrument_id, epoch, deadline, position_root, allocation_root, total_payout), and
checks `sha256(journal) == journal_digest` (what the settlement contract recomputes) and that the
seal carries the RISC Zero router selector `73c457ba`. It also checks the proof's `image_id`
equals the **deployed type's pinned image_id** — i.e. this proof was produced by the *same* guest
the live instruments are pinned to (the versioning law). The fixture is the reproducible **demo**
proof for its own scenario; the live on-chain settlement is a *separate* artifact (step 2).

## 2 · The live settlement happened on-chain
A real default-to-payout has executed on Stellar testnet. Open the settlement transaction on the
explorer (the script prints the link) and confirm the events: a `transfer` from the vault to the
cover buyer and a `Settled(epoch)` marker. The factory and the Groth16 verifier are linked too.

## 3 · Verify the proof on-chain yourself
```bash
cargo test -p parallar-prover-host --test onchain_verify
```
Runs the committed proof through the **actual deployed Groth16 verifier's logic** in-process (the
Nethermind RISC Zero stack: BN254 pairing + Poseidon), with no 34-minute proving step. A tampered
seal is rejected; the valid one verifies and the payout flows.

## 4 · Verification is the *only* way the reserve moves
```bash
grep -rniE 'mock[-_]?verify' contracts/settlement/   # must be empty
```
There is no mock-verify path, no admin path, no pause-and-pay. The settlement contract's sole
authorization is verifying one proof against the pinned image_id; the vault's payout entrypoint
rejects any caller that is not the bound settlement (tested). A non-default is *unprovable* — feed
the guest a fully-paid epoch and it panics, so no proof can exist.

## 5 · Reproduce the pinned image_id
```bash
cd prover && cargo build -p parallar-methods
```
`SETTLE_CREDIT_V1_GUEST_ID` must hash to the deployed image_id (`705ddac4…`). This proves the
source in this repo is exactly the guest the live instruments are pinned to. (Needs the RISC Zero
toolchain; `weather_v1` = `d31246e6…`, `credit_v2` = `d07e6aaf…`.)

## 6 · Reproduce the determination + invariants
```bash
make test                                                           # build wasm + ALL contracts
cd prover && cargo test -p settle-credit-v1 -p parallar-proptests   # rule + property tests
cd prover && cargo test -p parallar-prover-host --test scale -- --ignored --nocapture  # cycle counts
```
The property tests fuzz the published rule and assert the invariants always hold (Σ payouts ≤
collateral; a non-default/non-breach is unprovable; determinism). The scale test reports the
hardware-independent zkVM cycle counts.

## 7 · Reproduce the money-flow layer (both sides' economics)
```bash
make build  # wasm (the factory tests contractimport! it)
cargo test -p parallar-yield-vault -p parallar-yield-router -p parallar-yield-factory
```
This verifies premium collection + pro-rata underwriter distribution + the protocol base fee +
float (`yield_vault`), the coupon waterfall + pBOND (`yield_router`), and the risk-tier factory
that prices each bond into a tier and yields different net coupons (`yield_factory`). Defaults still
pay only via the proof-gated settlement; premium/float are separate pools (Law #1). The full
economics + the non-circularity covenant are in [docs/ECONOMICS.md](docs/ECONOMICS.md).

## What verification means (and does not)
A proof guarantees **correct computation over the inputs the guest was supplied** — the
version-pinned rules executed over the committed positions and the given data, producing exactly
those payouts. It does **not** by itself guarantee the input payment data is canonical; that is the
honest trust boundary ([TECH_SPEC §1](docs/TECH_SPEC.md), [SECURITY.md](SECURITY.md)). The fix is
attested feeds (gap **G1**), of which a first cut is built — `credit_v2` verifies an issuer
signature over the payment snapshot in-guest. Within the proof boundary, no party, including us,
can inflate, favor, omit, or fabricate a payout.
