# Signer Lab agent instructions

Read `ARCHITECTURE.md` before changes to wallet ownership, persistence, signing, QR transport, or operation recovery.

## Workflow

- Ship production-quality changes. Model the domain before the UI.
- Keep BDK and Bitcoin behavior in `signer-lab-core`. GPUI only renders state and sends typed commands. The CLI prints stable JSON results for automation.
- Use Cove crates when their public APIs own the required behavior. Do not copy their implementations.
- Preserve unrelated changes. Use hunk staging if the user asks for a commit.
- Put ad hoc files in `_scratch/`.
- Do not add deprecated compatibility paths unless the user asks for them.

## Repository skills

- Before any dependency edit, read and use `.agents/skills/rust-crate-versions/SKILL.md`.
- After functional work and tests, read and use `.agents/skills/refactor-rust/SKILL.md` on the new code.

## Rust

- Use stable Rust and edition 2024. Do not set an MSRV unless the user asks.
- Use the Rust 2018 module layout. Do not create `mod.rs` files.
- Document every public library API.
- Use typed errors in libraries and `color-eyre` at the application boundary.
- Make invalid states hard to construct. Prefer newtypes and enums over flags and raw primitives.
- Keep functions shallow. Use guard clauses and if-let chains where they improve clarity.
- Use named imports. Do not use wildcard imports.
- Use inline variable capture in logs.
- Keep tests and test helpers out of production paths.
- Comments explain why. Start inline comments with lowercase text and do not end comments with a period.
- Put a blank line after a multiline construct before the next statement or block.

## Secrets and data

- Mnemonics live only in `FileSecretStore` files under the data directory's `secrets/`. Do not log or store them in Turso, SQLite metadata, fixtures, or snapshots.
- Mainnet is not a valid version 1 network state.
- BDK owns wallet changesets and outpoint locks. Toasty and Turso own app metadata and operation state.
- Persist operation state before wallet locks. Reconcile both stores at startup.

## Verification

- Run `just fmt`, `just clippy`, and `just test` after implementation changes.
- Test behavior and invariants. Do not add tests that only restate literals.
