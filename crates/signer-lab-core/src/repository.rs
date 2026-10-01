use std::path::{Path, PathBuf};

use jiff::Timestamp;

use crate::{
    AccountIndex, LabNetwork, LabStore, MnemonicSecret, NetworkWallet, OperationId, SecretStore,
    SignerId, SignerName, VirtualSigner,
};

mod bulk;

pub use bulk::{
    BulkExecutionError, BulkExecutionReport, RecoverableBulkOperation, ReservationReconciliation,
};

/// A newly generated signer and the recovery words shown once to the user
pub struct CreatedSigner {
    /// Persisted public signer metadata
    pub signer: VirtualSigner,
    /// Generated mnemonic stored in the configured secret store
    pub secret: MnemonicSecret,
}

/// Owns public metadata, secret storage, and BDK wallet paths
pub struct SignerRepository<S> {
    store: LabStore,
    secret_store: S,
    wallet_root: PathBuf,
}

impl<S: SecretStore> SignerRepository<S> {
    /// Creates a repository from its three explicit storage owners
    #[must_use]
    pub fn new(store: LabStore, secret_store: S, wallet_root: impl Into<PathBuf>) -> Self {
        Self {
            store,
            secret_store,
            wallet_root: wallet_root.into(),
        }
    }

    /// Returns the BDK wallet database root
    #[must_use]
    pub fn wallet_root(&self) -> &Path {
        &self.wallet_root
    }

    /// Lists saved virtual signers
    ///
    /// # Errors
    ///
    /// Returns an error for database failures or corrupt metadata
    pub async fn signers(&mut self) -> Result<Vec<VirtualSigner>, RepositoryError> {
        self.store.signers().await.map_err(RepositoryError::Storage)
    }

    /// Changes the user-visible name without changing signer identity or secret data
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid name or metadata storage failure
    pub async fn rename(
        &mut self,
        signer: &VirtualSigner,
        name: impl Into<String>,
    ) -> Result<VirtualSigner, RepositoryError> {
        let mut renamed = signer.clone();
        renamed.name = SignerName::new(name).map_err(RepositoryError::SignerName)?;
        self.store
            .save_signer(&renamed)
            .await
            .map_err(RepositoryError::Storage)?;

        Ok(renamed)
    }

    /// Generates, stores, and returns a new virtual signer
    ///
    /// # Errors
    ///
    /// Returns an error for invalid inputs, entropy generation, Keychain, or metadata storage
    pub async fn generate(
        &mut self,
        name: impl Into<String>,
        account_index: u32,
        word_count: usize,
    ) -> Result<CreatedSigner, RepositoryError> {
        let secret = MnemonicSecret::generate(word_count).map_err(RepositoryError::Mnemonic)?;
        let signer = self.create(name, account_index, &secret).await?;

        Ok(CreatedSigner { signer, secret })
    }

    /// Imports and stores recovery words or standard `SeedQR` digits as a virtual signer
    ///
    /// # Errors
    ///
    /// Returns an error for invalid import data, metadata, Keychain, or database failures
    pub async fn import(
        &mut self,
        name: impl Into<String>,
        account_index: u32,
        phrase: &str,
    ) -> Result<VirtualSigner, RepositoryError> {
        let secret = MnemonicSecret::parse_import(phrase).map_err(RepositoryError::Import)?;

        self.create(name, account_index, &secret).await
    }

    /// Stores a validated mnemonic as a virtual signer
    ///
    /// This boundary supports binary formats such as compact `SeedQR` without converting secret
    /// bytes into an intermediate text format
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, Keychain, or database failures
    pub async fn import_secret(
        &mut self,
        name: impl Into<String>,
        account_index: u32,
        secret: MnemonicSecret,
    ) -> Result<VirtualSigner, RepositoryError> {
        self.create(name, account_index, &secret).await
    }

    /// Loads the secret and opens all BDK accounts for one network
    ///
    /// # Errors
    ///
    /// Returns an error for Keychain, mnemonic, filesystem, or BDK failures
    pub fn open_wallet(
        &self,
        signer: &VirtualSigner,
        network: LabNetwork,
    ) -> Result<NetworkWallet, RepositoryError> {
        let secret = self
            .secret_store
            .load(signer.id)
            .map_err(RepositoryError::SecretStore)?;

        NetworkWallet::open(
            &self.wallet_root,
            signer.id,
            network,
            signer.account_index,
            &secret,
        )
        .map_err(RepositoryError::Wallet)
    }

    /// Loads recovery words for an explicit reveal action
    ///
    /// # Errors
    ///
    /// Returns an error when Keychain access or mnemonic validation fails
    pub fn reveal_secret(&self, signer_id: SignerId) -> Result<MnemonicSecret, RepositoryError> {
        self.secret_store
            .load(signer_id)
            .map_err(RepositoryError::SecretStore)
    }

    /// Removes public metadata and the Keychain secret
    ///
    /// BDK database files are retained so a failed delete remains recoverable
    ///
    /// # Errors
    ///
    /// Returns an error when Keychain or database deletion fails
    pub async fn delete(&mut self, signer_id: SignerId) -> Result<(), RepositoryError> {
        self.secret_store
            .delete(signer_id)
            .map_err(RepositoryError::SecretStore)?;
        self.store
            .delete_signer(signer_id)
            .await
            .map_err(RepositoryError::Storage)
    }

    async fn create(
        &mut self,
        name: impl Into<String>,
        account_index: u32,
        secret: &MnemonicSecret,
    ) -> Result<VirtualSigner, RepositoryError> {
        let signer = VirtualSigner {
            id: SignerId::new(),
            name: SignerName::new(name).map_err(RepositoryError::SignerName)?,
            fingerprint: secret.fingerprint().map_err(RepositoryError::Mnemonic)?,
            account_index: AccountIndex::new(account_index)
                .map_err(RepositoryError::AccountIndex)?,
            created_at: Timestamp::now(),
        };
        self.secret_store
            .save(signer.id, secret)
            .map_err(RepositoryError::SecretStore)?;

        if let Err(error) = self.store.save_signer(&signer).await {
            let rollback = self.secret_store.delete(signer.id).err();
            return Err(RepositoryError::MetadataSave {
                source: error,
                rollback,
            });
        }

        Ok(signer)
    }
}

/// Signer repository errors
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    /// Signer name validation failed
    #[error("invalid signer name: {0}")]
    SignerName(#[source] crate::SignerNameError),
    /// Account-index validation failed
    #[error("invalid account index: {0}")]
    AccountIndex(#[source] crate::AccountIndexError),
    /// Mnemonic generation, parsing, or fingerprinting failed
    #[error("mnemonic operation failed: {0}")]
    Mnemonic(#[source] crate::MnemonicError),
    /// Recovery-word or standard `SeedQR` import failed
    #[error("signer import failed: {0}")]
    Import(#[source] crate::MnemonicImportError),
    /// Secret storage failed
    #[error("secret storage failed: {0}")]
    SecretStore(#[source] crate::SecretStoreError),
    /// Public metadata storage failed
    #[error("metadata storage failed: {0}")]
    Storage(#[source] crate::StorageError),
    /// Metadata storage failed after Keychain storage
    #[error("metadata storage failed: {source}")]
    MetadataSave {
        /// Metadata write failure
        source: crate::StorageError,
        /// Keychain rollback failure, when rollback also failed
        rollback: Option<crate::SecretStoreError>,
    },
    /// Another recoverable bulk operation already owns wallet inputs
    #[error("bulk operation {0} is already active for this network")]
    ActiveBulkOperation(OperationId),
    /// BDK wallet setup failed
    #[error("wallet open failed: {0}")]
    Wallet(#[source] crate::WalletError),
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use bdk_wallet::{
        KeychainKind,
        bitcoin::{Amount, Transaction, TxOut, absolute::LockTime, transaction::Version},
    };

    use super::SignerRepository;
    use crate::{
        AddressType, BulkPlanner, BulkRequest, DestinationScripts, LabFeeRate, LabNetwork,
        LabStore, MnemonicSecret, SecretStore, SecretStoreError, SignerId,
    };

    #[derive(Default)]
    struct MemorySecretStore(Mutex<HashMap<SignerId, String>>);

    impl SecretStore for MemorySecretStore {
        fn save(
            &self,
            signer_id: SignerId,
            secret: &MnemonicSecret,
        ) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .expect("test secret store lock is not poisoned")
                .insert(signer_id, secret.expose_phrase().to_owned());
            Ok(())
        }

        fn load(&self, signer_id: SignerId) -> Result<MnemonicSecret, SecretStoreError> {
            let phrase = self
                .0
                .lock()
                .expect("test secret store lock is not poisoned")
                .get(&signer_id)
                .cloned()
                .expect("test signer secret exists");
            MnemonicSecret::parse(&phrase).map_err(SecretStoreError::InvalidSecret)
        }

        fn delete(&self, signer_id: SignerId) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .expect("test secret store lock is not poisoned")
                .remove(&signer_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn imported_signer_opens_both_test_networks() {
        let temp =
            std::env::temp_dir().join(format!("signer-lab-repository-{}", uuid::Uuid::new_v4()));
        let store = LabStore::create(temp.join("metadata.db"))
            .await
            .expect("test metadata store creates");
        let mut repository =
            SignerRepository::new(store, MemorySecretStore::default(), temp.join("wallets"));
        let signer = repository
            .import(
                "Imported",
                0,
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .await
            .expect("test signer import succeeds");

        for network in [LabNetwork::MutinyNet, LabNetwork::Testnet4] {
            let wallet = repository
                .open_wallet(&signer, network)
                .expect("test wallet opens");
            assert_eq!(wallet.network, network);
        }

        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn rename_preserves_signer_identity_and_wallet_access() {
        let temp = std::env::temp_dir().join(format!("signer-lab-rename-{}", uuid::Uuid::new_v4()));
        let store = LabStore::create(temp.join("metadata.db"))
            .await
            .expect("test metadata store creates");
        let mut repository =
            SignerRepository::new(store, MemorySecretStore::default(), temp.join("wallets"));
        let signer = repository
            .import(
                "Before",
                0,
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .await
            .expect("test signer import succeeds");

        let renamed = repository
            .rename(&signer, "  After  ")
            .await
            .expect("test signer rename succeeds");

        assert_eq!(renamed.id, signer.id);
        assert_eq!(renamed.fingerprint, signer.fingerprint);
        assert_eq!(renamed.name.as_str(), "After");
        assert_eq!(
            repository.signers().await.expect("renamed signer reloads")[0]
                .name
                .as_str(),
            "After"
        );
        repository
            .open_wallet(&renamed, LabNetwork::MutinyNet)
            .expect("renamed signer keeps its secret and wallet access");

        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn prepared_bulk_operation_recovers_and_cancel_releases_inputs() {
        let temp =
            std::env::temp_dir().join(format!("signer-lab-recovery-{}", uuid::Uuid::new_v4()));
        let store = LabStore::create(temp.join("metadata.db"))
            .await
            .expect("test metadata store creates");
        let mut repository =
            SignerRepository::new(store, MemorySecretStore::default(), temp.join("wallets"));
        let signer = repository
            .import(
                "Bulk signer",
                0,
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .await
            .expect("test signer import succeeds");
        let mut wallet = repository
            .open_wallet(&signer, LabNetwork::MutinyNet)
            .expect("test wallet opens");
        let account = wallet.account_mut(AddressType::NativeSegwit);
        let receive = account
            .bdk()
            .peek_address(KeychainKind::External, 0)
            .address;
        let funding = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(50_000),
                script_pubkey: receive.script_pubkey(),
            }],
        };
        account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
        account.persist().expect("funded test wallet persists");
        let destinations = DestinationScripts::new(vec![receive.script_pubkey(); 2])
            .expect("test destinations are valid");
        let request = BulkRequest::fan_out(Amount::from_sat(2_000), destinations)
            .expect("test fan-out request is valid");
        let plan = BulkPlanner::new(account)
            .plan(&request, LabFeeRate::DEFAULT)
            .expect("funded test wallet can plan fan-out");
        assert_eq!(account.snapshot().locked_utxo_count, 0);
        drop(wallet);

        let operation = repository
            .prepare_bulk_operation(signer.id, LabNetwork::MutinyNet, &plan)
            .await
            .expect("test bulk operation saves");
        let reconciliation = repository
            .reconcile_bulk_reservations(&signer, LabNetwork::MutinyNet)
            .await
            .expect("test reservations reconcile");
        assert_eq!(reconciliation.active_reservations, 1);
        assert_eq!(reconciliation.orphaned_locks_released, 0);
        let wallet = repository
            .open_wallet(&signer, LabNetwork::MutinyNet)
            .expect("test wallet opens");
        assert_eq!(
            wallet
                .account(AddressType::NativeSegwit)
                .snapshot()
                .locked_utxo_count,
            1
        );
        drop(wallet);
        let recovered = repository
            .recoverable_bulk_operation(signer.id, LabNetwork::MutinyNet)
            .await
            .expect("test recoverable operation loads")
            .expect("test operation is present");
        assert_eq!(recovered.operation_id, operation.lifecycle.id);
        assert_eq!(recovered.plan, plan);
        assert!(
            repository
                .recoverable_bulk_operation(signer.id, LabNetwork::Testnet4)
                .await
                .expect("test operation lookup succeeds")
                .is_none()
        );

        repository
            .cancel_bulk_operation(&signer, LabNetwork::MutinyNet, operation.lifecycle.id)
            .await
            .expect("test bulk operation cancels");
        let wallet = repository
            .open_wallet(&signer, LabNetwork::MutinyNet)
            .expect("test wallet opens");
        assert_eq!(
            wallet
                .account(AddressType::NativeSegwit)
                .snapshot()
                .locked_utxo_count,
            0
        );
        assert!(
            repository
                .recoverable_bulk_operation(signer.id, LabNetwork::MutinyNet)
                .await
                .expect("test operation lookup succeeds")
                .is_none()
        );

        let _ = std::fs::remove_dir_all(temp);
    }
}
