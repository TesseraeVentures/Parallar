# Parallar — Production Gap Register

**Purpose:** "production-right first time" means the *architecture* shipped at the hackathon is final — contract separation, generic journal, registry interface, guest plug-in boundary. This document enumerates everything that stands between the hackathon demo and a live pilot on a real client bond issuance, so hardening is a checklist, not a rebuild. Each gap names its trigger condition — most are sequenced after (a) Stellar ecosystem support post-hackathon and (b) a committed counterparty via the client issuance.

**Standing rule:** no real client paper, names, or funds touch this system until G1–G7 are closed. Putting a client's bond on unaudited contracts would damage the exact relationships this product exists to serve.

---

## G1 — Input canonicity (attested data feeds) · BLOCKING for pilot

**Today:** the keeper supplies payment history; the proof guarantees computation, not data canonicity. Permissionless keeping + on-chain deadline checks mitigate, not close.
**Production:** settlement guests accept only attested inputs — (a) issuer-signed payment attestations verified in-guest, and/or (b) oracle-attested ledger snapshots (Reflector or equivalent), and/or (c) light-client-style verification of ledger entry proofs in-guest (strongest; investigate feasibility on Stellar's ledger structure).
**Touches:** guest input section + `config_hash` contents only. Journal, contracts, registry unchanged.

## G2 — Buyer-held openings & escape hatch · BLOCKING for pilot

**Today:** demo keeper reads buyers' commitment openings from a local file.
**Production:** buyers hold their own openings (wallet-side); keepers settle from encrypted-to-keeper submissions or buyers co-sign settlement requests. Plus the Lighter-inspired **escape hatch**: a `claim_direct` path where a buyer proves their own position against `position_root` and claims their allocation if no keeper settles within a timeout — ZK as unconditional claimability.
**Touches:** vault (+1 entrypoint), one small guest. Core unchanged.

## G3 — Solvency mechanism finalization

**Today:** **Option B** shipped (decision June 17) — a running PUBLIC aggregate `total_cover` with `cover ≤ collateral` enforced on every purchase; an individual cover is revealed transiently in the buy tx and never persisted per-buyer (`contracts/vault/src/lib.rs`). Option C (purchase-time solvency proof) was the deferred-to-production choice.
**Production:** Option C (purchase-time solvency proof) regardless of what shipped; plus seller withdrawal queues replacing the blunt settlement-window freeze.

## G4 — Holder-set dynamics

**Today:** holder snapshot fixed at issuance.
**Production:** per-epoch snapshots (record-date model — matches real bond mechanics anyway) committed via G1's attestation path.

## G5 — Audit · BLOCKING for pilot

Scope: all four contracts + both guests + the verifier integration. Sequence after ecosystem support (potential SDF audit support/grants — raise in post-hackathon conversations). Pre-audit hygiene to maintain from day one: invariant tests green, no admin paths in settlement, frozen journal, pinned dependency commits.

## G6 — Operational security & governance

Registry admin key → multisig (type registration is the only privileged operation; keep it that way). Key management for deployer/admin (HSM or institutional custody). Collateral eligibility today is an admin-curated allowlist (`set_collateral_eligible`); production replaces the manual list with an on-chain read of the asset's `AUTH_CLAWBACK_ENABLED` / `AUTH_REVOCABLE` flags at deploy time, so the claw/freeze gate (§10.1) needs no trusted curation. Incident runbook: what happens if a guest bug is found post-deployment (answer by construction: register `credit_v2`, new instruments use it, existing instruments are immutable — document the comms protocol around this).

## G7 — Pilot infrastructure

Testnet pilot with the counterparty on a mirrored issuance before mainnet. Monitoring (settlement-window watchers, keeper liveness). Fee/proving-cost model at real N (current benchmarks extrapolate; measure). Remote-proving SLA or owned x86 proving infra.

## G8 — Additional instrument types (weather, trade settlement)

Not pilot-blocking; strategically central (proves the layer thesis). **Instance #2** (`weather_v1`): index settlement on attested observations — small once G1 exists. **Today:** the guest is built — `settle_weather_v1` (rainfall-shortfall parametric rule) compiles to image_id `d31246e6…`, is executor-verified in the zkVM, and is parity-tested byte-identical to `credit_v1` on the generic primitives, so the same factory/vault/settlement WASM accept it. Remaining: register the type + prove one live settlement on x86, then attested feeds (G1). **Instance #3** (`trade_settlement_v1`): commodity provisional-to-final invoicing (quotational pricing, quality adjustments, demurrage) with buyer funds as the reserve — the deepest commercial pool of the three, and the most attestation-dependent: requires G1 extended to signed trade documents (DCSA eBLs, inspection certificates verified in-guest) plus licensed index data (Platts/Argus) — the licensing question should be scoped before any build. The core is unchanged by design for both.

## G9 — State-rent (TTL) operations · BLOCKING for pilot

**Fact:** Soroban persistent entries are archived when TTL/rent lapses (auto-restorable at extra fee, but the Oct 2025 Protocol-23 archival incident shows this layer deserves vigilance). A live instrument's vault commitments, settled flags, and registry entries must remain live for the instrument's full tenor — potentially years.
**Today:** the registry uses `persistent` storage; per-instrument vault/settlement bindings (`position_root`, settled flags, collateral aggregates) use `instance` storage — both are archival-class (never `temporary`, auto-restorable, extended on access), so no permanent-loss path exists. The instance bundle is the MVP simplification.
**Production:** tier the long-lived per-instrument bindings into dedicated `persistent` entries with explicit per-entry rent budgeting; TTL monitoring + scheduled extension service (keeper duty or cron), rent budgeting per instrument at deploy time (factory could escrow rent), alerting on entries approaching expiry.

## G10 — Deep history access

**Fact:** default Stellar RPC retains events ~7 days; settling or auditing older epochs requires archive RPC/Horizon/history archives.
**Production:** history_builder backed by an archive data source (own Horizon, archive RPC provider, or — strongest — G1's attestation path making raw history access unnecessary at settlement time).

## G11 — Yield router (protected share class) · the distribution layer

**What:** TECH_SPEC §5A — wrap bond → pBOND receipt; coupons waterfall premium to the vault and net yield to holders (14% gross → 12% protected); cover auto-sized and auto-renewed; layered fees per §5A (base 12% + distribution 10–15% + structuring + float share; MGA-benchmarked on distributed flow).
**Sequencing:** after G1 + G5 (it touches client money flows and must launch audited); ideal flagship for the first issuance pilot, since "buy the protected share class" is the cleanest counterparty pitch. Build decisions parked deliberately: premium-in-arrears vs one-epoch escrow; the transparent-vs-sealed product-line split; insurance-distribution counsel review alongside the (P)SPI wrapper.

## G12 — Reserve yield management (float) · revenue + capacity

**What:** TECH_SPEC §3.2 — reserves held in yield-bearing eligible assets (tokenized T-bill/MMF class) so underwriters earn premium + float; protocol takes ~10% of float yield. Governed by the non-circularity covenant (reference exclusion, no self-reference, no rehypothecation, trigger-correlation gate) plus liquidity haircuts and denomination matching, enforced through the factory's eligible-reserve-asset list (distinct from the frozen type/instrument registry surface).
**Sequencing:** design with G3 (solvency) since haircuts change the cover ≤ reserves check; verify candidate assets' clawback flags and redemption rails (BENJI-class) before audit; valuation oracle for NAV-floating reserves is the one new dependency — keep MVP-pilot reserves in the payout asset if it isn't ready. Float yield is the revenue line that exists even in quiet quarters and the underwriter pitch that solves capacity cold-start.

## G13 — Lending-market integration (pBOND as enhanced collateral)

**What:** TECH_SPEC §5A composability thesis — pBOND's truncated loss distribution earns higher LTV in external lending markets (Blend first), making protection self-financing via capital efficiency and multiplying premium throughput through loops.
**Prerequisites in order:** (1) `credit_principal_v1` guest — principal protection at maturity; coupon-only cover supports partial uplift and must be presented to lenders as exactly that; (2) G11 yield router live and audited (pBOND must exist); (3) liquidation-compatible unwrap mechanics incl. mid-epoch premium accrual; (4) risk-parameter engagement with the lending protocol — bring the on-chain-provable reserve adequacy argument, it is the differentiator vs bilateral credit enhancement.
**Boundary (non-negotiable):** pBOND is permitted in external lending markets and prohibited in Parallar reserves (covenant rule 2). Loop leverage is user risk; published LTV-band guidance and stress-unwind documentation accompany any listing.

---

## Sequencing sketch (post-hackathon, support + counterparty secured)

1. **Weeks 1–3:** G1 (attestation design + guest integration, subsumes G10) ∥ G3 ∥ G2 escape hatch
2. **Weeks 3–5:** G4, G6, G9 (TTL ops), pre-audit hygiene pass, G7 benchmarks at real N incl. chunked-payout path
3. **Weeks 5–9:** G5 audit + remediation
4. **Weeks 9–12:** G7 testnet pilot on mirrored issuance → mainnet go/no-go
5. **Anytime after G1:** G8 instance #2

Estimates assume solo founder + Claude Code with SDF technical support; audit timing is the long pole and external.
