use std::{fs, path::Path};

use bdk_wallet::{
    Balance, KeychainKind, PersistedWallet, SignOptions, Wallet,
    bitcoin::{
        Psbt,
        bip32::{ChildNumber, DerivationPath, Fingerprint, Xpriv, Xpub},
        key::Secp256k1,
    },
    rusqlite::Connection,
};
use bip39::Mnemonic;
use cove_bdk::descriptor_ext::DescriptorExt as _;
use cove_util::ResultExt as _;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{AccountIndex, AddressType, LabNetwork, MnemonicError, MnemonicSecret, SignerId};

type SqliteWallet = PersistedWallet<Connection>;

/// Public derivation data for one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPublicData {
    /// Account address type.
    pub address_type: AddressType,
    /// Master key fingerprint.
    pub master_fingerprint: Fingerprint,
    /// Full account derivation path.
    pub account_path: DerivationPath,
    /// Account extended public key.
    pub account_xpub: Xpub,
    /// Normalized external descriptor.
    pub external_descriptor: String,
    /// Normalized internal descriptor.
    pub internal_descriptor: String,
    /// BIP389 multipath descriptor when the pair can be combined.
    pub multipath_descriptor: String,
}

/// A read-only account snapshot for UI rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletAccountSnapshot {
    /// Account address type.
    pub address_type: AddressType,
    /// Total wallet balance.
    pub balance: Balance,
    /// Current receive address without advancing the keychain.
    pub receive_address: String,
    /// Number of unspent outputs.
    pub utxo_count: usize,
    /// Number of persistently reserved outputs.
    pub locked_utxo_count: usize,
}

/// Result details from a PSBT signing pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignReport {
    /// Inputs whose metadata matches the signer fingerprint.
    pub matched_inputs: usize,
    /// Inputs that gained a partial signature or final witness.
    pub signed_inputs: usize,
    /// Total PSBT input count.
    pub total_inputs: usize,
    /// True when BDK finalized the complete PSBT.
    pub finalized: bool,
}

/// One persisted BDK account and its database connection.
pub struct WalletAccount {
    address_type: AddressType,
    fingerprint: Fingerprint,
    wallet: SqliteWallet,
    connection: Connection,
}

impl WalletAccount {
    fn open(
        path: &Path,
        network: LabNetwork,
        address_type: AddressType,
        fingerprint: Fingerprint,
        descriptors: &SecretDescriptors,
    ) -> Result<Self, WalletError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(WalletError::Io)?;
        }

        let mut connection = Connection::open(path).map_err(WalletError::Sqlite)?;
        let loaded = Wallet::load()
            .descriptor(KeychainKind::External, Some(descriptors.external.clone()))
            .descriptor(KeychainKind::Internal, Some(descriptors.internal.clone()))
            .extract_keys()
            .check_network(network.bitcoin_network())
            .load_wallet(&mut connection)
            .map_err_str(WalletError::Bdk)?;

        let wallet = match loaded {
            Some(wallet) => wallet,
            None => Wallet::create(descriptors.external.clone(), descriptors.internal.clone())
                .network(network.bitcoin_network())
                .create_wallet(&mut connection)
                .map_err_str(WalletError::Bdk)?,
        };

        Ok(Self {
            address_type,
            fingerprint,
            wallet,
            connection,
        })
    }

    /// Returns the address type owned by this account.
    #[must_use]
    pub const fn address_type(&self) -> AddressType {
        self.address_type
    }

    /// Returns public account derivation data.
    ///
    /// # Errors
    ///
    /// Returns an error when the descriptor pair cannot be exported.
    pub fn public_data(
        &self,
        account_xpub: Xpub,
        account_path: DerivationPath,
    ) -> Result<AccountPublicData, WalletError> {
        let external = self.wallet.public_descriptor(KeychainKind::External);
        let internal = self.wallet.public_descriptor(KeychainKind::Internal);
        let multipath_descriptor =
            cove_bdk::descriptor_ext::DescriptorExt::to_multipath_string(external, internal)
                .map_err_str(WalletError::Descriptor)?;

        Ok(AccountPublicData {
            address_type: self.address_type,
            master_fingerprint: self.fingerprint,
            account_path,
            account_xpub,
            external_descriptor: external.to_normalized_string(),
            internal_descriptor: internal.to_normalized_string(),
            multipath_descriptor,
        })
    }

    /// Returns a current account snapshot without advancing the receive keychain.
    #[must_use]
    pub fn snapshot(&self) -> WalletAccountSnapshot {
        let receive_address = self
            .wallet
            .peek_address(KeychainKind::External, 0)
            .address
            .to_string();

        WalletAccountSnapshot {
            address_type: self.address_type,
            balance: self.wallet.balance(),
            receive_address,
            utxo_count: self.wallet.list_unspent().count(),
            locked_utxo_count: self.wallet.list_locked_unspent().count(),
        }
    }

    /// Reveals and persists a fresh receive address.
    ///
    /// # Errors
    ///
    /// Returns an error when BDK cannot persist the address index.
    pub fn reveal_receive_address(&mut self) -> Result<String, WalletError> {
        let address = self
            .wallet
            .reveal_next_address(KeychainKind::External)
            .address
            .to_string();
        self.persist()?;

        Ok(address)
    }

    /// Signs the matching inputs in a PSBT.
    ///
    /// # Errors
    ///
    /// Returns an error when no input matches or BDK rejects the PSBT.
    pub fn sign(
        &self,
        psbt: &mut Psbt,
        trust_witness_utxo: bool,
    ) -> Result<SignReport, WalletError> {
        let matched_inputs = matching_input_count(psbt, self.fingerprint);
        if matched_inputs == 0 {
            return Err(WalletError::NoMatchingInputs);
        }

        let signed_before = signed_input_count(psbt);
        let finalized = self
            .wallet
            .sign(
                psbt,
                SignOptions {
                    trust_witness_utxo,
                    ..SignOptions::default()
                },
            )
            .map_err_str(WalletError::Sign)?;
        let signed_inputs = signed_input_count(psbt).saturating_sub(signed_before);

        Ok(SignReport {
            matched_inputs,
            signed_inputs,
            total_inputs: psbt.inputs.len(),
            finalized,
        })
    }

    /// Persists all staged BDK changes.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` rejects the change.
    pub fn persist(&mut self) -> Result<bool, WalletError> {
        self.wallet
            .persist(&mut self.connection)
            .map_err(WalletError::Sqlite)
    }

    /// Gives read-only access to the BDK wallet.
    #[must_use]
    pub const fn bdk(&self) -> &SqliteWallet {
        &self.wallet
    }

    /// Gives mutable access to the BDK wallet for transaction planning.
    #[must_use]
    pub const fn bdk_mut(&mut self) -> &mut SqliteWallet {
        &mut self.wallet
    }
}

/// The three BDK accounts for one signer on one network.
pub struct NetworkWallet {
    /// Signer that owns these accounts.
    pub signer_id: SignerId,
    /// Chain used by these accounts.
    pub network: LabNetwork,
    fingerprint: Fingerprint,
    account_index: AccountIndex,
    root_xpriv: Xpriv,
    legacy: WalletAccount,
    wrapped_segwit: WalletAccount,
    native_segwit: WalletAccount,
}

impl NetworkWallet {
    /// Opens or creates all account databases for a signer and network.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid secret material, filesystem failures, or BDK failures.
    pub fn open(
        root: &Path,
        signer_id: SignerId,
        network: LabNetwork,
        account_index: AccountIndex,
        secret: &MnemonicSecret,
    ) -> Result<Self, WalletError> {
        let mnemonic = secret.to_mnemonic().map_err(WalletError::Mnemonic)?;
        let root_xpriv = root_xpriv(&mnemonic, network)?;
        let fingerprint = Xpub::from_priv(&Secp256k1::new(), &root_xpriv).fingerprint();
        let wallet_root = root.join(signer_id.to_string()).join(network_slug(network));

        let legacy_descriptors =
            secret_descriptors(&root_xpriv, network, account_index, AddressType::Legacy);
        let wrapped_descriptors = secret_descriptors(
            &root_xpriv,
            network,
            account_index,
            AddressType::WrappedSegwit,
        );
        let native_descriptors = secret_descriptors(
            &root_xpriv,
            network,
            account_index,
            AddressType::NativeSegwit,
        );

        let legacy = WalletAccount::open(
            &wallet_root.join("bip44.sqlite"),
            network,
            AddressType::Legacy,
            fingerprint,
            &legacy_descriptors,
        )?;
        let wrapped_segwit = WalletAccount::open(
            &wallet_root.join("bip49.sqlite"),
            network,
            AddressType::WrappedSegwit,
            fingerprint,
            &wrapped_descriptors,
        )?;
        let native_segwit = WalletAccount::open(
            &wallet_root.join("bip84.sqlite"),
            network,
            AddressType::NativeSegwit,
            fingerprint,
            &native_descriptors,
        )?;

        Ok(Self {
            signer_id,
            network,
            fingerprint,
            account_index,
            root_xpriv,
            legacy,
            wrapped_segwit,
            native_segwit,
        })
    }

    /// Returns the signer fingerprint.
    #[must_use]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Returns one account.
    #[must_use]
    pub const fn account(&self, address_type: AddressType) -> &WalletAccount {
        match address_type {
            AddressType::Legacy => &self.legacy,
            AddressType::WrappedSegwit => &self.wrapped_segwit,
            AddressType::NativeSegwit => &self.native_segwit,
        }
    }

    /// Returns one mutable account.
    #[must_use]
    pub const fn account_mut(&mut self, address_type: AddressType) -> &mut WalletAccount {
        match address_type {
            AddressType::Legacy => &mut self.legacy,
            AddressType::WrappedSegwit => &mut self.wrapped_segwit,
            AddressType::NativeSegwit => &mut self.native_segwit,
        }
    }

    /// Returns all public account data in BIP44, BIP49, and BIP84 order.
    ///
    /// # Errors
    ///
    /// Returns an error when account derivation or descriptor export fails.
    pub fn public_accounts(&self) -> Result<Vec<AccountPublicData>, WalletError> {
        [
            AddressType::Legacy,
            AddressType::WrappedSegwit,
            AddressType::NativeSegwit,
        ]
        .into_iter()
        .map(|address_type| {
            let path = account_path(self.network, self.account_index, address_type);
            let xpriv = self
                .root_xpriv
                .derive_priv(&Secp256k1::new(), &path)
                .map_err(WalletError::Bip32)?;
            let xpub = Xpub::from_priv(&Secp256k1::new(), &xpriv);

            self.account(address_type).public_data(xpub, path)
        })
        .collect()
    }

    /// Signs a PSBT with every matching account.
    ///
    /// # Errors
    ///
    /// Returns an error when no account matches or any matching signing pass fails.
    pub fn sign(
        &self,
        psbt: &mut Psbt,
        trust_witness_utxo: bool,
    ) -> Result<SignReport, WalletError> {
        let matched_inputs = matching_input_count(psbt, self.fingerprint);
        if matched_inputs == 0 {
            return Err(WalletError::NoMatchingInputs);
        }

        let signed_before = signed_input_count(psbt);
        let mut finalized = false;

        for address_type in [
            AddressType::Legacy,
            AddressType::WrappedSegwit,
            AddressType::NativeSegwit,
        ] {
            let path = account_path(self.network, self.account_index, address_type);
            if account_matches_psbt(psbt, self.fingerprint, &path) {
                finalized = self
                    .account(address_type)
                    .sign(psbt, trust_witness_utxo)?
                    .finalized;
            }
        }

        Ok(SignReport {
            matched_inputs,
            signed_inputs: signed_input_count(psbt).saturating_sub(signed_before),
            total_inputs: psbt.inputs.len(),
            finalized,
        })
    }
}

#[derive(Debug)]
struct SecretDescriptors {
    external: String,
    internal: String,
}

fn root_xpriv(mnemonic: &Mnemonic, network: LabNetwork) -> Result<Xpriv, WalletError> {
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    Xpriv::new_master(network.bitcoin_network(), seed.as_ref()).map_err(WalletError::Bip32)
}

fn secret_descriptors(
    root_xpriv: &Xpriv,
    network: LabNetwork,
    account_index: AccountIndex,
    address_type: AddressType,
) -> SecretDescriptors {
    let purpose = address_type.purpose();
    let coin_type = network.coin_type();
    let account = account_index.get();
    let external_key = format!("{root_xpriv}/{purpose}'/{coin_type}'/{account}'/0/*");
    let internal_key = format!("{root_xpriv}/{purpose}'/{coin_type}'/{account}'/1/*");
    let wrap = |key: String| match address_type {
        AddressType::Legacy => format!("pkh({key})"),
        AddressType::WrappedSegwit => format!("sh(wpkh({key}))"),
        AddressType::NativeSegwit => format!("wpkh({key})"),
    };

    SecretDescriptors {
        external: wrap(external_key),
        internal: wrap(internal_key),
    }
}

fn account_path(
    network: LabNetwork,
    account_index: AccountIndex,
    address_type: AddressType,
) -> DerivationPath {
    DerivationPath::from(vec![
        ChildNumber::from_hardened_idx(address_type.purpose()).expect("BIP purpose is valid"),
        ChildNumber::from_hardened_idx(network.coin_type()).expect("coin type is valid"),
        ChildNumber::from_hardened_idx(account_index.get()).expect("account index is checked"),
    ])
}

fn matching_input_count(psbt: &Psbt, fingerprint: Fingerprint) -> usize {
    psbt.inputs
        .iter()
        .filter(|input| {
            input
                .bip32_derivation
                .values()
                .any(|(origin, _)| *origin == fingerprint)
        })
        .count()
}

fn signed_input_count(psbt: &Psbt) -> usize {
    psbt.inputs
        .iter()
        .filter(|input| {
            !input.partial_sigs.is_empty()
                || input.final_script_sig.is_some()
                || input.final_script_witness.is_some()
        })
        .count()
}

fn account_matches_psbt(
    psbt: &Psbt,
    fingerprint: Fingerprint,
    account_path: &DerivationPath,
) -> bool {
    psbt.inputs.iter().any(|input| {
        input.bip32_derivation.values().any(|(origin, path)| {
            *origin == fingerprint && path.as_ref().starts_with(account_path.as_ref())
        })
    })
}

const fn network_slug(network: LabNetwork) -> &'static str {
    match network {
        LabNetwork::MutinyNet => "mutinynet",
        LabNetwork::Testnet4 => "testnet4",
    }
}

/// Wallet construction, persistence, or signing errors.
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// Filesystem setup failed.
    #[error("wallet filesystem operation failed: {0}")]
    Io(#[source] std::io::Error),
    /// `SQLite` persistence failed.
    #[error("wallet database operation failed: {0}")]
    Sqlite(#[source] bdk_wallet::rusqlite::Error),
    /// BDK rejected wallet construction or persisted state.
    #[error("BDK wallet operation failed: {0}")]
    Bdk(String),
    /// BIP32 derivation failed.
    #[error("BIP32 derivation failed: {0}")]
    Bip32(#[source] bitcoin::bip32::Error),
    /// Mnemonic parsing failed.
    #[error("mnemonic operation failed: {0}")]
    Mnemonic(#[source] MnemonicError),
    /// Descriptor export failed.
    #[error("descriptor export failed: {0}")]
    Descriptor(String),
    /// No PSBT input belongs to this signer.
    #[error("PSBT has no input that matches this signer")]
    NoMatchingInputs,
    /// BDK rejected PSBT signing.
    #[error("PSBT signing failed: {0}")]
    Sign(String),
}

#[cfg(test)]
mod tests {
    use super::NetworkWallet;
    use crate::{AccountIndex, AddressType, LabNetwork, MnemonicSecret, SignerId};

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn all_accounts_have_cove_normalized_public_descriptors() {
        let temp = std::env::temp_dir().join(format!("signer-lab-test-{}", uuid::Uuid::new_v4()));
        let secret = MnemonicSecret::parse(TEST_MNEMONIC).expect("test mnemonic is valid");
        let wallet = NetworkWallet::open(
            &temp,
            SignerId::new(),
            LabNetwork::MutinyNet,
            AccountIndex::default(),
            &secret,
        )
        .expect("test wallet opens");

        let accounts = wallet
            .public_accounts()
            .expect("test wallet exposes public accounts");

        assert_eq!(accounts.len(), 3);
        assert_eq!(accounts[2].address_type, AddressType::NativeSegwit);
        assert!(
            accounts
                .iter()
                .all(|account| account.multipath_descriptor.contains("/<0;1>/*"))
        );
        assert!(
            accounts
                .iter()
                .all(|account| !account.external_descriptor.contains('\''))
        );

        let _ = std::fs::remove_dir_all(temp);
    }
}
