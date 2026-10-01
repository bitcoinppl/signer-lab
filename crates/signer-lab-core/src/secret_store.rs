use crate::{MnemonicSecret, SignerId, mnemonic::MnemonicError};

const KEYCHAIN_SERVICE: &str = "org.bitcoinppl.signer-lab";

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

/// A macOS Keychain secret store.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeychainSecretStore;

impl KeychainSecretStore {
    fn entry(signer_id: SignerId) -> Result<keyring::Entry, SecretStoreError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &signer_id.to_string())
            .map_err(SecretStoreError::Keychain)
    }
}

impl SecretStore for KeychainSecretStore {
    fn save(&self, signer_id: SignerId, secret: &MnemonicSecret) -> Result<(), SecretStoreError> {
        Self::entry(signer_id)?
            .set_password(secret.expose_phrase())
            .map_err(SecretStoreError::Keychain)
    }

    fn load(&self, signer_id: SignerId) -> Result<MnemonicSecret, SecretStoreError> {
        let phrase = Self::entry(signer_id)?
            .get_password()
            .map_err(SecretStoreError::Keychain)?;

        MnemonicSecret::parse(&phrase).map_err(SecretStoreError::InvalidSecret)
    }

    fn delete(&self, signer_id: SignerId) -> Result<(), SecretStoreError> {
        match Self::entry(signer_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(SecretStoreError::Keychain(error)),
        }
    }
}

/// Errors from secret storage.
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    /// The Keychain operation failed.
    #[error("Keychain operation failed: {0}")]
    Keychain(#[source] keyring::Error),
    /// Stored secret data is not a valid mnemonic.
    #[error("stored signer secret is invalid: {0}")]
    InvalidSecret(#[source] MnemonicError),
}
