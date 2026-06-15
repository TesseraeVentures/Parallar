# Security & threat model — Parallar

This is a **Stellar testnet hackathon build**. No real funds, no client paper, no production
deployment. It states its trust boundary plainly rather than overclaiming; the canonical trust
statement is [TECH_SPEC §1](docs/TECH_SPEC.md), and the path from this build to a pilot is
[PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md).

## The trust boundary (read this first)

A settlement proof guarantees that **the published, version-pinned rules were executed
correctly over the inputs the guest was supplied, and the result is exactly the committed
payouts** — verifiable execution over committed positions. It does **not** guarantee that the
input data is the canonical truth of the world: it is *correct computation on inputs, not fair
inputs*.

The one honest assumption today is the integrity of the **payment snapshot** fed to the guest.
Mitigations in this build: permissionless keeping (anyone can settle) and independent on-chain
deadline enforcement. The real fix is **attested data feeds** — gap **G1** in PRODUCTION_GAP.
Within the proof boundary, no party — including the authors — can inflate, favor, omit, or
fabricate a payout.

## The two architectural laws

1. **ZK is structurally necessary.** No code path lets the chain or any admin compute or
   authorize a payout from public state. There is no payments getter on the bond, no plaintext
   covers, no settlement override, no pause-and-pay, and no admin path to the reserve. The
   settlement contract's *sole* authorization is verifying one Groth16 proof against the
   instrument's pinned `image_id`. (CI enforces that no `mock-verify` path exists.)
2. **The production surfaces are final.** Contract separation (factory / bond / vault /
   settlement), the 116-byte generic journal, the registry interface, and the guest plug-in
   contract do not bend. A second instrument type (`weather_v1`) settles through the *unchanged*
   vault and settlement WASM — evidence the boundary holds.

## Invariants (tested; see the suites)

- Payouts move **only** via a verified settlement (`pay_allocations` requires the bound
  settlement's auth — tested against an unauthorized caller).
- One settlement per epoch; replay, settling before the deadline, stale `position_root`, and a
  tampered `allocation_root` all revert.
- A non-default is **unprovable**: if the trigger did not occur the guest panics, so no proof
  exists (property-tested over randomized books).
- Σ payouts ≤ collateral, and each payout ≤ its cover (property-tested).
- The factory rejects ineligible collateral via a curated **on-chain allowlist** (clawback/
  freezable assets are kept off it). A Soroban contract cannot introspect a classic asset's
  `AUTH_CLAWBACK_ENABLED` flag on-chain, so eligibility is verified off-chain at listing under the
  registry multisig, with an off-chain monitor that de-lists on any issuer-flag change (G6).
- A guest is pinned forever: a new rule is a **new type with a new `image_id`**, never an
  in-place upgrade. `credit_v1` = `705ddac4…`; `weather_v1` = `d31246e6…`.

## What is mocked

See the real-vs-mocked table in [README.md](README.md). Headline: issuance is a mock modeled on
a live Stellar corporate bond (unnamed); the qualifying-payment history is supplied to the guest,
not yet attested (G1); the demo keeper reads buyers' openings from a local file (G2). The Groth16
proof, the on-chain verification, and the contracts are real — a settlement has verified on
testnet (tx `8b19cc71…`).

## Reporting

This is a hackathon testnet build; there is nothing of value to attack on-chain. If you find a
soundness issue in the determination rules, the journal/commitment encodings, or an
authorization path, please open an issue describing it. Production hardening (audit = gap **G5**)
is gated before any real issuance touches the system.
