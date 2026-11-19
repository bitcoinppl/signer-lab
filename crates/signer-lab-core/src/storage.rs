// toasty 0.10's `#[index]` expansion reuses field spans for underscore-prefixed items it
// generates outside the struct, so a struct-level allow cannot reach them
#![allow(clippy::used_underscore_items)]

use std::{
    path::{Path, PathBuf},
    str::FromStr as _,
};

use bitcoin::bip32::Fingerprint;
use jiff::Timestamp;
use serde_json::Value;
use toasty_driver_turso::Turso;

use crate::{AccountIndex, OperationId, OperationLifecycle, SignerId, SignerName, VirtualSigner};

const FORMAT_VERSION: i64 = 1;

#[derive(Debug, Clone, toasty::Model)]
struct StoreMetadata {
    #[key]
    singleton: i64,
    format_version: i64,
}

#[derive(Debug, Clone, toasty::Model)]
struct SignerRecord {
    #[key]
    id: String,
    name: String,
    fingerprint: String,
    account_index: i64,
    created_at: String,
}

#[derive(Debug, Clone, toasty::Model)]
#[index(signer_id, updated_at)]
struct OperationRecord {
    #[key]
    id: String,
    signer_id: String,
    lifecycle_json: String,
    plan_json: String,
    updated_at: String,
}

/// A durable operation with its typed lifecycle and versioned plan document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOperation {
    /// Signer that owns the operation.
    pub signer_id: SignerId,
    /// Checked operation state.
    pub lifecycle: OperationLifecycle,
    /// Serialized transaction plan for recovery.
    pub plan: Value,
}

/// Toasty-backed public metadata and operation storage.
pub struct LabStore {
    db: toasty::Db,
    path: PathBuf,
}

impl LabStore {
    /// Creates a new store and applies the compiled Toasty schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the path exists or the database cannot be created.
    pub async fn create(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if path.exists() {
            return Err(StorageError::AlreadyExists(path.to_path_buf()));
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StorageError::Io)?;
        }

        let mut db = connect(path).await?;
        db.push_schema().await?;
        toasty::create!(StoreMetadata {
            singleton: 1_i64,
            format_version: FORMAT_VERSION
        })
        .exec(&mut db)
        .await?;

        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    /// Opens an existing store and checks its format version.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is absent, invalid, or has an unsupported format.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(StorageError::NotFound(path.to_path_buf()));
        }

        let mut db = connect(path).await?;
        verify_schema(&mut db).await?;

        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    /// Creates a store when absent or opens the existing store.
    ///
    /// # Errors
    ///
    /// Returns an error when filesystem or database setup fails.
    pub async fn open_or_create(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if path.exists() {
            return Self::open(path).await;
        }

        Self::create(path).await
    }

    /// Returns the database path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Inserts or replaces one signer metadata record.
    ///
    /// # Errors
    ///
    /// Returns an error when the database rejects the write.
    pub async fn save_signer(&mut self, signer: &VirtualSigner) -> Result<(), StorageError> {
        SignerRecord::upsert_by_id(signer.id.to_string())
            .name(signer.name.to_string())
            .fingerprint(signer.fingerprint.to_string())
            .account_index(i64::from(signer.account_index.get()))
            .created_at(signer.created_at.to_string())
            .exec(&mut self.db)
            .await?;

        Ok(())
    }

    /// Lists all signer metadata, ordered by creation time and identifier.
    ///
    /// # Errors
    ///
    /// Returns an error for database failures or corrupt stored data.
    pub async fn signers(&mut self) -> Result<Vec<VirtualSigner>, StorageError> {
        let records = SignerRecord::all().exec(&mut self.db).await?;
        let mut signers = records
            .into_iter()
            .map(decode_signer)
            .collect::<Result<Vec<_>, _>>()?;
        signers.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.to_string().cmp(&right.id.to_string()))
        });

        Ok(signers)
    }

    /// Removes signer metadata and its stored operations.
    ///
    /// Secret file removal and BDK database removal are separate explicit actions.
    ///
    /// # Errors
    ///
    /// Returns an error when the database rejects either delete.
    pub async fn delete_signer(&mut self, signer_id: SignerId) -> Result<(), StorageError> {
        let signer_id = signer_id.to_string();
        OperationRecord::filter(OperationRecord::fields().signer_id().eq(signer_id.clone()))
            .delete()
            .exec(&mut self.db)
            .await?;
        SignerRecord::filter(SignerRecord::fields().id().eq(signer_id))
            .delete()
            .exec(&mut self.db)
            .await?;

        Ok(())
    }

    /// Inserts or replaces a recoverable bulk operation.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON encoding or the database write fails.
    pub async fn save_operation(
        &mut self,
        operation: &StoredOperation,
    ) -> Result<(), StorageError> {
        let lifecycle_json = serde_json::to_string(&operation.lifecycle)?;
        let plan_json = serde_json::to_string(&operation.plan)?;

        OperationRecord::upsert_by_id(operation.lifecycle.id.to_string())
            .signer_id(operation.signer_id.to_string())
            .lifecycle_json(lifecycle_json)
            .plan_json(plan_json)
            .updated_at(operation.lifecycle.updated_at.to_string())
            .exec(&mut self.db)
            .await?;

        Ok(())
    }

    /// Loads one operation by identifier.
    ///
    /// # Errors
    ///
    /// Returns an error for database failures or corrupt stored data.
    pub async fn operation(
        &mut self,
        operation_id: OperationId,
    ) -> Result<Option<StoredOperation>, StorageError> {
        OperationRecord::filter(OperationRecord::fields().id().eq(operation_id.to_string()))
            .first()
            .exec(&mut self.db)
            .await?
            .map(|record| decode_operation(&record))
            .transpose()
    }

    /// Lists recoverable operations for one signer, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error for database failures or corrupt stored data.
    pub async fn operations_for_signer(
        &mut self,
        signer_id: SignerId,
    ) -> Result<Vec<StoredOperation>, StorageError> {
        OperationRecord::filter(
            OperationRecord::fields()
                .signer_id()
                .eq(signer_id.to_string()),
        )
        .order_by(OperationRecord::fields().updated_at().desc())
        .exec(&mut self.db)
        .await?
        .iter()
        .map(decode_operation)
        .collect()
    }
}

async fn connect(path: &Path) -> Result<toasty::Db, StorageError> {
    let mut builder = toasty::Db::builder();
    builder
        .models(toasty::models!(
            StoreMetadata,
            SignerRecord,
            OperationRecord
        ))
        .max_pool_size(1);

    Ok(builder.build(Turso::file(path)).await?)
}

async fn verify_schema(db: &mut toasty::Db) -> Result<(), StorageError> {
    let metadata = StoreMetadata::filter(StoreMetadata::fields().singleton().eq(1_i64))
        .first()
        .exec(db)
        .await?
        .ok_or(StorageError::MissingMetadata)?;

    if metadata.format_version != FORMAT_VERSION {
        return Err(StorageError::UnsupportedFormat {
            expected: FORMAT_VERSION,
            actual: metadata.format_version,
        });
    }

    Ok(())
}

fn decode_signer(record: SignerRecord) -> Result<VirtualSigner, StorageError> {
    let account_index = u32::try_from(record.account_index)
        .map_err(|_| StorageError::Corrupt("negative signer account index".to_owned()))?;

    Ok(VirtualSigner {
        id: SignerId::from_str(&record.id).map_err(StorageError::SignerId)?,
        name: SignerName::new(record.name).map_err(StorageError::SignerName)?,
        fingerprint: Fingerprint::from_str(&record.fingerprint)
            .map_err(StorageError::Fingerprint)?,
        account_index: AccountIndex::new(account_index).map_err(StorageError::AccountIndex)?,
        created_at: Timestamp::from_str(&record.created_at).map_err(StorageError::Timestamp)?,
    })
}

fn decode_operation(record: &OperationRecord) -> Result<StoredOperation, StorageError> {
    let lifecycle: OperationLifecycle = serde_json::from_str(&record.lifecycle_json)?;
    if lifecycle.id.to_string() != record.id {
        return Err(StorageError::Corrupt(
            "operation identifier does not match its lifecycle".to_owned(),
        ));
    }

    Ok(StoredOperation {
        signer_id: SignerId::from_str(&record.signer_id).map_err(StorageError::SignerId)?,
        lifecycle,
        plan: serde_json::from_str(&record.plan_json)?,
    })
}

/// Durable storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The requested new store already exists.
    #[error("store already exists at {0}")]
    AlreadyExists(PathBuf),
    /// The requested store does not exist.
    #[error("store does not exist at {0}")]
    NotFound(PathBuf),
    /// Filesystem setup failed.
    #[error("store filesystem operation failed: {0}")]
    Io(#[source] std::io::Error),
    /// Toasty or Turso failed.
    #[error("store database operation failed: {0}")]
    Database(#[from] toasty::Error),
    /// The store metadata row is missing.
    #[error("store metadata is missing")]
    MissingMetadata,
    /// The store format is newer or older than this build supports.
    #[error("unsupported store format {actual}; expected {expected}")]
    UnsupportedFormat {
        /// Format supported by this build.
        expected: i64,
        /// Format found in the store.
        actual: i64,
    },
    /// Stored signer identity is invalid.
    #[error("stored signer identifier is invalid: {0}")]
    SignerId(#[source] uuid::Error),
    /// Stored signer name is invalid.
    #[error("stored signer name is invalid: {0}")]
    SignerName(#[source] crate::SignerNameError),
    /// Stored fingerprint is invalid.
    #[error("stored signer fingerprint is invalid: {0}")]
    Fingerprint(#[source] bitcoin::hex::HexToArrayError),
    /// Stored account index is invalid.
    #[error("stored signer account index is invalid: {0}")]
    AccountIndex(#[source] crate::AccountIndexError),
    /// Stored timestamp is invalid.
    #[error("stored timestamp is invalid: {0}")]
    Timestamp(#[source] jiff::Error),
    /// JSON encoding or decoding failed.
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// A cross-field storage invariant failed.
    #[error("stored data is corrupt: {0}")]
    Corrupt(String),
}

#[cfg(test)]
mod tests {
    use bitcoin::bip32::Fingerprint;
    use jiff::Timestamp;
    use serde_json::json;

    use super::{LabStore, StoredOperation};
    use crate::{
        AccountIndex, OperationKind, OperationLifecycle, SignerId, SignerName, VirtualSigner,
    };

    #[tokio::test]
    async fn signer_and_operation_survive_reopen() {
        let temp =
            std::env::temp_dir().join(format!("signer-lab-store-{}.db", uuid::Uuid::new_v4()));
        let signer = VirtualSigner {
            id: SignerId::new(),
            name: SignerName::new("Test signer").expect("test signer name is valid"),
            fingerprint: Fingerprint::from([1, 2, 3, 4]),
            account_index: AccountIndex::default(),
            created_at: Timestamp::UNIX_EPOCH,
        };
        let lifecycle = OperationLifecycle::prepared(OperationKind::FanOut, Timestamp::UNIX_EPOCH);
        let operation = StoredOperation {
            signer_id: signer.id,
            lifecycle: lifecycle.clone(),
            plan: json!({"outputs": 20}),
        };
        let mut store = LabStore::create(&temp)
            .await
            .expect("test metadata store creates");
        store.save_signer(&signer).await.expect("test signer saves");
        store
            .save_operation(&operation)
            .await
            .expect("test operation saves");
        drop(store);

        let mut reopened = LabStore::open(&temp)
            .await
            .expect("test metadata store reopens");
        assert_eq!(
            reopened.signers().await.expect("test signers load"),
            vec![signer]
        );
        assert_eq!(
            reopened
                .operation(lifecycle.id)
                .await
                .expect("test operation loads"),
            Some(operation)
        );

        drop(reopened);
        let _ = std::fs::remove_file(temp);
    }
}
