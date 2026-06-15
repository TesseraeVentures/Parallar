# Parallar — Production Gap Register

**Purpose:** "production-right first time" means the *architecture* shipped at the hackathon is final — contract separation, generic journal, registry interface, guest plug-in boundary. This document enumerates everything that stands between the hackathon demo and a live pilot on a real client bond issuance, so hardening is a checklist, not a rebuild. Each gap names its trigger condition — most are sequenced after (a) Stellar ecosystem support post-hackathon and (b) a committed counterparty via the client issuance.

**Standing rule:** no real client paper, names, or funds touch this system until G1–G7 are closed. Putting a client's bond on unaudited contracts would damage the exact relationships this product exists to serve.

---

## G1 — Input canonicity (attested data feeds) · BLOCKING for pilot

**Today:** the keeper supplies payment history; the proof guarantees computation, not data canonicity. Permissionless keeping + on-chain deadline checks mitigate, not close. **A first cut is BUILT:** `settle_credit_v2` (image_id `d07e6aaf…`) implements path (a) — it verifies an issuer Ed25519 signature over the payment snapshot IN-GUEST (the issuer key is committed in `terms_hash`), executor-verified in the zkVM, registerable as a new type via the unchanged factory. credit_v1 stays pinned (the versioning law).
**Production:** settlement guests accept only attested inputs — (a) issuer-signed payment attestations verified in-guest (prototyped as credit_v2), and/or (b) oracle-attested ledger snapshots (Reflector or equivalent), and/or (c) light-client-style verification of ledger entry proofs in-guest (strongest; investigate feasibility on Stellar's ledger structure).
**Touches:** guest input section + `config_hash` contents only. Journal, contracts, registry unchanged.

## G2 — Buyer-held openings & escape hatch · BLOCKING for pilot

**Today (base escape hatch, BUILT):** settlement is already PERMISSIONLESS — after the deadline, anyone (including a covered buyer who assembles the witness from public chain data) can submit a valid proof and settle; no keeper has special power (tested: `settle_is_permissionless_no_privileged_caller`). The demo keeper reads buyers' openings from a local file.
**ZK core of the single-buyer claim, BUILT:** `claim_credit_v1` proves a SINGLE buyer's allocation against the committed `position_root` using only the PUBLIC commitments (recoverable from `buy_protection` tx history) + that buyer's OWN opening — other buyers' openings stay private — and emits a single-allocation journal identical in amount to what a full settlement would pay (tested). "ZK as unconditional claimability."
**Surface decision — RESOLVED via path (a), and the contract is BUILT:** `parallar-claim-settlement` (a NEW instrument-family settlement variant; the deployed generic settlement stays frozen) exposes `claim_direct(proof, journal, claimant, amount)` — it verifies a single-allocation claim proof against the CLAIM guest image_id (`b4319def…`, `claim_credit_v1`) and pays one buyer, gated by deadline+grace, with per-claimant dedup and `settle`/`claim_direct` MUTUAL EXCLUSION so neither path can double-pay (the vault's Σ ≤ collateral bounds cumulative claims). 8 contract tests + the claim guest's zkVM executor parity. It carries TWO image_ids (settle + claim) at init. **Remaining:** a claimable-family deploy path (the existing factory wires one image_id per type; a claimable factory variant or a config-carried claim_image_id supplies both) + a live claim on x86. Path (b) — a Merkle-tree `position_root` (new vault) for knows-ONLY-your-own-opening inclusion — remains the alternative refinement.
**Production also:** buyers hold their own openings (wallet-side); keepers settle from encrypted-to-keeper submissions or buyers co-sign settlement requests.

## G3 — Solvency mechanism finalization

**Today:** **Option B** shipped (decision June 17) — a running PUBLIC aggregate `total_cover` with `cover ≤ collateral` enforced on every purchase; an individual cover is revealed transiently in the buy tx and never persisted per-buyer (`contracts/vault/src/lib.rs`). Option C (purchase-time solvency proof) was the deferred-to-production choice.
**Option C — ZK core BUILT:** `solvency_v1` (`prover/guests/solvency_v1`) proves a purchase preserves solvency (`new_total ≤ collateral`) while hiding BOTH the cover and the running totals (the aggregate is a Poseidon commitment), and binds the same hidden cover to the buyer's position commitment — closing Option B's transient-cover leak. 7 native tests incl. `the_cover_never_appears_in_the_journal` (structural privacy check) and `cover_must_match_the_position_commitment` (no cover-swap). Journal: prev/new aggregate commitments + position commitment + collateral (112 bytes); no cover.
**Production — remaining (a NEW vault version, deployed vault stays pinned):** a confidential-cover vault that stores the aggregate as a commitment: `buy_protection_proven` advances it against a `solvency_v1` proof; `withdraw` likewise proves collateral-after ≥ committed cover (its current plaintext check no longer applies once the aggregate is hidden); plus the keeper/sequencer coordination that hands the current aggregate opening to the next buyer (a confidential running aggregate). Also seller withdrawal queues replacing the blunt settlement-window freeze.

## G4 — Holder-set dynamics

**Today:** holder snapshot fixed at issuance.
**Production:** per-epoch snapshots (record-date model — matches real bond mechanics anyway) committed via G1's attestation path.

## G5 — Audit · BLOCKING for pilot

Scope: all four contracts + both guests + the verifier integration. Sequence after ecosystem support (potential SDF audit support/grants — raise in post-hackathon conversations). Pre-audit hygiene to maintain from day one: invariant tests green, no admin paths in settlement, frozen journal, pinned dependency commits.

## G6 — Operational security & governance

**Built (docs):** the governance model + incident runbook + collateral-eligibility design + TTL ops are documented in [docs/OPERATIONS.md](OPERATIONS.md). Registry admin key → multisig (type registration is the only privileged operation; keep it that way). Key management for deployer/admin (HSM or institutional custody). **Collateral eligibility — corrected design:** a Soroban contract CANNOT introspect a classic asset's issuer flags (`AUTH_CLAWBACK_ENABLED` / `AUTH_REVOCABLE`) on-chain (no SAC getter exposes them), so the earlier "on-chain flag read" is not feasible; the production gate is the curated allowlist governed by the registry multisig, eligibility verified off-chain at listing, plus an off-chain monitor that de-lists on any issuer-flag change (native XLM is structurally claw/freeze-proof). Incident runbook (by construction): a guest bug ships as a NEW type (e.g. `credit_v3`, exactly as `credit_v2` did); new instruments use it; existing instruments are immutable — see OPERATIONS.md for the comms protocol.

## G7 — Pilot infrastructure

Testnet pilot with the counterparty on a mirrored issuance before mainnet. Monitoring (settlement-window watchers, keeper liveness). Fee/proving-cost model at real N (current benchmarks extrapolate; measure). Remote-proving SLA or owned x86 proving infra.

## G8 — Additional instrument types (weather, trade settlement)

Not pilot-blocking; strategically central (proves the layer thesis). **Instance #2** (`weather_v1`): index settlement on attested observations — small once G1 exists. **Today:** the guest is built — `settle_weather_v1` (rainfall-shortfall parametric rule) compiles to image_id `d31246e6…`, is executor-verified in the zkVM, and is parity-tested byte-identical to `credit_v1` on the generic primitives, so the same factory/vault/settlement WASM accept it. Remaining: register the type + prove one live settlement on x86, then attested feeds (G1). **Principal protection (`credit_principal`) needs NO new guest:** it is `credit_v1` with `coupon_rate_bps = 10000` (owed = the full principal) over a single maturity epoch — demonstrated in `settle_credit_v1` tests (`principal_protection_is_a_credit_v1_config_no_new_guest`, `partial_principal_repayment_pays_pro_rata`). The surfaces serve it by config (anti-gold-plating: zero new code); G13's "credit_principal_v1" is therefore a deploy-time config, not a guest. **Instance #3** (`trade_settlement_v1`): commodity provisional-to-final invoicing (quotational pricing, quality adjustments, demurrage) with buyer funds as the reserve — the deepest commercial pool of the three, and the most attestation-dependent: requires G1 extended to signed trade documents (DCSA eBLs, inspection certificates verified in-guest) plus licensed index data (Platts/Argus) — the licensing question should be scoped before any build. The core is unchanged by design for both.

## G9 — State-rent (TTL) operations · BLOCKING for pilot

**Fact:** Soroban persistent entries are archived when TTL/rent lapses (auto-restorable at extra fee, but the Oct 2025 Protocol-23 archival incident shows this layer deserves vigilance). A live instrument's vault commitments, settled flags, and registry entries must remain live for the instrument's full tenor — potentially years.
**Today:** the registry uses `persistent` storage; per-instrument vault/settlement bindings (`position_root`, settled flags, collateral aggregates) use `instance` storage — both are archival-class (never `temporary`, auto-restorable, extended on access), so no permanent-loss path exists. The instance bundle is the MVP simplification.
**Production:** tier the long-lived per-instrument bindings into dedicated `persistent` entries with explicit per-entry rent budgeting (a v-next contract change, not a retrofit); TTL monitoring + scheduled extension service (keeper duty or cron), rent budgeting per instrument at deploy time (factory could escrow rent), alerting on entries approaching expiry. **Built:** the keeper runbook `scripts/ttl_monitor.sh` (lists the contracts + the `stellar contract extend` command per contract) + the operating design in [docs/OPERATIONS.md](OPERATIONS.md); the deployed contracts use a demo `extend_ttl(50,100)`, tenor-derived extension is the v-next change.

## G10 — Deep history access

**Fact:** default Stellar RPC retains events ~7 days; settling or auditing older epochs requires archive RPC/Horizon/history archives.
**Production:** history_builder backed by an archive data source (own Horizon, archive RPC provider, or — strongest — G1's attestation path making raw history access unnecessary at settlement time).

## G11 — Yield router (protected share class) · the distribution layer

**What:** TECH_SPEC §5A — wrap bond → pBOND receipt; coupons waterfall premium to the vault and net yield to holders (14% gross → 12% protected); cover auto-sized and auto-renewed; layered fees per §5A (base 12% + distribution 10–15% + structuring + float share; MGA-benchmarked on distributed flow).
**Foundation BUILT:** `contracts/yield_vault` (`parallar-yield-vault`, a new instrument-family vault version) makes both sides' premium economics real — buyers pay a premium on `buy_protection` (or it arrives via `receive_premium` from the router's coupon waterfall); underwriters earn it pro-rata to collateral via a rewards-per-share accrual (`claim_premium`); the protocol takes a base fee (`claim_protocol_fee`, ~12% per §5A). The default path (`pay_allocations`, settlement-only) is unchanged and pays only from the reserve — premium is a separate pool (Law #1). 9 tests.
**Router BUILT:** `contracts/yield_router` (`parallar-yield-router`) — `wrap` mints pBOND (cover auto-sizes to the wrapped balance, registered with the vault, solvency-bounded); `route_coupon` runs the §5A waterfall (premium → vault [→ underwriters + base fee], distribution fee → router, NET → pBOND holders pro-rata); `unwrap` burns + returns the bond; pBOND `transfer` makes it usable as external collateral (G13). 4 tests incl. the full 14%-gross → 12%-net waterfall. Premium-in-arrears resolution: premium deducted per coupon (a defaulted epoch routes none; settlement pays the holders). Visibility tension: pBOND cover is public by design (the transparent product line); the sealed line stays on the vault's commitment path. **Tiered factory BUILT:** `contracts/yield_factory` (`parallar-yield-factory`, a new factory version) registers standardised RISK TIERS (a `[min,max]` premium band + haircut + label per risk profile) and `deploy_protected` cross-binds a full family (yield_vault + settlement + yield_router) for ANY bond in one tx, with the instrument's premium RISK-PRICED within its tier's band — so different bonds yield different net coupons (tested: investment-grade 2%→600 net vs high-yield 10%→200 net). Each instrument keeps its own reserve. The tier is the unit of underwriter appetite. **Remaining:** the one-epoch-escrow alternative (if chosen over arrears); a shared-reserve-per-tier option (correlated-default decision); counsel review of the distribution model (the (P)SPI wrapper) before mainnet.
**Sequencing:** after G1 + G5 (it touches client money flows and must launch audited); ideal flagship for the first issuance pilot, since "buy the protected share class" is the cleanest counterparty pitch. Build decisions parked deliberately: premium-in-arrears vs one-epoch escrow; the transparent-vs-sealed product-line split; insurance-distribution counsel review alongside the (P)SPI wrapper.

## G12 — Reserve yield management (float) · revenue + capacity

**What:** TECH_SPEC §3.2 — reserves held in yield-bearing eligible assets (tokenized T-bill/MMF class) so underwriters earn premium + float; protocol takes ~10% of float yield. Governed by the non-circularity covenant (reference exclusion, no self-reference, no rehypothecation, trigger-correlation gate) plus liquidity haircuts and denomination matching, enforced through the factory's eligible-reserve-asset list (distinct from the frozen type/instrument registry surface).
**Sequencing:** design with G3 (solvency) since haircuts change the cover ≤ reserves check; verify candidate assets' clawback flags and redemption rails (BENJI-class) before audit; valuation oracle for NAV-floating reserves is the one new dependency — keep MVP-pilot reserves in the payout asset if it isn't ready. Float yield is the revenue line that exists even in quiet quarters and the underwriter pitch that solves capacity cold-start.
**Built:** `yield_vault::harvest_float` distributes float yield to underwriters pro-rata (same accrual as premium) minus the protocol's float share (`set_float_fee_bps`, ~10%); the liquidity haircut (`total_cover ≤ (1−h)·collateral`) is enforced in the solvency floor; the non-circularity covenant (4 rules) + denomination matching are documented in [docs/ECONOMICS.md](ECONOMICS.md). **Remaining:** the live yield-strategy adapter (a real BENJI-class asset + a NAV oracle) behind `harvest_float`, and the on-chain eligible-reserve-asset list enforcing rules 1/2 (reject reference/pBOND reserves) — both audit-gated.

## G13 — Lending-market integration (pBOND as enhanced collateral)

**What:** TECH_SPEC §5A composability thesis — pBOND's truncated loss distribution earns higher LTV in external lending markets (Blend first), making protection self-financing via capital efficiency and multiplying premium throughput through loops.
**Prerequisites in order:** (1) `credit_principal_v1` guest — principal protection at maturity; coupon-only cover supports partial uplift and must be presented to lenders as exactly that; (2) G11 yield router live and audited (pBOND must exist); (3) liquidation-compatible unwrap mechanics incl. mid-epoch premium accrual; (4) risk-parameter engagement with the lending protocol — bring the on-chain-provable reserve adequacy argument, it is the differentiator vs bilateral credit enhancement.
**Boundary (non-negotiable):** pBOND is permitted in external lending markets and prohibited in Parallar reserves (covenant rule 2). Loop leverage is user risk; published LTV-band guidance and stress-unwind documentation accompany any listing.
**Built / documented:** pBOND is a transferable receipt (`yield_router::transfer`), so it can already post as external collateral; principal protection (the full-LTV-uplift case) is a `credit_v1` config at 100% rate over the maturity epoch — NO new guest (demonstrated in tests). The covenant boundary, the LTV/loss-truncation thesis, and the honest constraints (coupon ≠ principal) are in [docs/ECONOMICS.md](ECONOMICS.md). **Remaining:** the external lending-market listing itself (Blend) + risk-parameter engagement — external + audit-gated.

---

## Sequencing sketch (post-hackathon, support + counterparty secured)

1. **Weeks 1–3:** G1 (attestation design + guest integration, subsumes G10) ∥ G3 ∥ G2 escape hatch
2. **Weeks 3–5:** G4, G6, G9 (TTL ops), pre-audit hygiene pass, G7 benchmarks at real N incl. chunked-payout path
3. **Weeks 5–9:** G5 audit + remediation
4. **Weeks 9–12:** G7 testnet pilot on mirrored issuance → mainnet go/no-go
5. **Anytime after G1:** G8 instance #2

Estimates assume solo founder + Claude Code with SDF technical support; audit timing is the long pole and external.
