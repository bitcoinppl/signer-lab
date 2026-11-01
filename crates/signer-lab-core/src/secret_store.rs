use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{self, Write as _},
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use zeroize::Zeroizing;

use crate::{MnemonicSecret, SignerId, mnemonic::MnemonicError};

/// Storage for signer secrets.
pub trait SecretStore: Send + Sync {
    /// Saves or replaces a signer mnemonic.
    ///
    /// # Errors
    ///
    /// Returns an error when the secret store rejects the write.
    fn save(&self, signer_id: SignerId, secret: &MnemonicSecret) -> Result<(), SecretStoreError>;

    /// Loads a signer mnemonic.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is missing, inaccessible, or invalid.
    fn load(&self, signer_id: SignerId) -> Result<MnemonicSecret, SecretStoreError>;

    /// Deletes a signer mnemonic.
    ///
    /// Missing entries are treated as already deleted.
    ///
    /// # Errors
    ///
    /// Returns an error when the secret store rejects the delete.
    fn delete(&self, signer_id: SignerId) -> Result<(), SecretStoreError>;
}

/// A secret store that keeps one owner-only plain-text mnemonic file per signer
///
/// Signer Lab only holds test-network keys, so plain files trade Keychain protection for
/// automation that never waits on a native access prompt
#[derive(Debug, Clone)]
pub struct FileSecretStore(PathBuf);

impl FileSecretStore {
    /// Creates a store that keeps secrets in `directory`, creating it on first save
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self(directory.into())
    }

    /// Directory that holds one mnemonic file per signer
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.0
    }

    fn path(&self, signer_id: SignerId) -> PathBuf {
        self.0.join(format!("{signer_id}.txt"))
    }
}

impl SecretStore for FileSecretStore {
    fn save(&self, signer_id: SignerId, secret: &MnemonicSecret) -> Result<(), SecretStoreError> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.0)
            .map_err(SecretStoreError::io(&self.0))?;

        // write a sibling and rename it so a crash never leaves a truncated mnemonic
        let path = self.path(signer_id);
        let staged = path.with_extension("txt.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&staged)
            .map_err(SecretStoreError::io(&staged))?;

        file.write_all(secret.expose_phrase().as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(SecretStoreError::io(&staged))?;

        fs::rename(&staged, &path).map_err(SecretStoreError::io(&path))
    }

    fn load(&self, signer_id: SignerId) -> Result<MnemonicSecret, SecretStoreError> {
        let path = self.path(signer_id);
        let phrase = match fs::read_to_string(&path) {
            Ok(phrase) => Zeroizing::new(phrase),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(SecretStoreError::Missing { signer_id, path });
            }
            Err(source) => return Err(SecretStoreError::Io { path, source }),
        };

        MnemonicSecret::parse(phrase.trim()).map_err(SecretStoreError::InvalidSecret)
    }

    fn delete(&self, signer_id: SignerId) -> Result<(), SecretStoreError> {
        let path = self.path(signer_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SecretStoreError::Io { path, source }),
        }
    }
}

/// Errors from secret storage.
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    /// No secret file exists for the signer
    #[error("no recovery words saved for signer {signer_id} at {}", path.display())]
    Missing {
        /// Signer whose secret was requested
        signer_id: SignerId,
        /// Expected secret file
        path: PathBuf,
    },
    /// Reading or writing a secret file failed
    #[error("secret file operation failed at {}: {source}", path.display())]
    Io {
        /// File or directory that failed
        path: PathBuf,
        /// Filesystem failure
        #[source]
        source: io::Error,
    },
    /// Stored secret data is not a valid mnemonic.
    #[error("stored signer secret is invalid: {0}")]
    InvalidSecret(#[source] MnemonicError),
}

impl SecretStoreError {
    fn io(path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_path_buf();
        move |source| Self::Io { path, source }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::{FileSecretStore, SecretStore as _, SecretStoreError};
    use crate::{MnemonicSecret, SignerId};

    const WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn secrets_round_trip_through_owner_only_files() {
        let directory =
            std::env::temp_dir().join(format!("signer-lab-secrets-{}", SignerId::new()));
        let store = FileSecretStore::new(directory.join("secrets"));
        let signer_id = SignerId::new();
        let secret = MnemonicSecret::parse(WORDS).expect("test mnemonic");

        store.save(signer_id, &secret).expect("save");
        store.save(signer_id, &secret).expect("replace");

        let loaded = store.load(signer_id).expect("load");
        assert_eq!(loaded.expose_phrase(), WORDS);

        let mode = |path: &std::path::Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(store.directory()), 0o700);
        assert_eq!(mode(&store.path(signer_id)), 0o600);

        store.delete(signer_id).expect("delete");
        store.delete(signer_id).expect("delete is idempotent");
        assert!(matches!(
            store.load(signer_id),
            Err(SecretStoreError::Missing { .. })
        ));

        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
