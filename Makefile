# Parallar — contract workspace.
# The factory tests embed the vault/settlement wasm via `contractimport!`, so the
# wasm must be built before `cargo test`. `make test` enforces that ordering.
.PHONY: build test fmt clean

build:
	stellar contract build

test: build
	cargo test

fmt:
	cargo fmt --all

clean:
	cargo clean
