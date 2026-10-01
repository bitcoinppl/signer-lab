use std::str::FromStr as _;

use bitcoin::{Amount, ScriptBuf, bip32::Xpub};
use color_eyre::eyre::{Result, eyre};
use signer_lab_core::{
    AddressType, BulkPlanner, BulkRequest, Destination, DestinationScripts, ExternalDescriptor,
    FixedAddress, LabNetwork, SignerId,
};

use super::{
    output::{BulkCancellationOutput, BulkExecutionOutput, BulkOperationOutput, BulkPlanOutput},
    support,
};
use crate::cli::{
    BulkCommand, BulkPlanCommand, ConsolidationPlanArgs, DestinationCommand, TransferPlanArgs,
    WalletSelection,
};

pub(super) async fn run(repository: &mut support::Repository, command: BulkCommand) -> Result<()> {
    match command {
        BulkCommand::Plan {
            wallet,
            fee_rate,
            kind,
        } => plan(repository, wallet, fee_rate, kind).await,
        BulkCommand::List(selection) => {
            let signer = support::find_signer(repository, selection.signer).await?;
            let operations = repository.bulk_operations(signer.id).await?;
            let output = operations
                .into_iter()
                .map(BulkOperationOutput::from)
                .collect::<Vec<_>>();

            support::print_json(&output)
        }
        BulkCommand::Execute {
            wallet: selection,
            operation,
        } => {
            let signer = support::find_signer(repository, selection.signer.signer).await?;
            let report = repository
                .execute_bulk_operation(&signer, selection.network.into(), operation)
                .await?;

            support::print_json(&BulkExecutionOutput {
                operation_id: report.operation_id,
                txids: report.txids,
            })
        }
        BulkCommand::Cancel {
            wallet: selection,
            operation,
        } => {
            let signer = support::find_signer(repository, selection.signer.signer).await?;
            repository
                .cancel_bulk_operation(&signer, selection.network.into(), operation)
                .await?;

            support::print_json(&BulkCancellationOutput {
                cancelled_operation_id: operation,
            })
        }
    }
}

async fn plan(
    repository: &mut support::Repository,
    selection: WalletSelection,
    fee_rate: signer_lab_core::LabFeeRate,
    kind: BulkPlanCommand,
) -> Result<()> {
    let signer = support::find_signer(repository, selection.signer.signer).await?;
    let network = selection.network.into();
    repository
        .reconcile_bulk_reservations(&signer, network)
        .await?;
    let request = match kind {
        BulkPlanCommand::FanOut(args) => {
            let (amount, destinations) = transfer_destinations(repository, network, args).await?;
            BulkRequest::fan_out(amount, destinations)?
        }
        BulkPlanCommand::Batch(args) => {
            let (amount, destinations) = transfer_destinations(repository, network, args).await?;
            BulkRequest::batch(amount, destinations)?
        }
        BulkPlanCommand::Consolidate(ConsolidationPlanArgs { destination }) => {
            let script = destination_scripts(repository, network, destination, 1)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| eyre!("consolidation destination did not produce a script"))?;
            BulkRequest::consolidate_all(script)
        }
    };
    let operation_kind = request.kind();
    let mut wallet = repository.open_wallet(&signer, network)?;
    let plan =
        BulkPlanner::new(wallet.account_mut(AddressType::NativeSegwit)).plan(&request, fee_rate)?;
    let operation = repository
        .prepare_bulk_operation(signer.id, network, &plan)
        .await?;
    let reservation_error = plan
        .reserve_inputs(wallet.account_mut(AddressType::NativeSegwit))
        .err()
        .map(|error| error.to_string());

    support::print_json(&BulkPlanOutput {
        operation_id: operation.lifecycle.id,
        kind: operation_kind,
        network,
        plan,
        reservation_error,
    })
}

async fn transfer_destinations(
    repository: &mut support::Repository,
    network: LabNetwork,
    args: TransferPlanArgs,
) -> Result<(Amount, DestinationScripts)> {
    let scripts = destination_scripts(repository, network, args.destination, args.count).await?;
    let destinations = DestinationScripts::new(scripts)?;

    Ok((Amount::from_sat(args.amount), destinations))
}

async fn destination_scripts(
    repository: &mut support::Repository,
    network: LabNetwork,
    destination: DestinationCommand,
    count: usize,
) -> Result<Vec<ScriptBuf>> {
    match destination {
        DestinationCommand::Address { value } => {
            let address = FixedAddress::parse(&value, network)?;

            Ok(Destination::Fixed(address).derive_scripts(network, 0, count)?)
        }
        DestinationCommand::Xpub {
            value,
            address_type,
        } => {
            let xpub = Xpub::from_str(&value)?;
            let destination = Destination::AccountXpub {
                xpub,
                address_type: address_type.into(),
            };

            Ok(destination.derive_scripts(network, 0, count)?)
        }
        DestinationCommand::Descriptor { value } => {
            let descriptor = ExternalDescriptor::parse(&value, network)?;

            Ok(Destination::Descriptor(descriptor).derive_scripts(network, 0, count)?)
        }
        DestinationCommand::Signer {
            signer,
            address_type,
        } => {
            signer_destination_scripts(repository, network, signer, address_type.into(), count)
                .await
        }
    }
}

async fn signer_destination_scripts(
    repository: &mut support::Repository,
    network: LabNetwork,
    signer_id: SignerId,
    address_type: AddressType,
    count: usize,
) -> Result<Vec<ScriptBuf>> {
    let signer = support::find_signer(repository, signer_id).await?;
    let mut wallet = repository.open_wallet(&signer, network)?;
    let account = wallet.account_mut(address_type);
    let mut scripts = Vec::with_capacity(count);

    for _ in 0..count {
        let address = account.reveal_receive_address()?;
        let address = FixedAddress::parse(&address, network)?;
        scripts.extend(Destination::Fixed(address).derive_scripts(network, 0, 1)?);
    }

    Ok(scripts)
}
