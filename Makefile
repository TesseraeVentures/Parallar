# Parallar — contract workspace.
# The factory tests embed the vault/settlement wasm via `contractimport!`, so the
# wasm must be built before `cargo test`. `make test` enforces that ordering.
.PHONY: build test fmt clean demo reset frontend verify

build:
	stellar contract build

test: build
	cargo test

# Fresh-clone, one-command walkthrough of the full verified scenario (no testnet / no live
# proof needed — uses the committed real-proof fixture). See demo.sh.
demo:
	./demo.sh

# Return a clone to a clean pre-demo state: cargo clean for BOTH workspaces + drop
# throwaway artifacts (never the committed fixtures). The counterpart to demo. See reset.sh.
reset:
	./reset.sh

# Serve the live testnet console (frontend/). Reads deployments/testnet.json + live on-chain
# state from testnet RPC. Served from the repo root so the relative paths resolve.
frontend:
	@echo "→ open http://localhost:8765/frontend/" && python3 -m http.server 8765

# Independently verify the live settlement + the determination from public artifacts. See VERIFY.md.
verify:
	./scripts/verify.sh

fmt:
	cargo fmt --all

clean:
	cargo clean
