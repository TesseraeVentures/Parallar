#!/usr/bin/env bash
#
# Register weather_v1 (INSTANCE #2) on the EXISTING Parallar factory and factory-deploy a
# weather instrument — the SAME generic vault + settlement WASM, a NEW guest type. This is the
# layer thesis on testnet: a second instrument family on the unchanged production surfaces, no
# contract changes. Deploy does NOT need x86/Docker; the live PROOF does (run on the x86 box).
#
# Prereqs:
#   * the credit deploy ran (deployments/testnet.json has contracts.factory + collateral SAC)
#   * the weather guest is built (image_id d31246e6…; `cargo build -p parallar-methods`)
#   * a scenario.json from gen_weather_scenario whose terms_hash + snapshot_root MATCH the
#     witness you will prove (so the deployed instrument_id == the proof's journal instrument_id)
#   * funded `admin` (registers the type) + `deployer` identities in the config dir
#
# Usage:
#   SCENARIO_XLM=<C…> SCENARIO_BUYER=<G…> SCENARIO_REFERENCE=<G…> \
#     cargo test -p parallar-prover-host --test gen_weather_scenario -- --ignored --nocapture
#   ./scripts/deploy_weather.sh
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
export STELLAR_RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
export STELLAR_NETWORK_PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
CFG="${STELLAR_CONFIG_DIR:-$PWD/.stellar}"
IMAGE_ID="${WEATHER_IMAGE_ID:-d31246e6d19379cfecbc23434e8c4aba0571e12cb6374b286ad3e9598db4a9bb}"
SCENARIO="${SCENARIO:-/tmp/parallar_weather_scenario/scenario.json}"
DEPLOY_JSON="deployments/testnet.json"

c1() { grep -oE 'C[A-Z2-7]{55}' | tail -1; }
h64() { grep -oE '[0-9a-f]{64}' | tail -1; }
inv() { stellar contract invoke --config-dir "$CFG" "$@" 2>&1 | grep -vE "local config|migrate|^ℹ️|Simulation|Signing|Submitting|Transaction"; }
jget() { python3 -c "import json,sys;print(json.load(open(sys.argv[2]))$1)" "x" "$2"; }

[ -f "$SCENARIO" ] || { echo "need a scenario.json at $SCENARIO (run gen_weather_scenario first)"; exit 1; }
FACTORY="${FACTORY:-$(jget "['contracts']['factory']" "$DEPLOY_JSON")}"
XLM="${XLM:-$(jget "['contracts']['collateral_xlm_sac']" "$DEPLOY_JSON")}"
echo "factory=$FACTORY  collateral=$XLM  weather image_id=$IMAGE_ID"

TERMS_HASH=$(jget "['config']['terms_hash']" "$SCENARIO")
SNAPSHOT_ROOT=$(jget "['config']['snapshot_root']" "$SCENARIO")
SCHEDULE_ROOT=$(jget "['config']['schedule_root']" "$SCENARIO")
REFERENCE=$(jget "['config']['reference_asset']" "$SCENARIO")
PREMIUM=$(jget "['config']['premium_bps']" "$SCENARIO")
DEADLINE=$(jget "['deadline']" "$SCENARIO")

echo "→ build + upload the generic vault + settlement WASM (identical to credit's)"
stellar contract build >/dev/null 2>&1
VHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_vault.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)
SHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_settlement.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)
echo "   vault_wasm=$VHASH  settlement_wasm=$SHASH"

echo "→ register_type(weather_v1)  (admin)"
inv --id "$FACTORY" --source admin -- register_type --type_id weather_v1 \
  --t "{\"image_id\":\"$IMAGE_ID\",\"rules_version\":1,\"rules_uri\":\"ipfs://weather_v1\",\"vault_wasm\":\"$VHASH\",\"settlement_wasm\":\"$SHASH\"}" >/dev/null

echo "→ deploy_instrument(weather_v1)  (deployer)"
CFG_JSON="{\"reference_asset\":\"$REFERENCE\",\"terms_hash\":\"$TERMS_HASH\",\"schedule_root\":\"$SCHEDULE_ROOT\",\"snapshot_root\":\"$SNAPSHOT_ROOT\",\"collateral_token\":\"$XLM\",\"premium_bps\":$PREMIUM,\"epoch_deadlines\":[[1,$DEADLINE]]}"
INST=$(inv --id "$FACTORY" --source deployer -- deploy_instrument --type_id weather_v1 --config "$CFG_JSON" | grep -oE '\["[0-9a-f]{64}".*\]' | tail -1)

cat <<EOF

── Parallar weather_v1 (instance #2) on testnet ──────────────
factory     $FACTORY
type        weather_v1  image_id=$IMAGE_ID
instrument  $INST
explorer    https://stellar.expert/explorer/testnet/contract/$FACTORY
──────────────────────────────────────────────────────────────
Then, on the x86 proving box, fund the vault + buy_protection with the scenario
commitment, and settle it:
  parallar-prover prove  --guest weather --inputs witness.json --out weather_proof.json
  parallar-prover submit --artifact weather_proof.json --settlement <settlement C…>
(record the instrument in deployments/testnet.json)
EOF
