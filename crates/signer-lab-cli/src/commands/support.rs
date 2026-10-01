use std::{io::Write as _, path::PathBuf};

use color_eyre::eyre::{Result, WrapErr as _, eyre};
use directories::ProjectDirs;
use serde::Serialize;
use signer_lab_core::{
    AddressType, ExportKind, KeychainSecretStore, LabNetwork, LabStore, NetworkWallet,
    PsbtFileFormat, SignerId, SignerRepository, VirtualSigner,
};

use crate::cli::{AddressTypeArg, ExportCommand, NetworkArg, PsbtFormatArg, WalletSelection};

pub(super) type Repository = SignerRepository<KeychainSecretStore>;

pub(super) async fn open_repository(override_path: Option<PathBuf>) -> Result<Repository> {
    let data_dir = data_dir(override_path)?;
    std::fs::create_dir_all(&data_dir)
        .wrap_err_with(|| format!("cannot create data directory {}", data_dir.display()))?;
    let store = LabStore::open_or_create(data_dir.join("metadata.db")).await?;

    Ok(SignerRepository::new(
        store,
        KeychainSecretStore,
        data_dir.join("wallets"),
    ))
}

pub(super) async fn find_signer(
    repository: &mut Repository,
    signer_id: SignerId,
) -> Result<signer_lab_core::VirtualSigner> {
    repository
        .signers()
        .await?
        .into_iter()
        .find(|signer| signer.id == signer_id)
        .ok_or_else(|| eyre!("signer {signer_id} does not exist"))
}

pub(super) async fn open_wallet(
    repository: &mut Repository,
    selection: &WalletSelection,
) -> Result<(VirtualSigner, NetworkWallet)> {
    let signer = find_signer(repository, selection.signer.signer).await?;
    let network = selection.network.into();
    repository
        .reconcile_bulk_reservations(&signer, network)
        .await?;
    let wallet = repository.open_wallet(&signer, network)?;

    Ok((signer, wallet))
}

pub(super) fn data_dir(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }

    ProjectDirs::from("org", "bitcoinppl", "Signer Lab")
        .map(|paths| paths.data_local_dir().to_owned())
        .ok_or_else(|| eyre!("application data directory is not available"))
}

pub(super) fn print_json(value: &impl Serialize) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    writeln!(output)?;

    Ok(())
}

pub(super) const fn address_types() -> [AddressType; 3] {
    [
        AddressType::Legacy,
        AddressType::WrappedSegwit,
        AddressType::NativeSegwit,
    ]
}

impl From<NetworkArg> for LabNetwork {
    fn from(value: NetworkArg) -> Self {
        match value {
            NetworkArg::Mutinynet => Self::MutinyNet,
            NetworkArg::Testnet4 => Self::Testnet4,
        }
    }
}

impl From<AddressTypeArg> for AddressType {
    fn from(value: AddressTypeArg) -> Self {
        match value {
            AddressTypeArg::Legacy => Self::Legacy,
            AddressTypeArg::WrappedSegwit => Self::WrappedSegwit,
            AddressTypeArg::NativeSegwit => Self::NativeSegwit,
        }
    }
}

impl From<ExportCommand> for ExportKind {
    fn from(value: ExportCommand) -> Self {
        match value {
            ExportCommand::ColdcardJson => Self::ColdcardGenericJson,
            ExportCommand::WasabiJson => Self::WasabiJson,
            ExportCommand::JadeAccount => Self::JadeCryptoAccount,
            ExportCommand::Multipath(selection) => {
                Self::MultipathDescriptor(selection.address_type.into())
            }
            ExportCommand::DescriptorPair(selection) => {
                Self::DescriptorPair(selection.address_type.into())
            }
            ExportCommand::ElectrumJson(selection) => {
                Self::ElectrumJson(selection.address_type.into())
            }
            ExportCommand::Xpub(selection) => Self::AccountXpub(selection.address_type.into()),
            ExportCommand::Bip380(selection) => {
                Self::Bip380KeyExpression(selection.address_type.into())
            }
            ExportCommand::JadeHdkey(selection) => {
                Self::JadeCryptoHdkey(selection.address_type.into())
            }
        }
    }
}

impl From<PsbtFormatArg> for PsbtFileFormat {
    fn from(value: PsbtFormatArg) -> Self {
        match value {
            PsbtFormatArg::Base64 => Self::Base64,
            PsbtFormatArg::Binary => Self::Binary,
            PsbtFormatArg::Bbqr => Self::Bbqr,
            PsbtFormatArg::Ur => Self::Ur,
        }
    }
}

impl PsbtFormatArg {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Base64 => "base64",
            Self::Binary => "binary",
            Self::Bbqr => "bbqr",
            Self::Ur => "ur",
        }
    }
}
