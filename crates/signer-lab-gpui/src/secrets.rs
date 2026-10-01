use signer_lab_core::{
    KeychainSecretStore, MnemonicSecret, SecretStore, SecretStoreError, SignerId,
};

pub(crate) enum AppSecretStore {
    Keychain,
    #[cfg(feature = "test-support")]
    Memory(crate::test_support::MemorySecretStore),
}

impl SecretStore for AppSecretStore {
    fn save(&self, id: SignerId, secret: &MnemonicSecret) -> Result<(), SecretStoreError> {
        match self {
            Self::Keychain => KeychainSecretStore.save(id, secret),
            #[cfg(feature = "test-support")]
            Self::Memory(store) => store.save(id, secret),
        }
    }

    fn load(&self, id: SignerId) -> Result<MnemonicSecret, SecretStoreError> {
        match self {
            Self::Keychain => KeychainSecretStore.load(id),
            #[cfg(feature = "test-support")]
            Self::Memory(store) => store.load(id),
        }
    }

    fn delete(&self, id: SignerId) -> Result<(), SecretStoreError> {
        match self {
            Self::Keychain => KeychainSecretStore.delete(id),
            #[cfg(feature = "test-support")]
            Self::Memory(store) => store.delete(id),
        }
    }
}
