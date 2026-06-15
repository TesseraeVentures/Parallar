# Parallar — protocol economics & the non-circularity covenant (G11 / G12 / G13)

How money flows for **both sides**, what the protocol earns, and the hard rules that keep the
reserve safe. The mechanics here are built (`contracts/yield_vault`, `contracts/yield_router`)
except where marked; the regulated-wrapper and external-lending pieces are sequenced behind audit.

## The two sides

**Underwriters** (protection sellers) deposit collateral into the reserve and earn:
- **premium** — paid by cover buyers, distributed pro-rata to collateral (`yield_vault`,
  rewards-per-share; `claim_premium`); and
- **float** — yield on the idle reserve held in a yield-bearing eligible asset, distributed the
  same way (`harvest_float`), minus the protocol's float share.

**Cover buyers / protected holders** get paid on a default, solely through a verified settlement
proof (Law #1 — unchanged). Two ways in:
- **Sealed cover** (institutions concealing their book): `buy_protection` with a Poseidon
  commitment; the buyer pays the premium upfront; cover size stays private.
- **Protected share class** (the transparent line — treasuries, funds, retail): `wrap` a bond into
  **pBOND** via the `YieldRouter`; the coupon is routed automatically and the holder receives the
  net protected yield. Cover is the wrapped balance (public by design on this line).

## The coupon waterfall (protected share class, §5A)

For each epoch's gross coupon on wrapped bonds (`YieldRouter::route_coupon`):

```
gross coupon
  ├─ premium  = wrapped × premium_bps
  │     ├─ distribution fee (router's MGA-equivalent take, §5A(b))   → protocol
  │     └─ remainder → vault.receive_premium
  │             ├─ base fee (~12%)                                   → protocol
  │             └─ remainder → underwriters (pro-rata to collateral)
  └─ net = gross − premium                                           → pBOND holders (pro-rata)
```

Worked example (built + tested): a 14% gross coupon on 5,000 wrapped, 2% premium, 12% base fee,
10% distribution fee → **600 net to the holder (12% protected), 80 premium to underwriters,
10 base fee, 10 distribution fee = 700.** "Buy the protected version of the yield."

**Premium-in-arrears (§5A tension 2):** premium is deducted per coupon, so a defaulted epoch
routes no premium — exactly when underwriters pay out. Resolution shipped: pricing reflects it;
the one-epoch-escrow alternative is noted in PRODUCTION_GAP G11. **Visibility (tension 1):** the
router must see wrapped balances, so pBOND cover is public — hence the two product lines on one
core; the sealed line keeps the commitment path.

## The protocol's revenue (layered, MGA-benchmarked, §5A)

(a) base protocol fee ~12% on all premium reaching the vault · (b) distribution fee +10–15% on
premium the router originates via the protected share class (blended take ≈ 22–27% on distributed
flow vs MGA benchmarks of 20–30% of GWP) · (c) structuring fees on bespoke notional · (d) ~10% of
reserve float yield (§3.2). Marketplace-only flow stays at the 12% base to remain fork-resistant.

## Reserve float & the non-circularity covenant (§3.2, G12)

The reserve runs as insurance float — held in yield-bearing eligible assets so underwriter return
= premium + float. Eligibility is governed by **four hard rules** (the eligible-reserve-asset list,
a factory surface distinct from the frozen type registry):

1. **Reference exclusion** — a reserve asset may not be the reference asset, nor issued/guaranteed
   by the reference issuer or its affiliates.
2. **No self-reference** — no Parallar receipts (pBOND) and no Parallar-protected assets as
   reserves; recursive cover is prohibited by construction.
3. **No rehypothecation** — the vault holds sole title; reserve assets cannot simultaneously
   collateralize any other obligation.
4. **Trigger-correlation gate** — assets correlated with the instrument's trigger are excluded
   (e.g. no USDC reserves backing USDC-depeg cover).

Plus **liquidity haircuts** (`total_cover ≤ (1−h)·reserve_value`, h per asset tier — enforced
on-chain in `yield_vault`'s solvency floor) and **denomination matching** (reserve = payout asset,
or instantly redeemable into it within the settlement window). MVP holds XLM (h=0, no yield); the
yield-strategy adapter slots in behind `harvest_float` without touching the settlement surfaces.
Rules 1/2 are partly on-chain-enforceable (reject reference/pBOND reserves); 3/4 are governance,
verified off-chain at listing under the registry multisig (docs/OPERATIONS.md).

## pBOND as enhanced collateral (G13, composability thesis)

A protected bond has a **truncated loss distribution** — exactly what lending-market risk teams
price — so pBOND should command higher LTV and gentler liquidation than the naked bond (Blend on
Stellar is the natural first venue). pBOND is a transferable receipt (`YieldRouter::transfer`), so
it can post as external collateral today; the lending integration itself is external + audit-gated.
Honest constraints, in order:

1. **Coupon ≠ principal.** `credit_v1` protects coupons; full LTV-uplift needs principal protection
   — which is a `credit_v1` config at rate 100% over the maturity epoch (no new guest; demonstrated
   in tests). Coupon cover alone justifies *partial* uplift, presented plainly to lenders.
2. **Covenant boundary (non-negotiable).** pBOND in external lending = user leverage, permitted;
   pBOND inside Parallar reserves = prohibited (rule 2). The loop must never route back into the
   collateral backing the protection.
3. **Provable reserve adequacy.** Fully-funded-by-construction means a lender can verify reserve
   adequacy on-chain — credit enhancement a risk team can price, unlike a bilateral CDS promise.
4. Loop leverage is user risk; LTV bands + stress-unwind behavior are documentation, not protocol
   guarantees.

## Built vs remaining

**Built:** premium collection + pro-rata underwriter distribution + protocol base fee
(`yield_vault`); the coupon-waterfall router + pBOND + distribution fee + net-to-holder
(`yield_router`); float-yield distribution + the haircut solvency floor (`harvest_float`); pBOND
transferability. **Remaining:** the live yield-strategy adapter (a real BENJI-class asset + NAV
oracle), the factory deploying the router + the eligible-reserve list, the external lending
listing, and counsel review of the (P)SPI distribution wrapper — all post-audit (G5) / money-flow
gated.
