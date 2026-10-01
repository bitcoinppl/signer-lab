use std::{fmt, str::FromStr as _};

use bdk_wallet::{
    bitcoin::{
        Address, CompressedPublicKey, ScriptBuf,
        bip32::{ChildNumber, DerivationPath, Xpub},
        key::Secp256k1,
    },
    descriptor::IntoWalletDescriptor,
};
use cove_util::ResultExt as _;
use serde::{Deserialize, Serialize};

use crate::{AddressType, LabNetwork, SignerId};

/// A checked fixed destination address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedAddress {
    encoded: String,
}

impl FixedAddress {
    /// Parses an address for the active network.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is invalid for the active network.
    pub fn parse(value: &str, network: LabNetwork) -> Result<Self, DestinationError> {
        let address = Address::from_str(value).map_err(DestinationError::Address)?;
        let checked = address
            .require_network(network.bitcoin_network())
            .map_err(|_| DestinationError::WrongNetwork(network))?;

        Ok(Self {
            encoded: checked.to_string(),
        })
    }

    /// Returns the encoded address.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

/// A checked ranged external descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalDescriptor {
    encoded: String,
}

impl ExternalDescriptor {
    /// Parses a descriptor and verifies that it contains a derivation wildcard.
    ///
    /// # Errors
    ///
    /// Returns an error when the descriptor is invalid, fixed, or for the wrong key network.
    pub fn parse(value: &str, network: LabNetwork) -> Result<Self, DestinationError> {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let (descriptor, _) = value
            .into_wallet_descriptor(&secp, network.bitcoin_network().into())
            .map_err_str(DestinationError::Descriptor)?;

        if !descriptor.has_wildcard() {
            return Err(DestinationError::DescriptorNotRanged);
        }

        Ok(Self {
            encoded: value.to_owned(),
        })
    }

    /// Returns the checked descriptor text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

/// A destination for fan-out, batch, or consolidation planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    /// A fresh receive address from another saved signer.
    SavedSigner {
        /// Destination signer.
        signer_id: SignerId,
        /// Destination account address type.
        address_type: AddressType,
    },
    /// The same fixed address for each output.
    Fixed(FixedAddress),
    /// Fresh `/0/index` addresses derived from an account XPUB.
    AccountXpub {
        /// Account extended public key.
        xpub: Xpub,
        /// Address wrapper applied to the key.
        address_type: AddressType,
    },
    /// Fresh addresses from a ranged external descriptor.
    Descriptor(ExternalDescriptor),
}

impl Destination {
    /// Derives the requested public destination scripts.
    ///
    /// Fixed addresses repeat. XPUB and ranged-descriptor destinations advance from `start_index`.
    /// Saved signers must first be resolved through their owning BDK wallet.
    ///
    /// # Errors
    ///
    /// Returns an error for saved signers, invalid derivation indices, or descriptor derivation failures.
    pub fn derive_scripts(
        &self,
        network: LabNetwork,
        start_index: u32,
        count: usize,
    ) -> Result<Vec<ScriptBuf>, DestinationError> {
        if count == 0 {
            return Err(DestinationError::EmptyRequest);
        }

        match self {
            Self::SavedSigner { .. } => Err(DestinationError::SavedSignerNeedsWallet),
            Self::Fixed(address) => {
                let address = Address::from_str(address.as_str())
                    .map_err(DestinationError::Address)?
                    .assume_checked();

                Ok(vec![address.script_pubkey(); count])
            }
            Self::AccountXpub { xpub, address_type } => {
                derive_xpub_scripts(*xpub, *address_type, network, start_index, count)
            }
            Self::Descriptor(descriptor) => {
                derive_descriptor_scripts(descriptor, network, start_index, count)
            }
        }
    }
}

fn derive_xpub_scripts(
    xpub: Xpub,
    address_type: AddressType,
    network: LabNetwork,
    start_index: u32,
    count: usize,
) -> Result<Vec<ScriptBuf>, DestinationError> {
    let secp = Secp256k1::new();

    checked_indices(start_index, count)?
        .map(|index| {
            let path = DerivationPath::from(vec![
                ChildNumber::from_normal_idx(0).expect("zero is a valid child index"),
                ChildNumber::from_normal_idx(index).map_err(|_| DestinationError::IndexTooLarge)?,
            ]);
            let child = xpub
                .derive_pub(&secp, &path)
                .map_err(DestinationError::Bip32)?;
            let compressed = CompressedPublicKey(child.public_key);
            let address = match address_type {
                AddressType::Legacy => Address::p2pkh(
                    bitcoin::PublicKey::new(child.public_key),
                    network.bitcoin_network(),
                ),
                AddressType::WrappedSegwit => {
                    Address::p2shwpkh(&compressed, network.bitcoin_network())
                }
                AddressType::NativeSegwit => {
                    Address::p2wpkh(&compressed, network.bitcoin_network())
                }
            };

            Ok(address.script_pubkey())
        })
        .collect()
}

fn derive_descriptor_scripts(
    ranged: &ExternalDescriptor,
    network: LabNetwork,
    start_index: u32,
    count: usize,
) -> Result<Vec<ScriptBuf>, DestinationError> {
    let secp = Secp256k1::new();
    let (descriptor, _) = ranged
        .as_str()
        .into_wallet_descriptor(&secp, network.bitcoin_network().into())
        .map_err_str(DestinationError::Descriptor)?;

    checked_indices(start_index, count)?
        .map(|index| {
            let definite = descriptor
                .at_derivation_index(index)
                .map_err_str(DestinationError::Descriptor)?;
            let derived = definite
                .derived_descriptor(&secp)
                .map_err_str(DestinationError::Descriptor)?;

            Ok(derived.script_pubkey())
        })
        .collect()
}

fn checked_indices(
    start_index: u32,
    count: usize,
) -> Result<impl Iterator<Item = u32>, DestinationError> {
    let count = u32::try_from(count).map_err(|_| DestinationError::IndexTooLarge)?;
    let end = start_index
        .checked_add(count)
        .ok_or(DestinationError::IndexTooLarge)?;
    if end > (1 << 31) {
        return Err(DestinationError::IndexTooLarge);
    }

    Ok(start_index..end)
}

/// Errors from destination parsing and network checks.
#[derive(Debug, thiserror::Error)]
pub enum DestinationError {
    /// Address text is invalid.
    #[error("invalid address: {0}")]
    Address(#[source] bitcoin::address::ParseError),
    /// The address or descriptor is not valid for the active network.
    #[error("destination is not valid for {0}")]
    WrongNetwork(LabNetwork),
    /// Descriptor text is invalid.
    #[error("invalid descriptor: {0}")]
    Descriptor(String),
    /// A destination descriptor must derive more than one address.
    #[error("destination descriptor must contain a wildcard")]
    DescriptorNotRanged,
    /// At least one destination script is required.
    #[error("destination count cannot be zero")]
    EmptyRequest,
    /// Saved-signer destinations require access to that signer's BDK wallet.
    #[error("saved signer destination must be resolved through its wallet")]
    SavedSignerNeedsWallet,
    /// A requested child index is outside the non-hardened BIP32 range.
    #[error("destination derivation index is too large")]
    IndexTooLarge,
    /// XPUB child derivation failed.
    #[error("destination key derivation failed: {0}")]
    Bip32(#[source] bitcoin::bip32::Error),
}

impl fmt::Display for Destination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SavedSigner { .. } => formatter.write_str("Saved signer"),
            Self::Fixed(address) => formatter.write_str(address.as_str()),
            Self::AccountXpub { xpub, .. } => xpub.fmt(formatter),
            Self::Descriptor(descriptor) => formatter.write_str(descriptor.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FixedAddress;
    use crate::LabNetwork;

    #[test]
    fn fixed_address_is_bound_to_active_network() {
        let mainnet = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

        assert!(FixedAddress::parse(mainnet, LabNetwork::MutinyNet).is_err());
    }
}
