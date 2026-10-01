use std::{fmt, str::FromStr};

use bitcoin::{Network, bip32::Fingerprint};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stable identifier for a virtual signing device
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignerId(Uuid);

impl SignerId {
    /// Creates a random signer identifier
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SignerId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SignerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SignerId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// A checked user-visible signer name
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SignerName(String);

impl TryFrom<String> for SignerName {
    type Error = SignerNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl SignerName {
    /// Maximum number of Unicode scalar values in a signer name
    pub const MAX_CHARS: usize = 64;

    /// Validates and creates a signer name
    ///
    /// # Errors
    ///
    /// Returns an error for an empty name or a name longer than 64 characters
    pub fn new(value: impl Into<String>) -> Result<Self, SignerNameError> {
        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(SignerNameError::Empty);
        }

        let char_count = trimmed.chars().count();
        if char_count > Self::MAX_CHARS {
            return Err(SignerNameError::TooLong { char_count });
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the checked name
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SignerName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Errors from signer-name validation
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignerNameError {
    /// The name contains no visible text
    #[error("signer name cannot be empty")]
    Empty,
    /// The name is longer than the supported limit
    #[error("signer name has {char_count} characters; maximum is 64")]
    TooLong {
        /// Actual character count
        char_count: usize,
    },
}

/// A checked BIP32 account index
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32")]
pub struct AccountIndex(u32);

impl TryFrom<u32> for AccountIndex {
    type Error = AccountIndexError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AccountIndex {
    /// Highest account index exposed by version 1
    pub const MAX: u32 = 9_999;

    /// Validates an account index
    ///
    /// # Errors
    ///
    /// Returns an error when the value is greater than 9,999
    pub const fn new(value: u32) -> Result<Self, AccountIndexError> {
        if value > Self::MAX {
            return Err(AccountIndexError(value));
        }

        Ok(Self(value))
    }

    /// Returns the numeric account index
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An invalid account index
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("account index {0} is greater than 9999")]
pub struct AccountIndexError(u32);

/// A network supported by Signer Lab version 1
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabNetwork {
    /// `MutinyNet`, which uses Bitcoin Signet consensus rules
    #[default]
    MutinyNet,
    /// Bitcoin Testnet4
    Testnet4,
}

impl LabNetwork {
    /// Returns the BDK network value
    #[must_use]
    pub const fn bitcoin_network(self) -> Network {
        match self {
            Self::MutinyNet => Network::Signet,
            Self::Testnet4 => Network::Testnet4,
        }
    }

    /// Returns the default Esplora API endpoint
    #[must_use]
    pub const fn esplora_url(self) -> &'static str {
        match self {
            Self::MutinyNet => "https://mutinynet.com/api",
            Self::Testnet4 => "https://mempool.space/testnet4/api",
        }
    }

    /// Returns the BIP44 coin type
    #[must_use]
    pub const fn coin_type(self) -> u32 {
        1
    }
}

impl fmt::Display for LabNetwork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::MutinyNet => "MutinyNet",
            Self::Testnet4 => "Testnet4",
        };

        formatter.write_str(name)
    }
}

/// A single-signature account address type supported by Cove imports
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressType {
    /// BIP44 P2PKH addresses
    Legacy,
    /// BIP49 P2SH-P2WPKH addresses
    WrappedSegwit,
    /// BIP84 P2WPKH addresses
    #[default]
    NativeSegwit,
}

impl AddressType {
    /// All supported address types in account order
    pub const ALL: [Self; 3] = [Self::Legacy, Self::WrappedSegwit, Self::NativeSegwit];

    /// Returns the BIP purpose value
    #[must_use]
    pub const fn purpose(self) -> u32 {
        match self {
            Self::Legacy => 44,
            Self::WrappedSegwit => 49,
            Self::NativeSegwit => 84,
        }
    }
}

/// Public metadata for one virtual signing device
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualSigner {
    /// Stable signer identifier
    pub id: SignerId,
    /// User-visible signer name
    pub name: SignerName,
    /// Master BIP32 fingerprint
    pub fingerprint: Fingerprint,
    /// BIP44 account index used for all supported address types
    pub account_index: AccountIndex,
    /// Creation timestamp
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::{AccountIndex, LabNetwork, SignerName};

    #[test]
    fn signer_name_trims_visible_text() {
        let name = SignerName::new("  Test signer  ").expect("test signer name is valid");

        assert_eq!(name.as_str(), "Test signer");
    }

    #[test]
    fn account_index_enforces_ui_limit() {
        assert!(AccountIndex::new(9_999).is_ok());
        assert!(AccountIndex::new(10_000).is_err());
    }

    #[test]
    fn deserialization_preserves_checked_metadata_invariants() {
        assert!(serde_json::from_str::<AccountIndex>("2147483648").is_err());
        assert!(serde_json::from_str::<SignerName>("\"  \"").is_err());
        let index = AccountIndex::new(123).expect("valid index");
        let json = serde_json::to_string(&index).expect("index serializes");
        assert_eq!(
            serde_json::from_str::<AccountIndex>(&json).expect("index decodes"),
            index
        );
    }

    #[test]
    fn version_one_networks_are_never_mainnet() {
        for network in [LabNetwork::MutinyNet, LabNetwork::Testnet4] {
            assert_ne!(network.bitcoin_network(), bitcoin::Network::Bitcoin);
            assert_eq!(network.coin_type(), 1);
        }
    }
}
