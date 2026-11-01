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
