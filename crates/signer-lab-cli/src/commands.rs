mod bulk;
mod output;
mod psbt;
mod signer;
mod support;
mod wallet;

use color_eyre::eyre::Result;

use crate::cli::{Cli, Command};

pub(crate) async fn run(cli: Cli) -> Result<()> {
    let mut repository = support::open_repository(cli.data_dir).await?;

    match cli.command {
        Command::Signer { command } => signer::run(&mut repository, command).await,
        Command::Wallet { command } => wallet::run(&mut repository, command).await,
        Command::Psbt { command } => psbt::run(&mut repository, command).await,
        Command::Bulk { command } => bulk::run(&mut repository, command).await,
    }
}
