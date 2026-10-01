use std::collections::HashSet;

use bdk_esplora::esplora_client;
use cove_util::ResultExt as _;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{RepositoryError, SignerRepository};
use crate::{
    AddressType, BulkPlan, LabNetwork, OperationId, OperationLifecycle, OperationState,
    SecretStore, SignerId, StoredOperation, VirtualSigner,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredBulkPlan {
    network: LabNetwork,
    plan: BulkPlan,
}

/// A saved nonterminal bulk operation that can continue after restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableBulkOperation {
    /// Stable operation identifier.
    pub operation_id: OperationId,
    /// Active network captured when the plan was prepared.
    pub network: LabNetwork,
    /// Checked BDK transaction plan with persistent input reservations.
    pub plan: BulkPlan,
}

/// Result of reconciling Toasty operations with BDK persistent input locks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservationReconciliation {
    /// Expected operation inputs that were locked or already locked.
    pub active_reservations: usize,
    /// Locks without a nonterminal operation that were released.
    pub orphaned_locks_released: usize,
}

impl<S: SecretStore> SignerRepository<S> {
    /// Saves a recoverable bulk plan with a prepared lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON encoding or metadata storage fails.
    pub async fn prepare_bulk_operation(
        &mut self,
        signer_id: SignerId,
        network: LabNetwork,
        plan: &BulkPlan,
    ) -> Result<StoredOperation, RepositoryError> {
        for operation in self
            .store
            .operations_for_signer(signer_id)
            .await
            .map_err(RepositoryError::Storage)?
        {
            if !matches!(
                operation.lifecycle.state,
                OperationState::Prepared
                    | OperationState::Running { .. }
                    | OperationState::Interrupted { .. }
            ) {
                continue;
            }

            let stored: StoredBulkPlan = serde_json::from_value(operation.plan)
                .map_err(crate::StorageError::Json)
                .map_err(RepositoryError::Storage)?;
            if stored.network == network {
                return Err(RepositoryError::ActiveBulkOperation(operation.lifecycle.id));
            }
        }

        let operation = StoredOperation {
            signer_id,
            lifecycle: OperationLifecycle::prepared(plan.kind(), Timestamp::now()),
            plan: serde_json::to_value(StoredBulkPlan {
                network,
                plan: plan.clone(),
            })
            .map_err(crate::StorageError::Json)
            .map_err(RepositoryError::Storage)?,
        };
        self.store
            .save_operation(&operation)
            .await
            .map_err(RepositoryError::Storage)?;

        Ok(operation)
    }

    /// Restores active plan locks and releases BDK locks without a nonterminal operation.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid saved plans, Keychain access, BDK persistence, or storage.
    pub async fn reconcile_bulk_reservations(
        &mut self,
        signer: &VirtualSigner,
        network: LabNetwork,
    ) -> Result<ReservationReconciliation, BulkExecutionError> {
        let operations = self
            .store
            .operations_for_signer(signer.id)
            .await
            .map_err(BulkExecutionError::Storage)?;
        let mut expected = HashSet::new();

        for operation in operations {
            if !matches!(
                operation.lifecycle.state,
                OperationState::Prepared
                    | OperationState::Running { .. }
                    | OperationState::Interrupted { .. }
            ) {
                continue;
            }

            let stored: StoredBulkPlan =
                serde_json::from_value(operation.plan).map_err(BulkExecutionError::Plan)?;
            if stored.network == network {
                expected.extend(stored.plan.reserved_outpoints());
            }
        }

        let mut wallet = self
            .open_wallet(signer, network)
            .map_err(BulkExecutionError::Repository)?;
        let account = wallet.account_mut(AddressType::NativeSegwit);
        let locked = account.bdk().list_locked_unspent().collect::<Vec<_>>();
        let orphaned_locks_released = locked
            .iter()
            .filter(|outpoint| !expected.contains(outpoint))
            .count();

        for outpoint in &expected {
            account.bdk_mut().lock_outpoint(*outpoint);
        }
        for outpoint in locked {
            if !expected.contains(&outpoint) {
                account.bdk_mut().unlock_outpoint(outpoint);
            }
        }
        account.persist().map_err(BulkExecutionError::Wallet)?;

        Ok(ReservationReconciliation {
            active_reservations: expected.len(),
            orphaned_locks_released,
        })
    }

    /// Lists saved bulk operations for a signer.
    ///
    /// # Errors
    ///
    /// Returns an error for database failures or corrupt operation data.
    pub async fn bulk_operations(
        &mut self,
        signer_id: SignerId,
    ) -> Result<Vec<StoredOperation>, RepositoryError> {
        self.store
            .operations_for_signer(signer_id)
            .await
            .map_err(RepositoryError::Storage)
    }

    /// Returns the newest nonterminal bulk operation for a signer and network.
    ///
    /// # Errors
    ///
    /// Returns an error for database failures or invalid saved plan data.
    pub async fn recoverable_bulk_operation(
        &mut self,
        signer_id: SignerId,
        network: LabNetwork,
    ) -> Result<Option<RecoverableBulkOperation>, BulkExecutionError> {
        let operations = self
            .store
            .operations_for_signer(signer_id)
            .await
            .map_err(BulkExecutionError::Storage)?;

        for operation in operations {
            if !matches!(
                operation.lifecycle.state,
                OperationState::Prepared
                    | OperationState::Running { .. }
                    | OperationState::Interrupted { .. }
            ) {
                continue;
            }

            let stored: StoredBulkPlan =
                serde_json::from_value(operation.plan).map_err(BulkExecutionError::Plan)?;
            if stored.network == network {
                return Ok(Some(RecoverableBulkOperation {
                    operation_id: operation.lifecycle.id,
                    network,
                    plan: stored.plan,
                }));
            }
        }

        Ok(None)
    }

    /// Loads the checked plan for any saved bulk-operation state.
    ///
    /// This includes failed operations whose signed PSBT was saved before relay.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation is missing, owned by another signer, or corrupt.
    pub async fn saved_bulk_plan(
        &mut self,
        signer_id: SignerId,
        operation_id: OperationId,
    ) -> Result<(LabNetwork, BulkPlan), BulkExecutionError> {
        let operation = self
            .store
            .operation(operation_id)
            .await
            .map_err(BulkExecutionError::Storage)?
            .ok_or(BulkExecutionError::NotFound(operation_id))?;
        if operation.signer_id != signer_id {
            return Err(BulkExecutionError::WrongSigner);
        }

        let stored: StoredBulkPlan =
            serde_json::from_value(operation.plan).map_err(BulkExecutionError::Plan)?;

        Ok((stored.network, stored.plan))
    }

    /// Signs and broadcasts a saved bulk operation in transaction order.
    ///
    /// The signed plan is saved before each broadcast attempt. A relay-policy failure therefore
    /// leaves the signed transaction available for inspection or later submission.
    ///
    /// # Errors
    ///
    /// Returns an error for missing operations, invalid state, signing, extraction, relay, or
    /// persistence failures.
    pub async fn execute_bulk_operation(
        &mut self,
        signer: &VirtualSigner,
        network: LabNetwork,
        operation_id: OperationId,
    ) -> Result<BulkExecutionReport, BulkExecutionError> {
        let mut operation = self
            .store
            .operation(operation_id)
            .await
            .map_err(BulkExecutionError::Storage)?
            .ok_or(BulkExecutionError::NotFound(operation_id))?;
        if operation.signer_id != signer.id {
            return Err(BulkExecutionError::WrongSigner);
        }

        let mut stored: StoredBulkPlan =
            serde_json::from_value(operation.plan.clone()).map_err(BulkExecutionError::Plan)?;
        if stored.network != network {
            return Err(BulkExecutionError::WrongNetwork {
                expected: stored.network,
                actual: network,
            });
        }
        let mut wallet = self
            .open_wallet(signer, network)
            .map_err(BulkExecutionError::Repository)?;
        stored
            .plan
            .reserve_inputs(wallet.account_mut(AddressType::NativeSegwit))
            .map_err(BulkExecutionError::BulkPlan)?;
        if !start_or_resume(&mut operation.lifecycle) {
            return Err(BulkExecutionError::InvalidState);
        }
        self.persist_operation_plan(&mut operation, &stored).await?;

        let client = esplora_client::Builder::new(network.esplora_url())
            .build_async()
            .map_err_str(BulkExecutionError::Esplora)?;
        let start_index =
            running_index(&operation.lifecycle).ok_or(BulkExecutionError::InvalidState)?;

        for index in start_index..stored.plan.transactions().len() {
            let transaction = stored
                .plan
                .transactions_mut()
                .get_mut(index)
                .ok_or(BulkExecutionError::MissingTransaction(index))?;
            match wallet.sign(transaction.psbt_mut(), true) {
                Ok(report) if report.finalized => {}
                Ok(_) => {
                    let error = "BDK did not finalize the planned PSBT".to_owned();
                    self.fail_operation(&mut operation, &stored, index, &error)
                        .await?;
                    return Err(BulkExecutionError::NotFinalized(index));
                }
                Err(source) => {
                    let error = source.to_string();
                    self.fail_operation(&mut operation, &stored, index, &error)
                        .await?;
                    return Err(BulkExecutionError::Wallet(source));
                }
            }
            operation.plan = serde_json::to_value(&stored).map_err(BulkExecutionError::Plan)?;
            self.store
                .save_operation(&operation)
                .await
                .map_err(BulkExecutionError::Storage)?;

            let transaction = stored.plan.transactions()[index]
                .psbt()
                .clone()
                .extract_tx()
                .map_err(BulkExecutionError::Extract)?;
            let txid = transaction.compute_txid();
            if let Err(source) = client.broadcast(&transaction).await {
                let error = source.to_string();
                self.fail_operation(&mut operation, &stored, index, &error)
                    .await?;
                return Err(BulkExecutionError::Broadcast {
                    index,
                    reason: error,
                });
            }

            operation
                .lifecycle
                .record_accepted(index, txid, Timestamp::now())
                .map_err(BulkExecutionError::State)?;
            self.persist_operation_plan(&mut operation, &stored).await?;
        }

        operation
            .lifecycle
            .complete(stored.plan.transactions().len(), Timestamp::now())
            .map_err(BulkExecutionError::State)?;
        self.persist_operation_plan(&mut operation, &stored).await?;

        let OperationState::Completed { txids } = operation.lifecycle.state else {
            return Err(BulkExecutionError::InvalidState);
        };

        Ok(BulkExecutionReport {
            operation_id,
            txids,
        })
    }

    /// Cancels a prepared operation or closes a failed operation and releases BDK reservations.
    ///
    /// # Errors
    ///
    /// Returns an error for missing operations, invalid state, wallet, plan, or storage failures.
    pub async fn cancel_bulk_operation(
        &mut self,
        signer: &VirtualSigner,
        network: LabNetwork,
        operation_id: OperationId,
    ) -> Result<(), BulkExecutionError> {
        let mut operation = self
            .store
            .operation(operation_id)
            .await
            .map_err(BulkExecutionError::Storage)?
            .ok_or(BulkExecutionError::NotFound(operation_id))?;
        if operation.signer_id != signer.id {
            return Err(BulkExecutionError::WrongSigner);
        }

        let stored: StoredBulkPlan =
            serde_json::from_value(operation.plan.clone()).map_err(BulkExecutionError::Plan)?;
        if stored.network != network {
            return Err(BulkExecutionError::WrongNetwork {
                expected: stored.network,
                actual: network,
            });
        }
        let mut wallet = self
            .open_wallet(signer, network)
            .map_err(BulkExecutionError::Repository)?;
        stored
            .plan
            .release_reservations(wallet.account_mut(AddressType::NativeSegwit))
            .map_err(BulkExecutionError::BulkPlan)?;
        match operation.lifecycle.state {
            OperationState::Prepared => operation
                .lifecycle
                .cancel(Timestamp::now())
                .map_err(BulkExecutionError::State)?,
            OperationState::Failed { .. } => {}
            _ => return Err(BulkExecutionError::InvalidState),
        }
        self.persist_operation_plan(&mut operation, &stored).await
    }

    async fn persist_operation_plan(
        &mut self,
        operation: &mut StoredOperation,
        plan: &StoredBulkPlan,
    ) -> Result<(), BulkExecutionError> {
        operation.plan = serde_json::to_value(plan).map_err(BulkExecutionError::Plan)?;
        self.store
            .save_operation(operation)
            .await
            .map_err(BulkExecutionError::Storage)
    }

    async fn fail_operation(
        &mut self,
        operation: &mut StoredOperation,
        plan: &StoredBulkPlan,
        index: usize,
        error: &str,
    ) -> Result<(), BulkExecutionError> {
        operation
            .lifecycle
            .fail(index, error, Timestamp::now())
            .map_err(BulkExecutionError::State)?;
        self.persist_operation_plan(operation, plan).await
    }
}

fn start_or_resume(lifecycle: &mut OperationLifecycle) -> bool {
    match lifecycle.state {
        OperationState::Prepared => lifecycle.start(Timestamp::now()).is_ok(),
        OperationState::Interrupted { .. } => lifecycle.resume(Timestamp::now()).is_ok(),
        OperationState::Running { .. } => true,
        _ => false,
    }
}

fn running_index(lifecycle: &OperationLifecycle) -> Option<usize> {
    let OperationState::Running { next_index, .. } = lifecycle.state else {
        return None;
    };

    Some(next_index)
}

/// Result of a completed bulk execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkExecutionReport {
    /// Executed operation identifier.
    pub operation_id: OperationId,
    /// Transaction identifiers accepted by the configured Esplora server.
    pub txids: Vec<bitcoin::Txid>,
}

/// Bulk-operation preparation, recovery, signing, and relay errors.
#[derive(Debug, thiserror::Error)]
pub enum BulkExecutionError {
    /// The operation does not exist.
    #[error("bulk operation {0} does not exist")]
    NotFound(OperationId),
    /// The selected signer does not own the operation.
    #[error("selected signer does not own this bulk operation")]
    WrongSigner,
    /// The selected network does not match the saved operation.
    #[error("bulk operation belongs to {expected}, not {actual}")]
    WrongNetwork {
        /// Network stored with the operation.
        expected: LabNetwork,
        /// Network requested for execution.
        actual: LabNetwork,
    },
    /// The saved lifecycle cannot execute the requested action.
    #[error("bulk operation is not in an executable state")]
    InvalidState,
    /// The plan omitted an expected transaction.
    #[error("bulk plan has no transaction at index {0}")]
    MissingTransaction(usize),
    /// Saved plan JSON is invalid.
    #[error("bulk plan JSON is invalid: {0}")]
    Plan(#[source] serde_json::Error),
    /// Public metadata storage failed.
    #[error("bulk operation storage failed: {0}")]
    Storage(#[source] crate::StorageError),
    /// Operation-state transition failed.
    #[error("bulk operation state transition failed: {0}")]
    State(#[source] crate::StateTransitionError),
    /// Repository wallet access failed.
    #[error("bulk operation wallet access failed: {0}")]
    Repository(#[source] RepositoryError),
    /// BDK wallet signing failed.
    #[error("bulk operation signing failed: {0}")]
    Wallet(#[source] crate::WalletError),
    /// BDK did not finalize a planned transaction.
    #[error("bulk transaction {0} was not finalized")]
    NotFinalized(usize),
    /// A planned transaction could not be extracted from its signed PSBT.
    #[error("signed transaction extraction failed: {0}")]
    Extract(#[source] bitcoin::psbt::ExtractTxError),
    /// Esplora client creation failed.
    #[error("Esplora client setup failed: {0}")]
    Esplora(String),
    /// Esplora rejected or failed to relay a signed transaction.
    #[error("bulk transaction {index} was saved but relay failed: {reason}")]
    Broadcast {
        /// Transaction index whose relay failed.
        index: usize,
        /// Stable relay failure text.
        reason: String,
    },
    /// BDK reservation release failed.
    #[error("bulk reservation release failed: {0}")]
    BulkPlan(#[source] crate::BulkPlanError),
}
