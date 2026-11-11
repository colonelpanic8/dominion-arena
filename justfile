default:
    @just --list

fmt:
    cargo fmt --all
    alejandra -q flake.nix

fmt-check:
    cargo fmt --all -- --check
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
