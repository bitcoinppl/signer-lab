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

# Sign the CLI so macOS keeps its Keychain approval across rebuilds
[positional-arguments]
cli *args:
    cargo --locked xtask run-cli -- "$@"

bundle:
    just xtask bundle-macos

# Build and install the macOS application
release:
    just xtask bundle-macos --destination applications
