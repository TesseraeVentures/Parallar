#!/usr/bin/env bash
#
# Deploy the Parallar stack to Stellar testnet and factory-deploy two instruments.
#
# Prereqs: rust + stellar-cli; funded `admin` and `deployer` identities in the config dir
# (the repo's gitignored .stellar/, or your global stellar config). The committed real-proof
# fixture supplies the guest image id. Records the result the way deployments/testnet.json does.
#
# MVP wiring note: settlement points its verifier DIRECTLY at the groth16-verifier — the
# production RiscZeroVerifierRouter is a thin selector-dispatcher with the identical
# verify(seal, image_id, journal) interface, so this is interface-compatible and upgradeable.
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
export STELLAR_RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
export STELLAR_NETWORK_PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
CFG="${STELLAR_CONFIG_DIR:-$PWD/.stellar}"
IMAGE_ID="${IMAGE_ID:-705ddac439284e2593aa8e510121e7e82dcc441549c7e8c8af0e77bd053d1891}"
VERIFIER_WASM="prover/host/tests/fixtures/groth16_verifier.wasm"

c1() { grep -oE 'C[A-Z2-7]{55}' | tail -1; }       # last contract id
h64() { grep -oE '[0-9a-f]{64}' | tail -1; }        # last 32-byte hex
inv() { stellar contract invoke --config-dir "$CFG" "$@" 2>&1 | grep -vE "local config|migrate|^ℹ️|Simulation|Signing|Submitting|Transaction"; }

ADMIN=$(stellar keys address admin --config-dir "$CFG" 2>/dev/null | grep -oE 'G[A-Z2-7]{55}')
DEPLOYER=$(stellar keys address deployer --config-dir "$CFG" 2>/dev/null | grep -oE 'G[A-Z2-7]{55}')
echo "admin=$ADMIN  deployer=$DEPLOYER"

echo "→ building contracts" && stellar contract build >/dev/null 2>&1

echo "→ deploying groth16-verifier"
VERIFIER=$(stellar contract deploy --wasm "$VERIFIER_WASM" --source deployer --config-dir "$CFG" 2>&1 | c1)

echo "→ uploading vault + settlement wasm"
VHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_vault.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)
SHASH=$(stellar contract upload --wasm target/wasm32v1-none/release/parallar_settlement.wasm --source deployer --config-dir "$CFG" 2>&1 | h64)

echo "→ deploying factory(admin, verifier)"
FACTORY=$(stellar contract deploy --wasm target/wasm32v1-none/release/parallar_factory.wasm \
  --source deployer --config-dir "$CFG" -- --admin "$ADMIN" --verifier_router "$VERIFIER" 2>&1 | c1)

echo "→ register_type(credit_v1)"
inv --id "$FACTORY" --source admin -- register_type --type_id credit_v1 \
  --t "{\"image_id\":\"$IMAGE_ID\",\"rules_version\":1,\"rules_uri\":\"ipfs://credit_v1\",\"vault_wasm\":\"$VHASH\",\"settlement_wasm\":\"$SHASH\"}" >/dev/null

echo "→ collateral = native XLM SAC; mark eligible"
XLM=$(stellar contract id asset --asset native --config-dir "$CFG" 2>&1 | c1)
inv --id "$FACTORY" --source admin -- set_collateral_eligible --token "$XLM" --eligible true >/dev/null

deploy_one() { # deploy_one <premium_bps> ; prints [iid, vault, settlement]
  local cfg="{\"reference_asset\":\"$ADMIN\",\"terms_hash\":\"${TERMS_HASH:-1111111111111111111111111111111111111111111111111111111111111111}\",\"schedule_root\":\"2222222222222222222222222222222222222222222222222222222222222222\",\"snapshot_root\":\"${SNAPSHOT_ROOT:-3333333333333333333333333333333333333333333333333333333333333333}\",\"collateral_token\":\"$XLM\",\"premium_bps\":$1,\"epoch_deadlines\":[[1,${DEADLINE:-1700000000}]]}"
  inv --id "$FACTORY" --source deployer -- deploy_instrument --type_id credit_v1 --config "$cfg" | grep -oE '\["[0-9a-f]{64}".*\]' | tail -1
}
echo "→ deploy_instrument #1" && I1=$(deploy_one 200)
echo "→ deploy_instrument #2 (replication beat)" && I2=$(deploy_one 300)

cat <<EOF

── Parallar testnet deployment ───────────────────────────────
verifier   $VERIFIER
factory    $FACTORY
collateral $XLM  (native XLM SAC)
type       credit_v1  image_id=$IMAGE_ID
instrument #1  $I1
instrument #2  $I2
explorer   https://stellar.expert/explorer/testnet/contract/$FACTORY
───────────────────────────────────────────────────────────────
(record these in deployments/testnet.json)
EOF
