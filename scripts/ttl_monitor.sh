#!/usr/bin/env bash
#
# TTL keeper runbook (PRODUCTION_GAP G9). The off-chain duty that keeps a live instrument's
# storage from archiving over its multi-year tenor. Soroban instance/persistent entries are
# archival-class (never deleted, but archived + restorable-at-fee if the TTL lapses), so a
# long-tenor instrument needs periodic extension. This prints the contracts to keep live and the
# `stellar contract extend` command for each. A production keeper automates the poll (RPC
# getLedgerEntries -> liveUntilLedgerSeq) and extends when within a tenor-derived threshold.
set -euo pipefail
cd "$(dirname "$0")/.."
D="deployments/testnet.json"
LEDGERS="${LEDGERS:-500000}" # extension window in ledgers (production: derive from instrument tenor)

[ -f "$D" ] || { echo "missing $D"; exit 1; }
python3 - "$D" "$LEDGERS" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); ledgers = sys.argv[2]
rows = [("factory", d["contracts"]["factory"]), ("verifier", d["contracts"]["groth16_verifier"])]
for i, ins in enumerate(d["instruments"], 1):
    rows.append((f"instrument#{i} vault", ins["vault"]))
    rows.append((f"instrument#{i} settlement", ins["settlement"]))
print("Keep these contracts' storage live (extend before the TTL lapses):\n")
for name, cid in rows:
    print(f"  {name:<26} {cid}")
    print(f"    stellar contract extend --id {cid} --durability persistent \\")
    print(f"      --ledgers-to-extend {ledgers} --source <keeper> --network testnet\n")
print("Production keeper: poll RPC getLedgerEntries for each contract's instance key, read")
print("liveUntilLedgerSeq, extend when within a tenor-derived threshold, and alert on near-expiry.")
print("See docs/OPERATIONS.md (G9).")
PY
