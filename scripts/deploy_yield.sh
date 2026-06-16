#!/usr/bin/env bash
#
# Deploy the YIELD family (PRODUCTION_GAP G11/G12): yield_factory → register a risk TIER, then
# deploy_protected (yield_vault + settlement + yield_router, cross-bound) for a bond into that tier.
# Optionally set_tranched_wasm + deploy_tranched for a first-loss tranched family.
#
# Prereqs as scripts/deploy_testnet.sh. Scaffolding for the x86 box — dry-run the invokes once and
# adjust arg formatting to your stellar-cli version before a real deploy.
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
export STELLAR_RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
export STELLAR_NETWORK_PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
CFG="${STELLAR_CONFIG_DIR:-$PWD/.stellar}"
IMAGE_ID="${IMAGE_ID:-705ddac439284e2593aa8e510121e7e82dcc441549c7e8c8af0e77bd053d1891}"
VERIFIER_WASM="prover/host/tests/fixtures/groth16_verifier.wasm"

c1() { grep -oE 'C[A-Z2-7]{55}' | tail -1; }
h64() { grep -oE '[0-9a-f]{64}' | tail -1; }
inv() { stellar contract invoke --config-dir "$CFG" "$@" 2>&1 | grep -vE "local config|migrate|^ℹ️|Simulation|Signing|Submitting|Transaction"; }

ADMIN=$(stellar keys address admin --config-dir "$CFG" 2>/dev/null | grep -oE 'G[A-Z2-7]{55}')
DEPLOYER=$(stellar keys address deployer --config-dir "$CFG" 2>/dev/null | grep -oE 'G[A-Z2-7]{55}')
echo "admin=$ADMIN  deployer=$DEPLOYER"

echo "→ building contracts" && stellar contract build >/dev/null 2>&1
echo "→ deploying groth16-verifier"
VERIFIER=$(stellar contract deploy --wasm "$VERIFIER_WASM" --source deployer --config-dir "$CFG" 2>&1 | c1)

echo "→ uploading yield_vault + settlement + yield_router wasm"
VHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_yield_vault.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)
SHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_settlement.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)
RHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_yield_router.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)

echo "→ deploying yield_factory(admin, verifier, vault_wasm, settlement_wasm, router_wasm)"
YF=$(stellar contract deploy --wasm target/wasm32v1-none/release/parallar_yield_factory.wasm \
  --source deployer --config-dir "$CFG" -- \
  --admin "$ADMIN" --verifier_router "$VERIFIER" --vault_wasm "$VHASH" --settlement_wasm "$SHASH" --router_wasm "$RHASH" 2>&1 | c1)

echo "→ register_tier(ig: investment grade, premium band 100–300 bps, haircut 0)"
inv --id "$YF" --source admin -- register_tier --tier_id ig --min_premium_bps 100 --max_premium_bps 300 --haircut_bps 0 --label "investment grade" >/dev/null

echo "→ collateral / coupon = native XLM SAC"
XLM=$(stellar contract id asset --asset native --config-dir "$CFG" 2>&1 | c1)
# bond_token: a separate SAC in production; reuse XLM here for the scaffold.
BOND="${BOND_TOKEN:-$XLM}"

echo "→ deploy_protected(ig, cfg) — risk-priced premium 200 bps within the tier band"
INSTR_ID="${INSTRUMENT_ID:-aa$(printf '%062d' 1)}"
PCFG="{\"instrument_id\":\"$INSTR_ID\",\"image_id\":\"$IMAGE_ID\",\"bond_token\":\"$BOND\",\"coupon_token\":\"$XLM\",\"premium_bps\":200,\"protocol_fee_bps\":1200,\"dist_fee_bps\":1000,\"epoch_deadlines\":[[1,${DEADLINE:-1700000000}]]}"
RES=$(inv --id "$YF" --source admin -- deploy_protected --tier_id ig --cfg "$PCFG")

cat <<EOF

── Parallar YIELD-family testnet deployment ───────────────────
verifier        $VERIFIER
yield_factory   $YF
collateral/coupon $XLM   bond $BOND
tier            ig (100–300 bps band)   instrument premium 200 bps
deploy_protected result  $RES
explorer        https://stellar.expert/explorer/testnet/contract/$YF
───────────────────────────────────────────────────────────────
For a TRANCHED family: upload parallar_tranched_vault.wasm, call set_tranched_wasm on the
factory, then deploy_tranched(tier, {…,weights:[3,2,1]}). Record results in deployments/testnet.json.
EOF
