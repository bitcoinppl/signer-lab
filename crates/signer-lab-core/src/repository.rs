use std::path::{Path, PathBuf};

use jiff::Timestamp;

use crate::{
    AccountIndex, LabNetwork, LabStore, MnemonicSecret, NetworkWallet, OperationId, SecretStore,
    SecretStoreError, SignerId, SignerName, StorageError, VirtualSigner,
};

mod bulk;
mod planning;

pub use planning::{
    BulkDestinationSpec, BulkPlanOperationError, BulkShape, InputReservation, OutputCount,
    PlanBulk, PlannedBulkOperation,
};

pub use bulk::{
    BulkExecutionError, BulkExecutionReport, CurrentBulkOperation, ReservationReconciliation,
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
    /// Returns an error for invalid inputs, entropy generation, secret storage, or metadata storage
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
    /// Returns an error for invalid import data, metadata, secret storage, or database failures
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
    /// Returns an error for invalid metadata, secret storage, or database failures
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
    /// Returns an error for secret storage, mnemonic, filesystem, or BDK failures
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
    /// Returns an error when secret storage access or mnemonic validation fails
    pub fn reveal_secret(&self, signer_id: SignerId) -> Result<MnemonicSecret, RepositoryError> {
        self.secret_store
            .load(signer_id)
            .map_err(RepositoryError::SecretStore)
    }

    /// Removes public metadata and the stored secret
    ///
    /// BDK database files are retained so a failed delete remains recoverable
    ///
    /// # Errors
    ///
    /// Returns an error when secret or database deletion fails
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
    SecretStore(#[source] SecretStoreError),
    /// Public metadata storage failed
    #[error("metadata storage failed: {0}")]
    Storage(#[source] StorageError),
    /// Metadata storage failed after the secret was saved
    #[error("metadata storage failed: {source}")]
    MetadataSave {
        /// Metadata write failure
        source: StorageError,
        /// Secret rollback failure, when rollback also failed
        rollback: Option<SecretStoreError>,
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
    use bitcoin::OutPoint;
    use jiff::Timestamp;
    use std::{collections::HashMap, sync::Mutex};

    use bdk_wallet::{
        KeychainKind,
        bitcoin::{Amount, Transaction, TxOut, absolute::LockTime, transaction::Version},
    };

    use super::SignerRepository;
    use crate::{
        AddressType, BulkExecutionError, BulkPlan, BulkPlanner, BulkRequest, CurrentBulkOperation,
        DestinationScripts, LabFeeRate, LabNetwork, LabStore, MnemonicSecret, OperationState,
        SecretStore, SecretStoreError, SignerId, StoredOperation, VirtualSigner,
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

    struct BulkFixture {
        repository: SignerRepository<MemorySecretStore>,
        signer: VirtualSigner,
        plan: BulkPlan,
        root: std::path::PathBuf,
    }

    impl BulkFixture {
        async fn new() -> Self {
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

            Self {
                repository,
                signer,
                plan,
                root: temp,
            }
        }

        fn lock_count(&self) -> usize {
            self.repository
                .open_wallet(&self.signer, LabNetwork::MutinyNet)
                .expect("wallet opens")
                .account(AddressType::NativeSegwit)
                .snapshot()
                .locked_utxo_count
        }

        async fn failed(&mut self, error: &str, legacy: bool) -> StoredOperation {
            let mut operation = self.prepare().await;
            operation.lifecycle.start(Timestamp::now()).expect("start");
            operation
                .lifecycle
                .fail(0, error, Timestamp::now())
                .expect("fail");

            if legacy {
                operation
                    .plan
                    .as_object_mut()
                    .expect("plan object")
                    .remove("acknowledged_at");
            }

            self.repository
                .store
                .save_operation(&operation)
                .await
                .expect("save failure");
            operation
        }

        async fn prepare(&mut self) -> StoredOperation {
            self.repository
                .prepare_bulk_operation(self.signer.id, LabNetwork::MutinyNet, &self.plan)
                .await
                .expect("bulk operation saves")
        }
    }

    impl Drop for BulkFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn prepared_bulk_operation_recovers_and_cancel_releases_inputs() {
        let mut fixture = BulkFixture::new().await;
        let operation = fixture.prepare().await;
        let mut wallet = fixture
            .repository
            .open_wallet(&fixture.signer, LabNetwork::MutinyNet)
            .expect("wallet opens");

        let account = wallet.account_mut(AddressType::NativeSegwit);
        let orphan = OutPoint::default();
        account.bdk_mut().lock_outpoint(orphan);
        account.persist().expect("orphan lock persists");
        drop(wallet);

        let reconciliation = fixture
            .repository
            .reconcile_bulk_reservations(&fixture.signer, LabNetwork::MutinyNet)
            .await
            .expect("reservations reconcile");

        assert_eq!(reconciliation.active_reservations, 1);
        assert_eq!(reconciliation.orphaned_locks_released, 1);
        assert_eq!(fixture.lock_count(), 1);
        let wallet = fixture
            .repository
            .open_wallet(&fixture.signer, LabNetwork::MutinyNet)
            .expect("wallet opens");

        assert!(
            !wallet
                .account(AddressType::NativeSegwit)
                .bdk()
                .is_outpoint_locked(orphan)
        );

        drop(wallet);
        let recovered = fixture
            .repository
            .current_bulk_operation(fixture.signer.id, LabNetwork::MutinyNet)
            .await
            .expect("recoverable operation loads")
            .expect("operation is present");

        let CurrentBulkOperation::Active { operation_id, plan } = recovered else {
            panic!("active operation");
        };
        assert_eq!(operation_id, operation.lifecycle.id);
        assert_eq!(plan, fixture.plan);
        assert!(
            fixture
                .repository
                .current_bulk_operation(fixture.signer.id, LabNetwork::Testnet4)
                .await
                .expect("operation lookup succeeds")
                .is_none()
        );

        fixture
            .repository
            .cancel_bulk_operation(
                &fixture.signer,
                LabNetwork::MutinyNet,
                operation.lifecycle.id,
            )
            .await
            .expect("operation cancels");

        assert_eq!(fixture.lock_count(), 0);
        assert!(
            fixture
                .repository
                .current_bulk_operation(fixture.signer.id, LabNetwork::MutinyNet)
                .await
                .expect("operation lookup succeeds")
                .is_none()
        );
    }

    #[tokio::test]
    async fn rejected_bulk_actions_do_not_change_locks() {
        let mut fixture = BulkFixture::new().await;
        let mut operation = fixture.prepare().await;
        fixture
            .repository
            .reconcile_bulk_reservations(&fixture.signer, LabNetwork::MutinyNet)
            .await
            .expect("reservations reconcile");

        operation
            .lifecycle
            .start(Timestamp::now())
            .expect("operation starts");

        fixture
            .repository
            .store
            .save_operation(&operation)
            .await
            .expect("running operation saves");

        assert!(matches!(
            fixture
                .repository
                .cancel_bulk_operation(
                    &fixture.signer,
                    LabNetwork::MutinyNet,
                    operation.lifecycle.id
                )
                .await,
            Err(BulkExecutionError::InvalidState)
        ));

        assert_eq!(fixture.lock_count(), 1);

        operation.lifecycle.state = OperationState::Prepared;
        fixture
            .repository
            .store
            .save_operation(&operation)
            .await
            .expect("prepared operation saves");

        fixture
            .repository
            .cancel_bulk_operation(
                &fixture.signer,
                LabNetwork::MutinyNet,
                operation.lifecycle.id,
            )
            .await
            .expect("operation cancels");

        assert!(matches!(
            fixture
                .repository
                .execute_bulk_operation(
                    &fixture.signer,
                    LabNetwork::MutinyNet,
                    operation.lifecycle.id
                )
                .await,
            Err(BulkExecutionError::InvalidState)
        ));

        assert_eq!(fixture.lock_count(), 0);
    }

    #[tokio::test]
    async fn failed_operations_remain_visible_until_acknowledged() {
        let mut fixture = BulkFixture::new().await;
        let older = fixture.failed("first failure", true).await;
        let newer = fixture.failed("second failure", false).await;
        let current = fixture
            .repository
            .current_bulk_operation(fixture.signer.id, LabNetwork::MutinyNet)
            .await
            .expect("current");

        assert_eq!(
            current,
            Some(CurrentBulkOperation::Failed {
                operation_id: newer.lifecycle.id,
                plan: fixture.plan.clone(),
                failed_index: 0,
                accepted_txids: Vec::new(),
                error: "second failure".to_owned(),
            })
        );

        assert!(
            fixture
                .repository
                .current_bulk_operation(fixture.signer.id, LabNetwork::Testnet4)
                .await
                .expect("other network")
                .is_none()
        );

        fixture
            .repository
            .acknowledge_failed_bulk_operation(fixture.signer.id, newer.lifecycle.id)
            .await
            .expect("acknowledge newest");

        let result = fixture
            .repository
            .current_bulk_operation(fixture.signer.id, LabNetwork::MutinyNet)
            .await
            .expect("next failure");

        assert!(matches!(
            result,
            Some(CurrentBulkOperation::Failed { operation_id, .. }) if operation_id == older.lifecycle.id
        ));

        fixture
            .repository
            .acknowledge_failed_bulk_operation(fixture.signer.id, older.lifecycle.id)
            .await
            .expect("acknowledge legacy");

        assert!(
            fixture
                .repository
                .current_bulk_operation(fixture.signer.id, LabNetwork::MutinyNet)
                .await
                .expect("no failures")
                .is_none()
        );
    }

    #[tokio::test]
    async fn acknowledgement_preserves_failed_state_and_first_timestamp() {
        let mut fixture = BulkFixture::new().await;
        let failed = fixture.failed("relay failure", true).await;
        fixture
            .repository
            .acknowledge_failed_bulk_operation(fixture.signer.id, failed.lifecycle.id)
            .await
            .expect("acknowledge");

        let saved = fixture
            .repository
            .store
            .operation(failed.lifecycle.id)
            .await
            .expect("load")
            .expect("saved");

        assert_eq!(saved.lifecycle, failed.lifecycle);
        assert!(saved.plan["acknowledged_at"].is_string());
        fixture
            .repository
            .acknowledge_failed_bulk_operation(fixture.signer.id, failed.lifecycle.id)
            .await
            .expect("repeat acknowledgement");

        assert_eq!(
            fixture
                .repository
                .store
                .operation(failed.lifecycle.id)
                .await
                .expect("load again")
                .expect("saved again"),
            saved
        );

        let (network, plan) = fixture
            .repository
            .saved_bulk_plan(fixture.signer.id, failed.lifecycle.id)
            .await
            .expect("failed plan remains available");

        assert_eq!(network, LabNetwork::MutinyNet);
        assert_eq!(plan, fixture.plan);
    }

    #[tokio::test]
    async fn active_operation_takes_priority_and_cannot_be_acknowledged() {
        let mut fixture = BulkFixture::new().await;
        let failed = fixture.failed("failure", false).await;
        let mut active = fixture.prepare().await;
        // force the active operation to sort after the failure in the newest-first query
        active.lifecycle.updated_at = Timestamp::from_second(0).expect("epoch");
        fixture
            .repository
            .store
            .save_operation(&active)
            .await
            .expect("save active");

        let result = fixture
            .repository
            .current_bulk_operation(fixture.signer.id, LabNetwork::MutinyNet)
            .await
            .expect("current active");

        assert!(matches!(
            result,
            Some(CurrentBulkOperation::Active { operation_id, .. }) if operation_id == active.lifecycle.id
        ));

        let result = fixture
            .repository
            .acknowledge_failed_bulk_operation(fixture.signer.id, active.lifecycle.id)
            .await;

        assert!(matches!(
            result,
            Err(BulkExecutionError::NotFailed(id)) if id == active.lifecycle.id
        ));

        assert_eq!(
            fixture
                .repository
                .store
                .operation(active.lifecycle.id)
                .await
                .expect("load active")
                .expect("active"),
            active
        );

        assert!(matches!(
            fixture
                .repository
                .acknowledge_failed_bulk_operation(SignerId::new(), failed.lifecycle.id)
                .await,
            Err(BulkExecutionError::WrongSigner)
        ));
    }
}
