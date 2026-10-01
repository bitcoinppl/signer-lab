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

cli *args:
    cargo run -p signer-lab-cli -- {{args}}

bundle:
    just xtask bundle-macos

# Build and install the macOS application
release:
    just xtask bundle-macos --destination applications
