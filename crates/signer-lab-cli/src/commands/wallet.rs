use color_eyre::eyre::Result;
use signer_lab_core::{SyncControl, WalletExporter, sync_account};

use super::{output::AddressOutput, output::WalletExportOutput, support};
use crate::cli::WalletCommand;

pub(super) async fn run(
    repository: &mut support::Repository,
    command: WalletCommand,
) -> Result<()> {
    match command {
        WalletCommand::Status(selection) => {
            let (_, wallet) = support::open_wallet(repository, &selection).await?;
            let snapshots = support::address_types()
                .map(|address_type| wallet.account(address_type).snapshot());

            support::print_json(&snapshots)
        }
        WalletCommand::Sync {
            wallet: selection,
            address_type,
        } => {
            let network = selection.network.into();
            let (_, mut wallet) = support::open_wallet(repository, &selection).await?;
            let address_types = address_type.map_or_else(
                || support::address_types().to_vec(),
                |address_type| vec![address_type.into()],
            );
            let control = SyncControl::new();
            let mut snapshots = Vec::with_capacity(address_types.len());

            for address_type in address_types {
                let report =
                    sync_account(wallet.account_mut(address_type), network, &control, |_| {})
                        .await?;
                snapshots.push(report.snapshot);
            }

            support::print_json(&snapshots)
        }
        WalletCommand::Address {
            wallet: selection,
            address_type,
        } => {
            let (_, mut wallet) = support::open_wallet(repository, &selection).await?;
            let address = wallet
                .account_mut(address_type.into())
                .reveal_receive_address()?;

            support::print_json(&AddressOutput { address })
        }
        WalletCommand::Export {
            wallet: selection,
            output,
            format,
        } => {
            let network = selection.network.into();
            let (_, wallet) = support::open_wallet(repository, &selection).await?;
            let accounts = wallet.public_accounts()?;
            let artifact = WalletExporter::new(network, &accounts)?.export(format.into())?;
            artifact.write_to_path(&output)?;

            support::print_json(&WalletExportOutput {
                kind: artifact.kind,
                media_type: artifact.media_type,
                output,
            })
        }
    }
}
