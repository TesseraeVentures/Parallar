# Parallar — operations, governance & incident runbook (G6 / G9)

How a live Parallar deployment is governed and operated, and what to do when something goes
wrong. This is hackathon-stage documentation of the *production* operating model; the testnet
build itself has nothing of value to attack.

## Governance model (G6)

**There is exactly one privileged operation: `register_type` on the factory/registry.** Grep the
contracts and you will find no upgrade, no pause, no admin-pay, no mock-verify, no migrate — the
settlement contract's sole authorization is verifying one Groth16 proof (Law #1), and the vault's
sole payout path is the bound settlement (tested). So governance reduces to controlling type
registration and the deploy keys.

- **Registry admin → multisig.** Type registration is the only thing an admin can do; keep it
  that way and put it behind an N-of-M multisig. A compromised admin can register *new* types; it
  **cannot** alter or drain any deployed instrument (instruments are pinned to their `image_id`
  and their vault forever — the versioning law).
- **Key management.** Deployer/admin keys in HSM or institutional custody; testnet uses
  Friendbot keys in a gitignored `.env` (never real keys).
- **No emergency lever, by design.** There is deliberately no pause/override (it would violate
  Law #1). The liveness protections are instead: independent on-chain deadlines, permissionless
  settlement (anyone can settle after the deadline), and the escape hatch (`claim_direct`, G2).

## Incident runbook (G6)

**A bug is found in a deployed guest's rules.** The answer is structural, not a scramble:

1. Fix the guest, which produces a **new `image_id`**. Register it as a **new type** (e.g.
   `credit_v3`) via `register_type`. This is exactly the path `credit_v2` (attested, G1) already
   exercised — a stronger rule shipped as a new type.
2. **New instruments** deploy against the corrected type. **Existing instruments are immutable** —
   pinned to the guest they were deployed with; the bug cannot be retro-exploited to drain them
   because the only payout path is a proof against *their* pinned (old) image_id, and a corrected
   guest has a different id.
3. Comms protocol: disclose the issue and the new type; guide counterparties to deploy/migrate to
   it; publish the diff. Do **not** (cannot) touch live instruments.

**A collateral asset becomes clawback/freeze-enabled after deployment** (issuer changes flags):
de-list it from the eligible set (no *new* instruments can use it) and alert existing
counterparties. See eligibility below.

**A keeper goes offline / withholds settlement.** No action needed at the protocol level: after
the deadline anyone can settle (permissionless), and after the grace window any buyer can
`claim_direct` their own share (G2). Keeper liveness is a convenience, not a trust dependency.

## Collateral eligibility (G6) — honest design

The factory gates collateral through a **curated eligible-asset list** (`set_collateral_eligible`,
admin-governed). The intent is to admit only claw-proof / freeze-proof collateral so the reserve
can never be pulled out from under the protocol.

**Why not an on-chain flag read?** A Soroban contract **cannot introspect a classic Stellar
asset's issuer flags** (`AUTH_CLAWBACK_ENABLED`, `AUTH_REVOCABLE`, `AUTH_REQUIRED`) — those are
classic-account settings and the Stellar Asset Contract exposes no getter for them. So the
production design is **not** an on-chain flag read (which earlier docs optimistically suggested);
it is:

- the curated allowlist, governed by the registry multisig, admitting only assets whose issuer
  flags have been verified off-chain at listing time; plus
- an **off-chain monitor** that watches the issuer accounts of listed assets for flag changes and
  alerts / de-lists on any `AUTH_CLAWBACK_ENABLED` / `AUTH_REVOCABLE` transition; plus
- a strong default: native XLM (used in the demo) is structurally claw-proof and freeze-proof.

## State-rent / TTL operations (G9)

Soroban ledger entries carry a TTL; persistent/instance entries are **archival-class** (never
deleted — archived and auto-restorable at a fee if the TTL lapses), while temporary entries are
deleted. Parallar uses **no temporary storage** for any binding, so there is no permanent-loss
path. But a multi-year instrument's entries must be kept *live* so settlement never pays a
restoration fee mid-default. Operating duties:

- **Monitor** every live instrument's entries (`scripts/ttl_monitor.sh` lists the contracts and
  the extend commands; a production keeper polls RPC `getLedgerEntries` for each
  `liveUntilLedgerSeq` and extends before a tenor-derived threshold, alerting on near-expiry).
- **Tenor-derived extension.** The deployed contracts use a demo `extend_ttl(50, 100)`; a v-next
  contract version derives the extension window from the instrument tenor (a 10-year bond's vault
  must not quietly archive). This is a new-version contract change, not a retrofit.
- **Rent budgeting.** Budget rent per instrument at deploy time (the factory could escrow it).
- The Oct 2025 Protocol-23 archival incident is why this layer deserves vigilance.
