mod bulk;
mod key_teleport;
mod output;
mod psbt;
mod signer;
mod support;
mod wallet;

use color_eyre::eyre::Result;

use crate::cli::{Cli, Command};

pub(crate) async fn run(cli: Cli) -> Result<()> {
    let data_dir = support::data_dir(cli.data_dir)?;
    let mut repository = support::open_repository(Some(data_dir.clone())).await?;

    match cli.command {
        Command::Signer { command } => signer::run(&mut repository, command).await,
        Command::Wallet { command } => wallet::run(&mut repository, command).await,
        Command::Psbt { command } => psbt::run(&mut repository, command).await,
        Command::Bulk { command } => bulk::run(&mut repository, command).await,
        Command::KeyTeleport { command } => {
            key_teleport::run(&mut repository, &data_dir, command).await
        }
    }
}
