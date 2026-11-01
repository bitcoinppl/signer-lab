use std::fmt;

use bip39::{Language, Mnemonic};
use bitcoin::{
    Network,
    bip32::{Fingerprint, Xpriv, Xpub},
    key::Secp256k1,
};
use cove_util::ResultExt as _;
use rand::RngExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use zeroize::Zeroizing;

use crate::StandardSeedQr;

/// A validated English BIP39 mnemonic kept in secret memory.
pub struct MnemonicSecret(SecretString);

impl MnemonicSecret {
    /// Generates a random 12-word or 24-word mnemonic.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested word count is not 12 or 24.
    pub fn generate(word_count: usize) -> Result<Self, MnemonicError> {
        let entropy_len = match word_count {
            12 => 16,
            24 => 32,
            count => return Err(MnemonicError::UnsupportedGeneratedWordCount(count)),
        };

        let mut entropy = Zeroizing::new(vec![0_u8; entropy_len]);
        rand::rng().fill(entropy.as_mut_slice());
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .map_err(MnemonicError::Invalid)?;

        Ok(Self(SecretString::from(mnemonic.to_string())))
    }

    /// Parses and normalizes an English BIP39 mnemonic.
    ///
    /// # Errors
    ///
    /// Returns an error when the words or checksum are invalid.
    pub fn parse(phrase: &str) -> Result<Self, MnemonicError> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase)
            .map_err(MnemonicError::Invalid)?;

        Ok(Self(SecretString::from(mnemonic.to_string())))
    }

    /// Parses recovery words or standard `SeedQR` digits
    ///
    /// # Errors
    ///
    /// Returns an error when the selected import format is invalid
    pub fn parse_import(value: &str) -> Result<Self, MnemonicImportError> {
        let value = value.trim();
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            return StandardSeedQr::parse(value)
                .map(StandardSeedQr::into_secret)
                .map_err(MnemonicImportError::SeedQr);
        }

        Self::parse(value).map_err(MnemonicImportError::Mnemonic)
    }

    /// Returns valid final-word suggestions for 11 or 23 valid words.
    ///
    /// # Errors
    ///
    /// Returns an error when an existing word is invalid or the count is unsupported.
    pub fn possible_final_words(partial_phrase: &str) -> Result<Vec<String>, MnemonicError> {
        for word in partial_phrase.split_whitespace() {
            if Language::English.find_word(word).is_none() {
                return Err(MnemonicError::InvalidWord(word.to_owned()));
            }
        }

        cove_bip39::generate_possible_final_words(partial_phrase)
            .map_err_str(MnemonicError::FinalWord)
    }

    /// Exposes the mnemonic for a short operation such as writing it to the secret store.
    #[must_use]
    pub fn expose_phrase(&self) -> &str {
        self.0.expose_secret()
    }

    /// Parses the secret into the BIP39 type.
    ///
    /// # Errors
    ///
    /// Returns an error only if secret memory was corrupted after construction.
    pub fn to_mnemonic(&self) -> Result<Mnemonic, MnemonicError> {
        Mnemonic::parse_in_normalized(Language::English, self.expose_phrase())
            .map_err(MnemonicError::Invalid)
    }

    /// Returns the master BIP32 fingerprint for the empty BIP39 passphrase.
    ///
    /// # Errors
    ///
    /// Returns an error if BIP32 master-key construction fails.
    pub fn fingerprint(&self) -> Result<Fingerprint, MnemonicError> {
        let mnemonic = self.to_mnemonic()?;
        let seed = Zeroizing::new(mnemonic.to_seed(""));
        let xpriv =
            Xpriv::new_master(Network::Bitcoin, seed.as_ref()).map_err(MnemonicError::Bip32)?;
        let xpub = Xpub::from_priv(&Secp256k1::new(), &xpriv);

        Ok(xpub.fingerprint())
    }
}

impl fmt::Debug for MnemonicSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MnemonicSecret([REDACTED])")
    }
}

/// Errors from mnemonic generation, parsing, or derivation.
#[derive(Debug, thiserror::Error)]
pub enum MnemonicError {
    /// Generated mnemonics support only 12 or 24 words.
    #[error("generated mnemonic word count must be 12 or 24, got {0}")]
    UnsupportedGeneratedWordCount(usize),
    /// A mnemonic word is not in the English BIP39 list.
    #[error("invalid BIP39 word: {0}")]
    InvalidWord(String),
    /// The mnemonic words or checksum are invalid.
    #[error("invalid BIP39 mnemonic: {0}")]
    Invalid(#[source] bip39::Error),
    /// Cove could not make final-word suggestions.
    #[error("cannot generate final-word suggestions: {0}")]
    FinalWord(String),
    /// BIP32 key construction failed.
    #[error("cannot derive BIP32 master key: {0}")]
    Bip32(#[source] bitcoin::bip32::Error),
}

/// Errors from recovery-word or standard `SeedQR` imports
#[derive(Debug, thiserror::Error)]
pub enum MnemonicImportError {
    /// Recovery words were invalid
    #[error(transparent)]
    Mnemonic(#[from] MnemonicError),
    /// Standard `SeedQR` digits were invalid
    #[error(transparent)]
    SeedQr(#[from] crate::SeedQrError),
}

#[cfg(test)]
mod tests {
    use super::{MnemonicImportError, MnemonicSecret};

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn secret_debug_never_contains_words() {
        let secret = MnemonicSecret::parse(TEST_MNEMONIC).expect("test mnemonic is valid");

        assert_eq!(format!("{secret:?}"), "MnemonicSecret([REDACTED])");
    }

    #[test]
    fn final_word_suggestions_use_cove_bip39() {
        let partial = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        let suggestions = MnemonicSecret::possible_final_words(partial)
            .expect("valid partial mnemonic has final-word suggestions");

        assert!(suggestions.iter().any(|word| word == "about"));
    }

    #[test]
    fn fingerprint_is_stable_for_same_mnemonic() {
        let first = MnemonicSecret::parse(TEST_MNEMONIC)
            .expect("test mnemonic is valid")
            .fingerprint()
            .expect("test mnemonic fingerprint is valid");
        let second = MnemonicSecret::parse(TEST_MNEMONIC)
            .expect("test mnemonic is valid")
            .fingerprint()
            .expect("test mnemonic fingerprint is valid");

        assert_eq!(first, second);
    }

    #[test]
    fn import_accepts_standard_seed_qr_digits() {
        let secret =
            MnemonicSecret::parse_import("000000000000000000000000000000000000000000000003")
                .expect("standard SeedQR import is valid");

        assert_eq!(secret.expose_phrase(), TEST_MNEMONIC);
    }

    #[test]
    fn numeric_import_reports_seed_qr_errors() {
        assert!(matches!(
            MnemonicSecret::parse_import("2048"),
            Err(MnemonicImportError::SeedQr(_))
        ));
    }
}
