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
    cargo clippy -p dominion-web --target wasm32-unknown-unknown --features web -- -D warnings

test:
    cargo test --workspace

build:
    cargo build --workspace

check: fmt-check clippy test

# Copy the agent-view@2 schema from a dominion-agents checkout.
sync-protocol agents="../dominion-agents":
    cp {{agents}}/protocol/agent-view.schema.json crates/dominion-agent/protocol/

web_out := "target/web-client"

# Build the web client (release) into target/web-client.
web-build:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf target/dx/dominion-web/release/web/public
    (cd crates/dominion-web && dx build --release --platform web --package dominion-web --features web)
    rm -rf {{web_out}}
    cp -r target/dx/dominion-web/release/web/public {{web_out}}
    echo "web client: {{web_out}}"

# Build the web client and serve it: dominion serve --web-dir target/web-client [args].
web-serve *args: web-build
    cargo run --release -p dominion-server -- serve --web-dir {{web_out}} {{args}}

# dx's dev server with hot reload, proxying /api and the asset cache to a dominion server.
web-dev server="127.0.0.1:7317" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates/dominion-web
    # dx reads proxies only from Dioxus.toml: add them for this run.
    saved=$(mktemp)
    cp Dioxus.toml "$saved"
    trap 'cp "$saved" Dioxus.toml; rm -f "$saved"' EXIT
    for path in api/ assets/images/ assets/fonts/ assets/sounds/ assets/manifest.json; do
        printf '\n[[web.proxy]]\nbackend = "http://%s/%s"\n' "{{server}}" "$path" >> Dioxus.toml
    done
    dx serve --platform web --package dominion-web --features web {{args}}
