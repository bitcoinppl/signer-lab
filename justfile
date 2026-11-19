set dotenv-load := false

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features

check:
    cargo check --workspace --all-targets

xtask *args:
    cargo --locked xtask {{args}}

run:
    just bundle
    open -n '_scratch/release/Signer Lab.app'

# Positional arguments keep quoted values such as signer names intact
[positional-arguments]
cli *args:
    cargo run --locked -p signer-lab-cli -- "$@"

bundle:
    just xtask bundle-macos

# Build and install the macOS application
release:
    just xtask bundle-macos --destination applications

# Render the README screenshots from offline fixtures into docs/screenshots
readme-screenshots:
    #!/usr/bin/env bash
    set -euo pipefail
    states=(wallet sign-signed export-jade-ur bulk-planned key-teleport-send)
    shots="$(mktemp -d)"
    trap 'rm -rf "$shots"' EXIT
    # prefixes also match sibling states such as wallet-rename, so only the listed names are kept
    SHOTS_DIR="$shots" SHOTS_ONLY="$(IFS=,; echo "${states[*]}")" \
        cargo test --locked -p signer-lab-gpui --all-features --test screenshots
    mkdir -p docs/screenshots
    for state in "${states[@]}"; do
        for mode in light dark; do
            sips --resampleWidth 1600 "$shots/$state-$mode.png" \
                --out "docs/screenshots/$state-$mode.png" >/dev/null
        done
    done
