default:
    @just --list

# The set modules are declared by the `sets!` macro, where `cargo fmt` does not find them.
sets := "crates/dominion-engine/src/cards/*.rs"

fmt:
    cargo fmt --all
    rustfmt --edition 2024 {{sets}}
    alejandra -q flake.nix

fmt-check:
    cargo fmt --all -- --check
    rustfmt --edition 2024 --check {{sets}}
    alejandra -q --check flake.nix

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

build:
    cargo build --workspace

check: fmt-check clippy test

# Copy the agent-view@2 schema from a dominion-agents checkout.
sync-protocol agents="../dominion-agents":
    cp {{agents}}/protocol/agent-view.schema.json crates/dominion-agent/protocol/
