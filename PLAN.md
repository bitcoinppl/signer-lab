---
status: implemented
---

# Signer Lab implementation plan

Build a macOS-first Rust GPUI application with a reusable BDK core. Support MutinyNet and Testnet4 virtual signers, Cove-compatible Coldcard and Jade exports, BBQR and UR PSBT signing, and bulk fan-out, batch, and consolidation operations.

Use the pinned reusable Cove crates for UR data, descriptor exports, coin selection, progressive BDK scans, and mnemonic completion. Store secrets in owner-only files, app state with Toasty and Turso, and wallet state with BDK SQLite persistence.

Require transaction review before signing. Support exact fee rates down to 0.1 sat/vB and preserve signed transactions when relay policy rejects them.

The initial implementation includes camera QR input, rendered QR output, full BIP44/BIP49/BIP84 export choices, wallet synchronization, saved bulk-operation recovery, input-lock reconciliation, and fixed-address, XPUB, descriptor, or saved-signer bulk destinations.

The implemented automation layer includes a JSON CLI over the same repository. It supports signer management, wallet synchronization and exports, PSBT file review and signing, and recoverable bulk operations. The desktop app includes native export-save and PSBT-open dialogs.
