#!/usr/bin/env bash
#
# Deploy the CLAIMABLE family (PRODUCTION_GAP G2): claim_factory → a vault + ClaimableSettlement
# (permissionless settle + buyer self-claim after a keeper-grace window), cross-bound in one tx.
#
# Prereqs as scripts/deploy_testnet.sh: rust + stellar-cli; funded `admin` + `deployer` identities
# in the config dir. Mirrors deploy_testnet.sh's proven flow. Scaffolding for the x86 box — dry-run
# the invokes once and adjust arg formatting to your stellar-cli version before a real deploy.
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
export STELLAR_RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
export STELLAR_NETWORK_PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
CFG="${STELLAR_CONFIG_DIR:-$PWD/.stellar}"
SETTLE_IMAGE_ID="${SETTLE_IMAGE_ID:-705ddac439284e2593aa8e510121e7e82dcc441549c7e8c8af0e77bd053d1891}"
CLAIM_IMAGE_ID="${CLAIM_IMAGE_ID:-b4319def9a29fe76cf7789e33741d7181f6cae44d6ac661011ec5a85132124cc}"
GRACE="${GRACE:-100}"
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

echo "→ uploading vault + claim_settlement wasm"
VHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_vault.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)
CHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_claim_settlement.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)

echo "→ deploying claim_factory(admin, verifier, vault_wasm, claim_settlement_wasm)"
CF=$(stellar contract deploy --wasm target/wasm32v1-none/release/parallar_claim_factory.wasm \
  --source deployer --config-dir "$CFG" -- \
  --admin "$ADMIN" --verifier_router "$VERIFIER" --vault_wasm "$VHASH" --claim_settlement_wasm "$CHASH" 2>&1 | c1)

echo "→ register_claimable_type(credit_v1_claim) — both guest image_ids + the grace window"
inv --id "$CF" --source admin -- register_claimable_type --type_id credit_v1_claim \
  --t "{\"settle_image_id\":\"$SETTLE_IMAGE_ID\",\"claim_image_id\":\"$CLAIM_IMAGE_ID\",\"rules_version\":1,\"grace\":$GRACE}" >/dev/null

echo "→ collateral = native XLM SAC; mark eligible"
XLM=$(stellar contract id asset --asset native --config-dir "$CFG" 2>&1 | c1)
inv --id "$CF" --source admin -- set_collateral_eligible --token "$XLM" --eligible true >/dev/null

echo "→ deploy_claimable"
CFG_JSON="{\"reference_asset\":\"$ADMIN\",\"terms_hash\":\"${TERMS_HASH:-1111111111111111111111111111111111111111111111111111111111111111}\",\"schedule_root\":\"2222222222222222222222222222222222222222222222222222222222222222\",\"snapshot_root\":\"${SNAPSHOT_ROOT:-3333333333333333333333333333333333333333333333333333333333333333}\",\"collateral_token\":\"$XLM\",\"premium_bps\":200,\"epoch_deadlines\":[[1,${DEADLINE:-1700000000}]]}"
RES=$(inv --id "$CF" --source deployer -- deploy_claimable --type_id credit_v1_claim --config "$CFG_JSON")

cat <<EOF

── Parallar CLAIMABLE-family testnet deployment ───────────────
verifier       $VERIFIER
claim_factory  $CF
collateral     $XLM  (native XLM SAC)
settle/claim image_ids  $SETTLE_IMAGE_ID / $CLAIM_IMAGE_ID  (grace=$GRACE)
deploy_claimable result  $RES
explorer       https://stellar.expert/explorer/testnet/contract/$CF
───────────────────────────────────────────────────────────────
(record these in deployments/testnet.json; settle via 'prove --guest credit' then
 claim_direct via 'prove --guest claim' after the grace window — see docs/RUNBOOK.md)
EOF
