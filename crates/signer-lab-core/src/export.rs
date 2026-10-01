use bbqr::file_type::FileType;
use bitcoin::bip32::DerivationPath;
use cove_ur::{CryptoCoinInfo, CryptoHdkey, CryptoKeypath};
use cove_util::ResultExt as _;
use foundation_ur::Encoder as UrEncoder;
use minicbor::{Encoder as CborEncoder, data::Tag};
use pubport::{
    Format,
    descriptor::ScriptType,
    json::{ElectrumJson, GenericJson, Keystore, SingleSig, WasabiJson},
};
use serde::{Deserialize, Serialize};

use crate::{AccountPublicData, AddressType, LabNetwork, QrDensity, QrFrames, QrTransportError};

/// A wallet-export format supported by Cove's import paths
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    /// Coldcard generic JSON with BIP44, BIP49, and BIP84 entries
    ColdcardGenericJson,
    /// One BIP389 multipath descriptor
    MultipathDescriptor(AddressType),
    /// External and internal descriptors on separate lines
    DescriptorPair(AddressType),
    /// Electrum single-signature JSON
    ElectrumJson(AddressType),
    /// Wasabi native `SegWit` JSON
    WasabiJson,
    /// A bare account extended public key
    AccountXpub(AddressType),
    /// A BIP380 account-key expression with both branches
    Bip380KeyExpression(AddressType),
    /// Jade-compatible `crypto-account` UR data
    JadeCryptoAccount,
    /// Jade-compatible `crypto-hdkey` UR data
    JadeCryptoHdkey(AddressType),
}

/// The data carried by one export artifact
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportPayload {
    /// UTF-8 text that can be copied or stored as a file
    Text(String),
    /// Typed CBOR and its default UR frames
    Ur {
        /// UR registry type
        ur_type: &'static str,
        /// Typed CBOR payload
        cbor: Vec<u8>,
        /// Default animated UR frames used for file output
        frames: Vec<String>,
    },
}

/// A complete export with a suggested file name and media type
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportArtifact {
    /// User-selected export kind
    pub kind: ExportKind,
    /// Suggested export file name
    pub file_name: String,
    /// Media type for saved files
    pub media_type: &'static str,
    /// Export data
    pub payload: ExportPayload,
}

impl ExportArtifact {
    /// Builds scan-safe animated QR frames for this export
    ///
    /// Text exports use multipart BBQR. Existing Jade UR exports keep their required transport
    ///
    /// # Errors
    ///
    /// Returns an error when BBQR framing fails
    pub fn qr_frames(
        &self,
        transport: crate::QrTransport,
        density: QrDensity,
    ) -> Result<QrFrames, QrTransportError> {
        match (&self.payload, transport) {
            (ExportPayload::Text(text), crate::QrTransport::Bbqr) => {
                QrFrames::from_bbqr_data(text.as_bytes(), self.bbqr_file_type(), density)
            }
            (ExportPayload::Text(text), crate::QrTransport::Ur) => {
                let mut cbor = Vec::new();
                minicbor::Encoder::new(&mut cbor)
                    .bytes(text.as_bytes())
                    .map_err(|error| QrTransportError::Ur(error.to_string()))?;

                Ok(QrFrames::from_ur_data("bytes", &cbor, density))
            }
            (ExportPayload::Ur { cbor, .. }, crate::QrTransport::Bbqr) => {
                QrFrames::from_bbqr_data(cbor, FileType::Cbor, density)
            }
            (ExportPayload::Ur { ur_type, cbor, .. }, crate::QrTransport::Ur) => {
                Ok(QrFrames::from_ur_data(ur_type, cbor, density))
            }
        }
    }

    const fn bbqr_file_type(&self) -> FileType {
        match self.kind {
            ExportKind::ColdcardGenericJson
            | ExportKind::ElectrumJson(_)
            | ExportKind::WasabiJson => FileType::Json,
            ExportKind::MultipathDescriptor(_)
            | ExportKind::DescriptorPair(_)
            | ExportKind::AccountXpub(_)
            | ExportKind::Bip380KeyExpression(_) => FileType::UnicodeText,
            ExportKind::JadeCryptoAccount | ExportKind::JadeCryptoHdkey(_) => FileType::Cbor,
        }
    }
}

/// Builds public-only Coldcard and Jade exports from checked account data
pub struct WalletExporter<'a> {
    network: LabNetwork,
    accounts: &'a [AccountPublicData],
}

impl<'a> WalletExporter<'a> {
    /// Creates an exporter after checking that all required account types exist
    ///
    /// # Errors
    ///
    /// Returns an error when an account type is missing or account data belongs to different keys
    pub fn new(
        network: LabNetwork,
        accounts: &'a [AccountPublicData],
    ) -> Result<Self, ExportError> {
        let first = accounts.first().ok_or(ExportError::NoAccounts)?;
        let required = [
            AddressType::Legacy,
            AddressType::WrappedSegwit,
            AddressType::NativeSegwit,
        ];

        for address_type in required {
            let account = account(accounts, address_type)?;
            if account.master_fingerprint != first.master_fingerprint {
                return Err(ExportError::MixedFingerprints);
            }
        }

        Ok(Self { network, accounts })
    }

    /// Builds one selected export artifact
    ///
    /// # Errors
    ///
    /// Returns an error when serialization, UR encoding, or Pubport validation fails
    pub fn export(&self, kind: ExportKind) -> Result<ExportArtifact, ExportError> {
        match kind {
            ExportKind::ColdcardGenericJson => self.generic_json(),
            ExportKind::MultipathDescriptor(address_type) => {
                self.multipath_descriptor(address_type)
            }
            ExportKind::DescriptorPair(address_type) => self.descriptor_pair(address_type),
            ExportKind::ElectrumJson(address_type) => self.electrum_json(address_type),
            ExportKind::WasabiJson => self.wasabi_json(),
            ExportKind::AccountXpub(address_type) => self.account_xpub(address_type),
            ExportKind::Bip380KeyExpression(address_type) => self.key_expression(address_type),
            ExportKind::JadeCryptoAccount => self.jade_crypto_account(),
            ExportKind::JadeCryptoHdkey(address_type) => self.jade_crypto_hdkey(address_type),
        }
    }

    fn generic_json(&self) -> Result<ExportArtifact, ExportError> {
        let legacy = self.single_sig(AddressType::Legacy)?;
        let wrapped = self.single_sig(AddressType::WrappedSegwit)?;
        let native = self.single_sig(AddressType::NativeSegwit)?;
        let fingerprint = legacy.xfp.clone();
        let json = GenericJson {
            chain: Some(chain_name(self.network).to_owned()),
            xfp: fingerprint,
            xpub: None,
            bip44: Some(legacy),
            bip49: Some(wrapped),
            bip84: Some(native),
            bip86: None,
        };
        let text = serde_json::to_string_pretty(&json).map_err(ExportError::Json)?;

        validated_text(
            ExportKind::ColdcardGenericJson,
            "coldcard-export.json",
            text,
        )
    }

    fn multipath_descriptor(
        &self,
        address_type: AddressType,
    ) -> Result<ExportArtifact, ExportError> {
        let text = account(self.accounts, address_type)?
            .multipath_descriptor
            .clone();

        validated_text(
            ExportKind::MultipathDescriptor(address_type),
            descriptor_file_name(address_type, "multipath"),
            text,
        )
    }

    fn descriptor_pair(&self, address_type: AddressType) -> Result<ExportArtifact, ExportError> {
        let account = account(self.accounts, address_type)?;
        let text = format!(
            "{}\n{}",
            account.external_descriptor, account.internal_descriptor
        );

        validated_text(
            ExportKind::DescriptorPair(address_type),
            descriptor_file_name(address_type, "pair"),
            text,
        )
    }

    fn electrum_json(&self, address_type: AddressType) -> Result<ExportArtifact, ExportError> {
        let account = account(self.accounts, address_type)?;
        let json = ElectrumJson {
            seed_version: 17,
            use_encryption: false,
            wallet_type: "standard".to_owned(),
            keystore: Keystore {
                derivation: apostrophe_path(&account.account_path),
                xpub: account.account_xpub.to_string(),
                ckcc_xfp: Some(u32::from_le_bytes(account.master_fingerprint.to_bytes())),
                ckcc_xpub: None,
            },
        };
        pubport::descriptor::Descriptors::try_from(json.clone())
            .map_err_str(ExportError::Pubport)?;
        let text = serde_json::to_string_pretty(&json).map_err(ExportError::Json)?;

        validated_text(
            ExportKind::ElectrumJson(address_type),
            format!("electrum-{}.json", address_type_slug(address_type)),
            text,
        )
    }

    fn wasabi_json(&self) -> Result<ExportArtifact, ExportError> {
        let account = account(self.accounts, AddressType::NativeSegwit)?;
        let json = WasabiJson {
            cold_card_firmware_version: "Signer Lab 0.1".to_owned(),
            master_fingerprint: account.master_fingerprint.to_string(),
            ext_pub_key: account.account_xpub.to_string(),
        };
        let text = serde_json::to_string_pretty(&json).map_err(ExportError::Json)?;

        validated_text(ExportKind::WasabiJson, "wasabi.json", text)
    }

    fn account_xpub(&self, address_type: AddressType) -> Result<ExportArtifact, ExportError> {
        let text = account(self.accounts, address_type)?
            .account_xpub
            .to_string();

        validated_text(
            ExportKind::AccountXpub(address_type),
            format!("{}-account-xpub.txt", address_type_slug(address_type)),
            text,
        )
    }

    fn key_expression(&self, address_type: AddressType) -> Result<ExportArtifact, ExportError> {
        let account = account(self.accounts, address_type)?;
        let path = display_path_without_root(&account.account_path);
        let text = format!(
            "[{}/{path}]{}/0/*",
            account.master_fingerprint, account.account_xpub
        );

        validated_text(
            ExportKind::Bip380KeyExpression(address_type),
            format!("{}-key-expression.txt", address_type_slug(address_type)),
            text,
        )
    }

    fn jade_crypto_hdkey(&self, address_type: AddressType) -> Result<ExportArtifact, ExportError> {
        let account = account(self.accounts, address_type)?;
        let cbor = hdkey(account)?.to_cbor().map_err_str(ExportError::Ur)?;
        let frames = ur_frames("crypto-hdkey", &cbor, 200);

        Ok(ExportArtifact {
            kind: ExportKind::JadeCryptoHdkey(address_type),
            file_name: format!("jade-{}-crypto-hdkey.ur", address_type_slug(address_type)),
            media_type: "text/plain",
            payload: ExportPayload::Ur {
                ur_type: "crypto-hdkey",
                cbor,
                frames,
            },
        })
    }

    fn jade_crypto_account(&self) -> Result<ExportArtifact, ExportError> {
        let first = self.accounts.first().ok_or(ExportError::NoAccounts)?;
        let cbor = self.crypto_account_cbor(first.master_fingerprint.to_bytes())?;
        let frames = ur_frames("crypto-account", &cbor, 200);

        Ok(ExportArtifact {
            kind: ExportKind::JadeCryptoAccount,
            file_name: "jade-crypto-account.ur".to_owned(),
            media_type: "text/plain",
            payload: ExportPayload::Ur {
                ur_type: "crypto-account",
                cbor,
                frames,
            },
        })
    }

    fn single_sig(&self, address_type: AddressType) -> Result<SingleSig, ExportError> {
        let account = account(self.accounts, address_type)?;

        Ok(SingleSig {
            name: Some(script_type(address_type)),
            xfp: Some(account.master_fingerprint.to_string()),
            deriv: Some(apostrophe_path(&account.account_path)),
            xpub: Some(account.account_xpub.to_string()),
            descriptor: Some(account.multipath_descriptor.clone()),
            first: None,
        })
    }

    fn crypto_account_cbor(&self, fingerprint: [u8; 4]) -> Result<Vec<u8>, ExportError> {
        let mut cbor = Vec::new();
        {
            let mut encoder = CborEncoder::new(&mut cbor);
            encoder
                .tag(Tag::new(cove_ur::registry::CRYPTO_ACCOUNT))
                .map_err(|error| cbor_error(&error))?;
            encoder.map(2).map_err(|error| cbor_error(&error))?;
            encoder
                .u32(cove_ur::registry::account_keys::MASTER_FINGERPRINT)
                .map_err(|error| cbor_error(&error))?;
            encoder
                .u32(u32::from_be_bytes(fingerprint))
                .map_err(|error| cbor_error(&error))?;
            encoder
                .u32(cove_ur::registry::account_keys::OUTPUT_DESCRIPTORS)
                .map_err(|error| cbor_error(&error))?;
            encoder
                .array(self.accounts.len() as u64)
                .map_err(|error| cbor_error(&error))?;
        }

        for account in self.accounts {
            {
                let mut encoder = CborEncoder::new(&mut cbor);
                encoder
                    .tag(Tag::new(cove_ur::registry::CRYPTO_OUTPUT))
                    .map_err(|error| cbor_error(&error))?;
                for &tag in script_tags(account.address_type) {
                    encoder
                        .tag(Tag::new(tag))
                        .map_err(|error| cbor_error(&error))?;
                }
            }

            cbor.extend_from_slice(&hdkey(account)?.to_cbor().map_err_str(ExportError::Ur)?);
        }

        cove_ur::CryptoAccount::from_cbor(&cbor).map_err_str(ExportError::Ur)?;

        Ok(cbor)
    }
}

fn account(
    accounts: &[AccountPublicData],
    address_type: AddressType,
) -> Result<&AccountPublicData, ExportError> {
    accounts
        .iter()
        .find(|account| account.address_type == address_type)
        .ok_or(ExportError::MissingAccount(address_type))
}

fn hdkey(account: &AccountPublicData) -> Result<CryptoHdkey, ExportError> {
    let components = account.account_path.to_u32_vec();
    let depth = u32::try_from(components.len()).map_err(|_| ExportError::PathTooDeep)?;
    let mut hdkey = CryptoHdkey::from_xpub(&account.account_xpub);
    hdkey.use_info = Some(CryptoCoinInfo::new(Some(0), Some(1)));
    hdkey.origin = Some(CryptoKeypath::new(
        components,
        Some(account.master_fingerprint.to_bytes()),
        Some(depth),
    ));
    hdkey.name = Some(format!(
        "Signer Lab {}",
        address_type_slug(account.address_type)
    ));
    hdkey.source = Some("Signer Lab".to_owned());

    Ok(hdkey)
}

fn validated_text(
    kind: ExportKind,
    file_name: impl Into<String>,
    text: String,
) -> Result<ExportArtifact, ExportError> {
    Format::try_new_from_str(&text).map_err_str(ExportError::Pubport)?;

    Ok(ExportArtifact {
        kind,
        file_name: file_name.into(),
        media_type: if text.trim_start().starts_with('{') {
            "application/json"
        } else {
            "text/plain"
        },
        payload: ExportPayload::Text(text),
    })
}

fn ur_frames(ur_type: &str, cbor: &[u8], fragment_len: usize) -> Vec<String> {
    let mut encoder = UrEncoder::new();
    encoder.start(ur_type, cbor, fragment_len);
    let frame_count = encoder.sequence_count() as usize;

    (0..frame_count)
        .map(|_| encoder.next_part().to_string())
        .collect()
}

fn script_type(address_type: AddressType) -> ScriptType {
    match address_type {
        AddressType::Legacy => ScriptType::P2pkh,
        AddressType::WrappedSegwit => ScriptType::P2shP2wpkh,
        AddressType::NativeSegwit => ScriptType::P2wpkh,
    }
}

fn script_tags(address_type: AddressType) -> &'static [u64] {
    match address_type {
        AddressType::Legacy => &[cove_ur::registry::PAY_TO_PUBKEY_HASH],
        AddressType::WrappedSegwit => &[
            cove_ur::registry::SCRIPT_HASH,
            cove_ur::registry::WITNESS_PUBKEY_HASH,
        ],
        AddressType::NativeSegwit => &[cove_ur::registry::WITNESS_PUBKEY_HASH],
    }
}

fn chain_name(network: LabNetwork) -> &'static str {
    match network {
        LabNetwork::MutinyNet => "SIGNET",
        LabNetwork::Testnet4 => "TESTNET",
    }
}

fn apostrophe_path(path: &DerivationPath) -> String {
    format!("m/{}", display_path_without_root(path).replace('h', "'"))
}

fn display_path_without_root(path: &DerivationPath) -> String {
    path.to_string().trim_start_matches("m/").to_owned()
}

fn address_type_slug(address_type: AddressType) -> &'static str {
    match address_type {
        AddressType::Legacy => "bip44",
        AddressType::WrappedSegwit => "bip49",
        AddressType::NativeSegwit => "bip84",
    }
}

fn descriptor_file_name(address_type: AddressType, shape: &str) -> String {
    format!("{}-{shape}-descriptor.txt", address_type_slug(address_type))
}

fn cbor_error<WriteError: std::fmt::Display>(
    error: &minicbor::encode::Error<WriteError>,
) -> ExportError {
    ExportError::Cbor(error.to_string())
}

/// Wallet export construction errors
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// No account data was provided
    #[error("no public account data was provided")]
    NoAccounts,
    /// A required address account is absent
    #[error("missing {0:?} account")]
    MissingAccount(AddressType),
    /// Account data contains different master fingerprints
    #[error("account data contains different master fingerprints")]
    MixedFingerprints,
    /// A path depth did not fit the UR field
    #[error("account path is too deep")]
    PathTooDeep,
    /// JSON serialization failed
    #[error("JSON export failed: {0}")]
    Json(#[source] serde_json::Error),
    /// Pubport rejected generated public data
    #[error("generated export failed Pubport validation: {0}")]
    Pubport(String),
    /// CBOR serialization failed
    #[error("CBOR export failed: {0}")]
    Cbor(String),
    /// UR serialization or validation failed
    #[error("UR export failed: {0}")]
    Ur(String),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use foundation_ur::{Decoder, UR};

    use super::{ExportKind, ExportPayload, WalletExporter};
    use crate::{
        AccountIndex, AddressType, LabNetwork, MnemonicSecret, NetworkWallet, QrDensity,
        QrTransport, SignerId,
    };

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn every_text_export_can_be_read_by_pubport() {
        let (temp, wallet) = test_wallet();
        let accounts = wallet
            .public_accounts()
            .expect("test wallet exposes public accounts");
        let exporter = WalletExporter::new(LabNetwork::MutinyNet, &accounts)
            .expect("test wallet accounts share one fingerprint");
        let kinds = [
            ExportKind::ColdcardGenericJson,
            ExportKind::MultipathDescriptor(AddressType::NativeSegwit),
            ExportKind::DescriptorPair(AddressType::NativeSegwit),
            ExportKind::ElectrumJson(AddressType::NativeSegwit),
            ExportKind::WasabiJson,
            ExportKind::AccountXpub(AddressType::NativeSegwit),
            ExportKind::Bip380KeyExpression(AddressType::NativeSegwit),
        ];

        for kind in kinds {
            let artifact = exporter
                .export(kind)
                .unwrap_or_else(|error| panic!("{kind:?} export failed: {error}"));
            assert!(matches!(artifact.payload, ExportPayload::Text(_)));
        }

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn jade_exports_decode_with_cove_ur() {
        let (temp, wallet) = test_wallet();
        let accounts = wallet
            .public_accounts()
            .expect("test wallet exposes public accounts");
        let exporter = WalletExporter::new(LabNetwork::MutinyNet, &accounts)
            .expect("test wallet accounts share one fingerprint");

        for kind in [
            ExportKind::JadeCryptoAccount,
            ExportKind::JadeCryptoHdkey(AddressType::NativeSegwit),
        ] {
            let artifact = exporter.export(kind).expect("test Jade export succeeds");
            let ExportPayload::Ur { frames, .. } = artifact.payload else {
                panic!("expected UR frames");
            };
            let mut decoder = Decoder::default();

            for frame in frames {
                let ur = UR::parse(&frame).expect("exported UR frame parses");
                decoder.receive(ur).expect("exported UR frame decodes");
                if decoder.is_complete() {
                    break;
                }
            }

            let cbor = decoder
                .message()
                .expect("complete decoder returns a message")
                .expect("complete decoder message is present");
            match kind {
                ExportKind::JadeCryptoAccount => {
                    cove_ur::CryptoAccount::from_cbor(cbor)
                        .expect("exported crypto-account CBOR is valid");
                }
                ExportKind::JadeCryptoHdkey(_) => {
                    cove_ur::CryptoHdkey::from_cbor(cbor)
                        .expect("exported crypto-hdkey CBOR is valid");
                }
                _ => unreachable!(),
            }
        }

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn every_export_supports_selectable_animated_qr_transports() {
        let (temp, wallet) = test_wallet();
        let accounts = wallet
            .public_accounts()
            .expect("test wallet exposes public accounts");
        let exporter = WalletExporter::new(LabNetwork::MutinyNet, &accounts)
            .expect("test wallet accounts share one fingerprint");

        for kind in [
            ExportKind::ColdcardGenericJson,
            ExportKind::MultipathDescriptor(AddressType::NativeSegwit),
            ExportKind::JadeCryptoAccount,
        ] {
            let artifact = exporter.export(kind).expect("test export succeeds");
            for transport in [QrTransport::Bbqr, QrTransport::Ur] {
                let qr = artifact
                    .qr_frames(transport, QrDensity::default())
                    .expect("export supports the selected QR transport");

                assert_eq!(qr.transport, transport);
                assert!(!qr.frames.is_empty());
                if transport == QrTransport::Bbqr {
                    assert!(qr.frames.len() >= 2);
                }
            }
        }

        let _ = std::fs::remove_dir_all(temp);
    }

    fn test_wallet() -> (PathBuf, NetworkWallet) {
        let temp = std::env::temp_dir().join(format!("signer-lab-export-{}", uuid::Uuid::new_v4()));
        let secret = MnemonicSecret::parse(TEST_MNEMONIC).expect("test mnemonic is valid");
        let wallet = NetworkWallet::open(
            &temp,
            SignerId::new(),
            LabNetwork::MutinyNet,
            AccountIndex::default(),
            &secret,
        )
        .expect("test wallet opens");

        (temp, wallet)
    }
}
