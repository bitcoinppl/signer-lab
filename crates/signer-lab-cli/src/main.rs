mod cli;
mod commands;

use clap::Parser as _;
use color_eyre::eyre::Result;

use crate::cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    commands::run(Cli::parse()).await
}
