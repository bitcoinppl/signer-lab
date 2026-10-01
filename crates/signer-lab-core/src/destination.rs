use std::str::FromStr as _;

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

use crate::{AddressType, LabNetwork};

/// A checked fixed destination address
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedAddress {
    encoded: String,
}

impl FixedAddress {
    /// Parses an address for the active network
    ///
    /// # Errors
    ///
    /// Returns an error when the address is invalid for the active network
    pub fn parse(value: &str, network: LabNetwork) -> Result<Self, DestinationError> {
        let address = Address::from_str(value).map_err(DestinationError::Address)?;
        let checked = address
            .require_network(network.bitcoin_network())
            .map_err(|_| DestinationError::WrongNetwork(network))?;

        Ok(Self {
            encoded: checked.to_string(),
        })
    }

    /// Returns the encoded address
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

/// A checked ranged external descriptor
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedDescriptor")]
pub struct ExternalDescriptor {
    encoded: String,
}

#[derive(Deserialize)]
struct UncheckedDescriptor {
    encoded: String,
}

impl TryFrom<UncheckedDescriptor> for ExternalDescriptor {
    type Error = DestinationError;

    fn try_from(value: UncheckedDescriptor) -> Result<Self, Self::Error> {
        Self::parse(&value.encoded, LabNetwork::MutinyNet)
    }
}

impl ExternalDescriptor {
    /// Parses a descriptor and verifies that it contains a derivation wildcard
    ///
    /// # Errors
    ///
    /// Returns an error when the descriptor is invalid, fixed, or for the wrong key network
    pub fn parse(value: &str, network: LabNetwork) -> Result<Self, DestinationError> {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let (descriptor, key_map) = value
            .into_wallet_descriptor(&secp, network.bitcoin_network().into())
            .map_err_str(DestinationError::Descriptor)?;

        if !descriptor.has_wildcard() {
            return Err(DestinationError::DescriptorNotRanged);
        }

        if !key_map.is_empty() {
            return Err(DestinationError::Descriptor(
                "destination descriptor must contain only public keys".to_owned(),
            ));
        }

        Ok(Self {
            encoded: value.to_owned(),
        })
    }

    /// Returns the checked descriptor text
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

/// A destination for fan-out, batch, or consolidation planning
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Destination {
    /// The same fixed address for each output
    Fixed(FixedAddress),
    /// Fresh `/0/index` addresses derived from an account XPUB
    AccountXpub {
        /// Account extended public key
        xpub: Xpub,
        /// Address wrapper applied to the key
        address_type: AddressType,
    },
    /// Fresh addresses from a ranged external descriptor
    Descriptor(ExternalDescriptor),
}

impl Destination {
    /// Derives the requested public destination scripts
    ///
    /// Fixed addresses repeat. XPUB and ranged-descriptor destinations advance from `start_index`
    ///
    /// # Errors
    ///
    /// Returns an error for invalid derivation indices, or descriptor derivation failures
    pub(crate) fn derive_scripts(
        &self,
        network: LabNetwork,
        start_index: u32,
        count: usize,
    ) -> Result<Vec<ScriptBuf>, DestinationError> {
        if count == 0 {
            return Err(DestinationError::EmptyRequest);
        }

        match self {
            Self::Fixed(address) => {
                let address = Address::from_str(address.as_str())
                    .map_err(DestinationError::Address)?
                    .require_network(network.bitcoin_network())
                    .map_err(|_| DestinationError::WrongNetwork(network))?;

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
    if xpub.network != network.bitcoin_network().into() {
        return Err(DestinationError::WrongNetwork(network));
    }

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

/// Errors from destination parsing and network checks
#[derive(Debug, thiserror::Error)]
pub enum DestinationError {
    /// Address text is invalid
    #[error("invalid address: {0}")]
    Address(#[source] bitcoin::address::ParseError),
    /// The address or descriptor is not valid for the active network
    #[error("destination is not valid for {0}")]
    WrongNetwork(LabNetwork),
    /// Descriptor text is invalid
    #[error("invalid descriptor: {0}")]
    Descriptor(String),
    /// A destination descriptor must derive more than one address
    #[error("destination descriptor must contain a wildcard")]
    DescriptorNotRanged,
    /// At least one destination script is required
    #[error("destination count cannot be zero")]
    EmptyRequest,
    /// A requested child index is outside the non-hardened BIP32 range
    #[error("destination derivation index is too large")]
    IndexTooLarge,
    /// XPUB child derivation failed
    #[error("destination key derivation failed: {0}")]
    Bip32(#[source] bitcoin::bip32::Error),
}

#[cfg(test)]
mod tests {
    use super::{Destination, ExternalDescriptor, FixedAddress};
    use crate::AddressType;
    use crate::LabNetwork;
    use bitcoin::{
        Network,
        bip32::{Xpriv, Xpub},
        secp256k1::Secp256k1,
    };

    #[test]
    fn destinations_reject_private_descriptors_and_mainnet_keys() {
        let master = Xpriv::new_master(Network::Bitcoin, &[0; 32]).expect("master key");
        let xpub = Xpub::from_priv(&Secp256k1::new(), &master);
        let destination = Destination::AccountXpub {
            xpub,
            address_type: AddressType::NativeSegwit,
        };

        assert!(
            destination
                .derive_scripts(LabNetwork::MutinyNet, 0, 1)
                .is_err()
        );

        let master = Xpriv::new_master(Network::Signet, &[0; 32]).expect("test master key");
        assert!(
            ExternalDescriptor::parse(&format!("wpkh({master}/0/*)"), LabNetwork::MutinyNet)
                .is_err()
        );

        let encoded = serde_json::json!({"encoded": format!("wpkh({master}/0/*)")});
        assert!(serde_json::from_value::<ExternalDescriptor>(encoded).is_err());
        let address: FixedAddress =
            serde_json::from_str(r#"{"encoded":"bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"}"#)
                .expect("address document");

        assert!(
            Destination::Fixed(address)
                .derive_scripts(LabNetwork::MutinyNet, 0, 1)
                .is_err()
        );
    }

    #[test]
    fn fixed_address_is_bound_to_active_network() {
        let mainnet = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

        assert!(FixedAddress::parse(mainnet, LabNetwork::MutinyNet).is_err());
    }
}
