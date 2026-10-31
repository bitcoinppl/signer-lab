# Signer Lab

Signer Lab is a macOS desktop test tool for Cove wallet import, PSBT signing, and UTXO testing. It replaces a Coldcard or Blockstream Jade during test flows. It does not emulate device screens, firmware, USB, or serial protocols.

Version 1 supports MutinyNet and Testnet4 only. Do not import a production mnemonic.

## Features

- Create a 12-word or 24-word virtual signer, or import BIP39 recovery words
- Use the dedicated SeedQR screen to import standard numeric text or standard and compact QR images
- Export recovery words as a standard SeedQR, then copy the digits or save the QR as SVG
- Store recovery words in macOS Keychain
- Open BIP44, BIP49, and BIP84 BDK wallets for MutinyNet or Testnet4
- Sync all accounts with Cove's progressive Esplora scanner
- Export Coldcard generic JSON, descriptors, Electrum JSON, Wasabi JSON, XPUBs, BIP380 keys, and Jade UR data
- Save wallet exports through the native file dialog
- Scan BBQR or `crypto-psbt` UR data with the camera, or paste PSBT and QR text
- Open binary, Base64, BBQR, or UR PSBT files through the native file dialog
- Review inputs, outputs, signer matches, change, and fees before signing
- Return a signed PSBT as BBQR or `crypto-psbt` UR QR frames
- Plan, save, sign, and broadcast fan-out, independent batch, and consolidation transactions
- Derive bulk destinations from an address, XPUB, ranged descriptor, or another saved signer
- Use exact fee rates from 0.1 sat/vB
- Send or receive recovery words with COLDCARD Key Teleport, for example to or from Cove

Key Teleport receive sessions are saved in `key-teleport/receive-session.json` in the data directory with owner-only permissions. The app and the CLI share the session, it expires after 24 hours, and it is deleted after a successful import. Only BIP39 recovery words can be received; extended private keys and secure notes are rejected.

Bulk plans are saved before BDK locks their inputs. On startup, Signer Lab restores active locks and releases locks that have no saved operation. A signed plan is saved before relay, so an Esplora policy rejection does not remove the signed PSBT.

## Development

```sh
just run
```

This command builds, signs, and opens the application bundle. The stable signature lets macOS keep the Keychain approval across rebuilds. An existing Keychain item can ask once when you first switch from an unsigned or ad-hoc build.

Create the bundle without opening it with:

```sh
just bundle
```

The local release is written to `_scratch/release/Signer Lab.app`. The build uses an installed Apple Development identity when one is available. Set `SIGNER_LAB_CODESIGN_IDENTITY=-` to request an ad-hoc signature.

Build and install the release in `/Applications` with:

```sh
just release
```

Verification:

```sh
just fmt
just clippy
just test
```

The application stores mnemonics in macOS Keychain. It stores Toasty metadata and BDK wallet databases in the macOS application data directory.

## CLI automation

The CLI uses the same data directory, Keychain secrets, BDK wallets, exports, signing rules, and recoverable bulk operations as the desktop app. Command results use JSON on standard output. Errors use standard error and a nonzero exit status.

`just cli` builds the debug CLI and signs it with the same identity as the application bundle, so macOS keeps an Always Allow Keychain approval across rebuilds. The CLI has its own identity, so it asks once separately from the app.

```sh
# List virtual signers
just cli signer list

# Import recovery words or standard SeedQR digits without a process argument
just cli signer import \
  --name "Test signer" \
  --mnemonic-file ./test-mnemonic.txt

# Export standard SeedQR digits as JSON
just cli signer seed-qr --signer "$SIGNER_ID"

# Save a Coldcard import file
just cli wallet export \
  --signer "$SIGNER_ID" \
  --network mutinynet \
  --output ./coldcard.json \
  coldcard-json

# Review and sign a PSBT file
just cli psbt review \
  --signer "$SIGNER_ID" \
  --network testnet4 \
  --input ./unsigned.psbt

just cli psbt sign \
  --signer "$SIGNER_ID" \
  --network testnet4 \
  --input ./unsigned.psbt \
  --output ./signed.ur \
  --format ur

# Key Teleport: encrypt recovery words for a receiver QR and code
just cli key-teleport send \
  --signer "$SIGNER_ID" \
  --request 'B$2R0100…' \
  --code 12345678

# Key Teleport: show a receiver request, then import the sender's response
just cli key-teleport receive start
just cli key-teleport receive finish \
  --name "Teleported signer" \
  --response 'B$2S0100…' \
  --password ABCD2345
```

Key Teleport commands accept BBQR text or `keyteleport.com` links. `send` prints the response QR text, its link, and the password the receiver types. `receive start` replaces any earlier session; `receive show` prints the active request again, and `receive cancel` deletes it.

Use `just cli --help` and the nested command help for all wallet and bulk commands. Generated or revealed recovery words are printed only for explicit signer commands. Do not use production recovery words.
