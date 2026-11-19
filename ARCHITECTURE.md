# Signer Lab architecture

Signer Lab is a macOS test application for Cove wallet imports, QR PSBT signing, and bulk test transactions.

## Ownership

`VirtualSigner` owns one BIP39 secret and account index. A `NetworkWallet` owns chain state for one virtual signer and one supported network. Each network wallet owns BIP44, BIP49, and BIP84 BDK accounts.

All BDK, Esplora, secret storage, export, signing, file-format, and operation behavior stays in `signer-lab-core`. GPUI owns window state and orchestrates typed core operations. The CLI maps typed commands to the same repository and domain APIs and writes JSON results.

PSBT file detection is content-based. The core accepts binary PSBT, Base64 PSBT, BBQR frames, and `crypto-psbt` UR frames. Wallet export files keep their original text format, with one UR frame per line for multipart data.

## QR transport

`QrFrames` owns one animated display cycle and the error-correction level its symbols need. A BBQR cycle is its split parts. A UR cycle starts with one frame for each fragment. When the payload has more than one fragment, the cycle then adds fountain parts: twice the fragment count, with at least 16, and more until every fragment appears in at least one mix. Each fountain part XORs a pseudo-random set of fragments, so a receiver that never reads one fragment's frame recovers it from a later mix. The app shows `n / total` during the fragment pass and `Mixed n / total` during the fountain parts. `foundation-ur` cannot seek, and its encoder borrows the message, so the cycle is computed once instead of streamed.

UR is case-insensitive. QR frames are uppercase so the symbol uses QR alphanumeric mode, and they render at error-correction level M. That symbol is never a larger QR version than lowercase byte mode at level L, and it is one version smaller at the default 200-byte fragments. BBQR parts are sized against level L capacity, so BBQR and SeedQR stay at L. Files and copied text use lowercase canonical UR with each fragment once.

## Persistence

The mnemonic is stored only in an owner-only plain-text file, `secrets/<signer-id>.txt`, in the data directory. Signer Lab rejects mainnet, so file storage lets CLI and UI automation run without Keychain prompts. Toasty with Turso stores signer metadata, network settings, destination reservations, and operation state. BDK stores wallet changesets and persistent outpoint locks in BDK-owned SQLite files.

Prepared operations are written before inputs are locked. Startup recovery compares nonterminal operations with BDK locks. It restores missing locks and releases locks without an owner.

## Cove dependencies

The project pins `cove-ur`, `cove-bdk`, `cove-bdk-progressive-scan`, `cove-bip39`, and `cove-util` to one exact Cove Git revision. This keeps UR registry data, descriptor exports, coin selection, progressive scans, mnemonic completion, and error conversion consistent with Cove.

`cove-types` is not used because its QR API is coupled to Cove application, Payjoin, database, and UniFFI types. `cove-tokio` is not used because the desktop application owns its Tokio runtime. BDK Esplora owns HTTP transport, so `cove-http` is not used.

## Key Teleport

The `keyteleport` crate implements the COLDCARD protocol and is the same crate Cove uses. Cove's own Key Teleport manager lives in its UniFFI application crate, so Signer Lab does not depend on it.

`signer-lab-core` parses scanned request and response text into typed packets, encrypts a signer's mnemonic for a receiver, and owns the receive session. The receiver's ephemeral private key is saved in an owner-only file under the data directory, because the CLI finishes a receive in a later process and the app can restart while the user works on the other device. Sessions expire after 24 hours. A successful import deletes the session, and a failed import keeps it so the same response can be retried.

## Network boundary

Version 1 supports only MutinyNet and Testnet4. Both use test-network address encodings. The active network is explicit and is never inferred only from an address string.
