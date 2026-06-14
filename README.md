# Parallar

**A verifiable settlement layer on Stellar. Credit default instruments are product #1.**

> Determination off-chain. Positions confidential. Settlement proven. Deploying a new protected instrument is one transaction.

*Submission to Stellar Hacks: Real-World ZK, June 2026.*

---

## The problem

Tokenized RWAs are arriving on Stellar, but the instruments built on top of them — protection, hedges, structured payouts — all hit the same three walls:

1. **Determination is heavy.** "Did the trigger occur?" (a coupon missed across 1,000 bondholders; an index breach over a data window) is computation no smart contract can run.
2. **Institutions won't trade on a transparent book.** Public position sizes leak credit exposure and strategy.
3. **Settlement requires trusting someone** — a committee, an operator, an admin key — to compute the payouts.

## What Parallar is

A factory of instruments sharing one provable settlement core:

- **Registry/Factory** — instrument *types* are registered: a settlement guest's image ID + version-pinned published rules + contract WASM hashes. Instrument *instances* (vault + settlement pair, cross-bound) deploy in **one transaction** via the Soroban deployer pattern. New guest version = new type; live instruments are pinned to their settlement logic forever — immutability institutions can underwrite against.
- **Generic core** — a vault holding seller collateral (public) and buyer positions as **Poseidon commitments only** (cover sizes never touch public state), and a settlement contract whose sole authorization path is verifying a Groth16 proof against the type's pinned image ID. No admin path exists.
- **Pluggable guests** — per-type RISC Zero programs. **Instance #1 (this repo):** parametric credit protection. The guest scans real payment history across all bondholders, establishes the coupon was missed or short-paid, opens every position commitment privately, computes payouts pro-rata to the default's severity, and proves the entire settlement. **Instance #2 (specified):** parametric weather/index protection — same core, different guest.

```
factory.deploy_instrument(credit_v1, config)          ← one tx, instrument live
        │
payment history ─┐
                 ├─► RISC Zero guest ─► Groth16 proof ─► settlement verifies ─► payouts
hidden positions ┘   (determine+settle)  (constant size)  (sole auth path)
```

**Why ZK is structural, not decorative:** the chain *cannot* compute these payouts — the determination is too heavy and the positions are hidden by construction. Remove the proof and no instrument in the family can exist. If the trigger did not occur, the guest panics on honest data: **no valid settlement proof can exist for a false claim.**

**Published settlement rule (credit_v1):** `payout = cover × (Σ shortfall / Σ owed)`, capped at cover. The proof enforces exactly this; the contract accepts no other allocation set. **Stellar-native default rules:** payments count net of clawback (an issuer cannot pay a coupon and take it back); freezing a holder's trustline counts as issuer default; a holder who removed their own trustline is excluded from owed (the issuer *cannot* pay them — holder-side failure is not issuer default). The factory refuses clawbackable or freezable assets as vault collateral, so the payout pool itself can never be pulled. Full normative list: TECH_SPEC §10.

## Trust model — stated plainly

Each proof guarantees the settlement **computation** — the version-pinned rules executed over the supplied input data and the committed positions — is correct (verifiable execution). It does not guarantee the input data is canonical: correct computation on inputs, not fair inputs. Mitigations today: permissionless keeping (anyone can settle) and independent on-chain deadline enforcement. The real fix, attested data feeds, is gap **G1** in [PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md), which enumerates the complete path from this demo to a live issuance pilot. Within the proof boundary, no party — including us — can inflate, favor, omit, or fabricate a payout.

## What's real / what's mocked

| Real | Mocked / simplified (→ production gap) |
|---|---|
| Factory deployment via the Soroban deployer pattern (vault+settlement cross-bound in one tx) | Issuance is a mock modeled on a live Stellar corporate bond (unnamed) |
| Bond as a Stellar asset — 10 holders, genuine (incl. partial) coupon transfers | Holder snapshot fixed at issuance |
| Poseidon-committed positions — cover sizes never touch public state | Flat premium (no pricing curve) |
| RISC Zero settlement proofs — real Groth16 generation (STARK→SNARK, Rosetta x86) | Demo keeper reads buyers' commitment openings from a local file (→ buyer-held openings + self-claim escape hatch, **G2**) |
| On-chain Groth16 verification is the settlement's **sole** payout path — a cross-contract call to the RISC Zero verifier router (BN254 + Poseidon host fns, Protocols 25–26); **no admin path exists** | Qualifying-payment history is supplied to the guest (not yet attested → **G1**) |

The proofs guarantee correct *computation over supplied inputs*, not input canonicity — see the trust model above and [PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md).

## Quick start

```bash
# prerequisites: rust, stellar-cli — docs/TECH_SPEC.md §2
make demo          # fresh-clone, one command: the full verified scenario (below)
make test          # build all four contracts to wasm + run the full suite
```

**`make demo`** (or `./demo.sh`) walks the whole scenario against real code — register `credit_v1` → **two instruments factory-deployed** → deposits + Poseidon-committed cover → a fully-paid epoch for which **no settlement proof can exist** → a partial default → a **real Groth16 proof verified on-chain** by the actual RISC Zero verifier → confidential payouts through the vault → forged-proof / replay / stale-root / tampered-allocation / pre-deadline attempts **reverting** → benchmarks. It's fresh-clone runnable in seconds: it uses the committed real-proof fixture (`prover/host/tests/fixtures/`), so **no testnet keys and no 34-minute live proof are needed**.

To generate a proof live and submit it to testnet, use the host CLI:

```bash
# generate a real Groth16 proof (needs Docker; ~34 min via Rosetta x86 on Apple Silicon):
cargo run -p parallar-prover-host --bin parallar-prover -- prove  --inputs witness.json --out proof.json
# submit it to a deployed instrument:
cargo run -p parallar-prover-host --bin parallar-prover -- submit --artifact proof.json --settlement <C…>
```

## Benchmarks

| Metric | Measured | Notes |
|---|---|---|
| On-chain Groth16 verify | **≈ 35M CPU insns** (Bn254Pairing 17.5M + G2-subgroup 11.8M + G1Mul 5.8M) | ~3× headroom under Soroban's ~100M/tx budget (verify step only) |
| Proof generation (N=1) | **2027.77 s** (~33.8 min) end-to-end | Apple-Silicon dev path via Rosetta-x86 Docker (STARK prove + SNARK wrap); production proving targets an x86 VM |

N=10 + 1k-holder extrapolation (proof time **and** verify fee / max allocation-list size, per TECH_SPEC §10.7) pending — run it push-button on x86 with `parallar-prover bench --inputs witness.json --n 10` (each proof is ~34 min under Rosetta, so this is an x86 job).

## Instance #2 (`weather_v1`) — same core, different guest

Parallar's claim is that protection is a *family* of instruments over one provable core, not a single product. The cleanest proof of that is a second instrument that reuses every frozen surface and changes only the one piece that is *meant* to change — the per-type guest.

A parametric weather/index instrument (e.g. a payout when rainfall over a window breaches a published threshold) maps onto the unchanged surfaces:

| Surface | `credit_v1` (built) | `weather_v1` (designed-for) | Changes? |
|---|---|---|---|
| Generic vault WASM | seller collateral + Poseidon-committed buyer positions | identical | **no** |
| Generic settlement WASM | sole auth path = verify one Groth16 proof vs the type's image ID | identical | **no** |
| 116-byte journal | the contract reads roots + allocation commitment, never the determination | identical layout | **no** |
| Registry interface | `register_type(image_id, rules, wasm hashes)` → `deploy_instrument` | identical call | **no** |
| Per-type RISC Zero guest | scans coupon payments, proves shortfall/owed, settles pro-rata | scans attested observations, proves the index breach, settles per the published parametric rule | **yes — this is the only new code** |

A new guest is a **new `image_id`, i.e. a new registered type** — never an in-place upgrade of an existing one. Live instruments stay pinned to the guest they were deployed with forever (the versioning law). So `weather_v1` ships the way `credit_v1` did: register the type, then factory-deploy instances against the *same* vault and settlement WASM already in this repo.

**Status: specified/designed-for, not built.** It is gap **G8** in [PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md) (small once attested feeds, **G1**, exist). The demo's replication beat — a **second** factory-deploy of `credit_v1` — already exercises the path a second *type* would take: the surfaces serve instance #2 by construction, not by speculative flexibility added on its behalf.

## Roadmap

[PRODUCTION_GAP.md](docs/PRODUCTION_GAP.md) in full; headlines: attested history feeds (G1) → buyer-held openings + escape-hatch self-claims (G2) → audit (G5) → testnet pilot on a mirrored real issuance → the **yield router** (wrap a bond, receive the protected share class: gross coupon in, premium to the reserve, net yield out — e.g. 14% → 12% protected) → further instrument types on the unchanged core: `weather_v1` (parametric index settlement) and `trade_settlement_v1` (commodity provisional-to-final invoicing — pricing, quality, demurrage — with buyer funds as the funded reserve) → multi-tranche vaults → regulated wrapper via Bermuda's BMA (P)SPI framework.

## Repo map

`demo.sh` · `Makefile` · `contracts/` (factory, bond, vault, settlement) · `prover/` (`guests/settle_credit_v1`, `host` + `parallar-prover` CLI, `methods`) · `spikes/poseidon_parity/` · `docs/` (PRD, TECH_SPEC, SPRINT_PLAN, PRODUCTION_GAP, STATUS) · `site/` (landing page) · `deck/` · `external/` (vendored Nethermind RISC Zero verifier, commit-pinned, gitignored)

MIT
