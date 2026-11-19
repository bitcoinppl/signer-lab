use bitcoin::consensus::encode::serialize_hex;
use color_eyre::eyre::Result;
use signer_lab_core::{PendingPsbt, read_psbt, write_psbt};

use super::{output::PsbtSignOutput, support};
use crate::cli::PsbtCommand;

pub(super) async fn run(repository: &mut support::Repository, command: PsbtCommand) -> Result<()> {
    match command {
        PsbtCommand::Review { wallet, input } => {
            let (_, wallet) = support::open_wallet(repository, &wallet).await?;
            let psbt = read_psbt(&input)?;
            let reviewed = PendingPsbt::new(psbt).review(&wallet)?;

            support::print_json(reviewed.review())
        }
        PsbtCommand::Sign {
            wallet: selection,
            input,
            output,
            format,
        } => {
            let (_, wallet) = support::open_wallet(repository, &selection).await?;
            let psbt = read_psbt(&input)?;
            let reviewed = PendingPsbt::new(psbt).review(&wallet)?;
            let signed = reviewed.sign(&wallet, true)?;
            write_psbt(&output, signed.psbt(), format.into())?;

            support::print_json(&PsbtSignOutput {
                output,
                format: format.name(),
                report: signed.report(),
                review: signed.review().clone(),
                transaction: signed.transaction().map(serialize_hex),
            })
        }
    }
}
