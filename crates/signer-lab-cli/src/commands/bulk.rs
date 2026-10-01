use std::str::FromStr as _;

use bitcoin::{Amount, bip32::Xpub};
use color_eyre::eyre::Result;
use signer_lab_core::{
    BulkDestinationSpec, BulkShape, ExternalDescriptor, FixedAddress, InputReservation, LabNetwork,
    OutputCount, PlanBulk,
};

use super::{
    output::{
        BulkCancellationOutput, BulkDismissalOutput, BulkExecutionOutput, BulkOperationOutput,
        BulkPlanOutput,
    },
    support,
};
use crate::cli::{
    BulkCommand, BulkPlanCommand, ConsolidationPlanArgs, DestinationCommand, WalletSelection,
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
        BulkCommand::Dismiss {
            signer: selection,
            operation,
        } => {
            repository
                .acknowledge_failed_bulk_operation(selection.signer, operation)
                .await?;

            support::print_json(&BulkDismissalOutput {
                acknowledged_operation_id: operation,
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
    let (shape, destination) = match kind {
        BulkPlanCommand::FanOut(args) => (
            BulkShape::fan_out(
                OutputCount::try_from(args.count)?,
                Amount::from_sat(args.amount),
            )?,
            args.destination,
        ),
        BulkPlanCommand::Batch(args) => (
            BulkShape::batch(
                OutputCount::try_from(args.count)?,
                Amount::from_sat(args.amount),
            )?,
            args.destination,
        ),
        BulkPlanCommand::Consolidate(ConsolidationPlanArgs { destination }) => {
            (BulkShape::consolidate(), destination)
        }
    };

    let destination = destination_spec(destination, network)?;
    let result = repository
        .plan_bulk_operation(PlanBulk {
            signer,
            network,
            shape,
            destination,
            fee_rate,
        })
        .await?;

    let reservation_error = match result.reservation {
        InputReservation::Held => None,
        InputReservation::Failed(error) => Some(error),
    };

    support::print_json(&BulkPlanOutput {
        operation_id: result.operation_id,
        kind: result.plan.kind(),
        network,
        plan: result.plan,
        reservation_error,
        destination_range: result.destination_range,
    })
}

fn destination_spec(
    destination: DestinationCommand,
    network: LabNetwork,
) -> Result<BulkDestinationSpec> {
    Ok(match destination {
        DestinationCommand::Address { value } => {
            BulkDestinationSpec::Fixed(FixedAddress::parse(&value, network)?)
        }
        DestinationCommand::Xpub {
            value,
            address_type,
            start_index,
        } => BulkDestinationSpec::AccountXpub {
            xpub: Xpub::from_str(&value)?,
            address_type: address_type.into(),
            start_index,
        },
        DestinationCommand::Descriptor { value, start_index } => BulkDestinationSpec::Descriptor {
            descriptor: ExternalDescriptor::parse(&value, network)?,
            start_index,
        },
        DestinationCommand::Signer {
            signer,
            address_type,
        } => BulkDestinationSpec::SavedSigner {
            signer_id: signer,
            address_type: address_type.into(),
        },
    })
}
