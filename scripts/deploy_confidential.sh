#!/usr/bin/env bash
#
# Deploy the CONFIDENTIAL-COVER vault (PRODUCTION_GAP G3). Unlike the factory families, the
# confidential_vault is deployed + init'd directly (its init takes an off-chain-computed
# initial_cover_commitment = commit_total(0, salt0), printed by the solvency witness generator).
# A standard settlement is deployed + cross-bound for the proof-gated DEFAULT payouts; the vault's
# own solvency_image_id gates the confidential buy/withdraw proofs.
#
# Prereqs as scripts/deploy_testnet.sh. Scaffolding for the x86 box — dry-run the invokes once and
# adjust arg formatting to your stellar-cli version before a real deploy.
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
export STELLAR_RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
export STELLAR_NETWORK_PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
CFG="${STELLAR_CONFIG_DIR:-$PWD/.stellar}"
# the credit guest for the DEFAULT settlement path; the solvency guest for the confidential buy/withdraw.
SETTLE_IMAGE_ID="${SETTLE_IMAGE_ID:-705ddac439284e2593aa8e510121e7e82dcc441549c7e8c8af0e77bd053d1891}"
SOLVENCY_IMAGE_ID="${SOLVENCY_IMAGE_ID:-c0b358d4606fa821706d9d6c61138796f8721c48f45cc20aa5c8e843c375aff2}"
# commit_total(0,[1;32]) from `gen_solvency_witnesses`; regenerate if you use a different salt0.
INITIAL_COVER_COMMITMENT="${INITIAL_COVER_COMMITMENT:-2ad6a6d586890de4416eebb8533b89a09e9dbf7041886a2d63f621391d10cbf6}"
INSTRUMENT_ID="${INSTRUMENT_ID:-cc00000000000000000000000000000000000000000000000000000000000001}"
VERIFIER_WASM="prover/host/tests/fixtures/groth16_verifier.wasm"

c1() { grep -oE 'C[A-Z2-7]{55}' | tail -1; }
inv() { stellar contract invoke --config-dir "$CFG" "$@" 2>&1 | grep -vE "local config|migrate|^ℹ️|Simulation|Signing|Submitting|Transaction"; }

ADMIN=$(stellar keys address admin --config-dir "$CFG" 2>/dev/null | grep -oE 'G[A-Z2-7]{55}')
DEPLOYER=$(stellar keys address deployer --config-dir "$CFG" 2>/dev/null | grep -oE 'G[A-Z2-7]{55}')
echo "admin=$ADMIN  deployer=$DEPLOYER"

echo "→ building contracts" && stellar contract build >/dev/null 2>&1
echo "→ deploying groth16-verifier"
VERIFIER=$(stellar contract deploy --wasm "$VERIFIER_WASM" --source deployer --config-dir "$CFG" 2>&1 | c1)
XLM=$(stellar contract id asset --asset native --config-dir "$CFG" 2>&1 | c1)

echo "→ deploying confidential_vault + settlement (no constructor args; init below)"
VAULT=$(stellar contract deploy --wasm target/wasm32v1-none/release/parallar_confidential_vault.wasm --source deployer --config-dir "$CFG" 2>&1 | c1)
SETTLEMENT=$(stellar contract deploy --wasm target/wasm32v1-none/release/parallar_settlement.wasm --source deployer --config-dir "$CFG" 2>&1 | c1)

echo "→ cross-bind: confidential_vault.init(settlement, collateral, admin, solvency_image_id, verifier, fee, initial_cover_commitment)"
inv --id "$VAULT" --source deployer -- init \
  --settlement "$SETTLEMENT" --collateral_token "$XLM" --admin "$ADMIN" \
  --solvency_image_id "$SOLVENCY_IMAGE_ID" --verifier "$VERIFIER" --protocol_fee_bps 1200 \
  --initial_cover_commitment "$INITIAL_COVER_COMMITMENT" >/dev/null

echo "→ settlement.init(image_id, instrument_id, vault, deadlines, verifier) — the default-payout path"
inv --id "$SETTLEMENT" --source deployer -- init \
  --image_id "$SETTLE_IMAGE_ID" --instrument_id "$INSTRUMENT_ID" --vault "$VAULT" \
  --deadlines "[[1,${DEADLINE:-1700000000}]]" --verifier_router "$VERIFIER" >/dev/null

cat <<EOF

── Parallar CONFIDENTIAL-COVER testnet deployment ─────────────
verifier            $VERIFIER
confidential_vault  $VAULT
settlement          $SETTLEMENT
collateral          $XLM  (native XLM SAC)
solvency image_id   $SOLVENCY_IMAGE_ID  (gates confidential buy/withdraw)
settle image_id     $SETTLE_IMAGE_ID    (gates default payouts)
initial cover commit $INITIAL_COVER_COMMITMENT
explorer            https://stellar.expert/explorer/testnet/contract/$VAULT
───────────────────────────────────────────────────────────────
Buy confidential cover: 'prove-solvency' (buy witness) → buy_protection_proven(seal, journal, premium).
Withdraw: 'prove-solvency' (withdraw witness) → withdraw_proven. See docs/RUNBOOK.md.
EOF
