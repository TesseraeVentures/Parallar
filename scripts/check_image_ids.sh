#!/usr/bin/env bash
#
# Reproducibility guard: rebuild the guest ELFs and assert each image_id matches its pinned
# value. credit_v1 MUST equal the deployed type's image_id (deployments/testnet.json) — the
# versioning law: live instruments are pinned to it forever. A drift here (e.g. an accidental
# guest dev-dependency, as caught in R34/R35) means a freshly built proof would no longer verify
# against the deployed instruments. Needs the RISC Zero toolchain; run on the x86 box or in CI.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.risc0/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

# weather_v1 / credit_v2 are pinned in the docs (not deployed); credit_v1 is the deployed type.
EXPECT_WEATHER=d31246e6d19379cfecbc23434e8c4aba0571e12cb6374b286ad3e9598db4a9bb
EXPECT_CREDIT2=d07e6aaf3e7506883bce340c019cd995e313359b062abf0bfab2b7e0bafecb3a
EXPECT_CLAIM=b4319def9a29fe76cf7789e33741d7181f6cae44d6ac661011ec5a85132124cc
EXPECT_CREDIT1=$(python3 -c "import json;print(json.load(open('deployments/testnet.json'))['type']['image_id'])")

echo "→ building the guest ELFs (parallar-methods)…"
( cd prover && cargo build -p parallar-methods >/dev/null 2>&1 )
gen=$(find prover/target -name methods.rs -path '*out*' -exec ls -t {} + | head -1)

python3 - "$gen" "$EXPECT_CREDIT1" "$EXPECT_WEATHER" "$EXPECT_CREDIT2" "$EXPECT_CLAIM" <<'PY'
import re, sys
gen, ec1, ew, e2, ecl = sys.argv[1:6]
t = open(gen).read()
ids = {}
for m in re.finditer(r'(\w+)_GUEST_ID: \[u32; 8\] = \[([\d, ]+)\]', t):
    nums = [int(x) for x in m.group(2).split(',')]
    ids[m.group(1)] = ''.join(w.to_bytes(4, 'little').hex() for w in nums)
expect = {
    'SETTLE_CREDIT_V1': ec1,
    'SETTLE_WEATHER_V1': ew,
    'SETTLE_CREDIT_V2': e2,
    'CLAIM_CREDIT_V1': ecl,
}
ok = True
for k, v in expect.items():
    got = ids.get(k, '(missing)')
    status = 'ok' if got == v else 'MISMATCH'
    if got != v:
        ok = False
    print(f"  {k:<18} {got[:16]}…  {status}")
print("image_ids match pinned values ✓" if ok else "IMAGE_ID DRIFT — a guest ELF changed; do NOT ship", file=sys.stderr if not ok else sys.stdout)
sys.exit(0 if ok else 1)
PY
