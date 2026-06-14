# Parallar — contract workspace.
# The factory tests embed the vault/settlement wasm via `contractimport!`, so the
# wasm must be built before `cargo test`. `make test` enforces that ordering.
.PHONY: build test fmt clean demo

build:
	stellar contract build

test: build
	cargo test

# Fresh-clone, one-command walkthrough of the full verified scenario (no testnet / no live
# proof needed — uses the committed real-proof fixture). See demo.sh.
demo:
	./demo.sh

fmt:
	cargo fmt --all

clean:
	cargo clean
