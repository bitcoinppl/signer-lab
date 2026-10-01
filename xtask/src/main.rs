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
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    match cli.command {
        Commands::BundleMacos { destination } => macos::bundle(destination),
    }
}
