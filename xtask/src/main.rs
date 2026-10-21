use std::ffi::OsString;

use clap::{Parser, Subcommand};
use color_eyre::Result;

mod macos;

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Build automation for Signer Lab")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and sign the macOS application bundle
    BundleMacos {
        /// Select where to write the application bundle
        #[arg(long, value_enum, default_value_t = macos::BundleDestination::Scratch)]
        destination: macos::BundleDestination,
    },
    /// Build, sign, and run the CLI with a Keychain identity that survives rebuilds
    RunCli {
        /// Arguments passed to the CLI
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    match cli.command {
        Commands::BundleMacos { destination } => macos::bundle(destination),
        Commands::RunCli { args } => macos::run_cli(args),
    }
}
