use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use signer_lab_core::{
    DestinationScripts, KeyTeleportRequest, KeyTeleportResponse, LabFeeRate, NumericCode,
    OperationId, SignerId, TeleportPassword,
};

#[derive(Debug, Parser)]
#[command(
    name = "signer-lab-cli",
    version,
    about = "Automate Signer Lab wallet tests"
)]
pub(crate) struct Cli {
    /// Override the shared Signer Lab application data directory
    #[arg(long, global = true, value_name = "DIRECTORY")]
    pub(crate) data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Create, import, inspect, or delete virtual signers
    Signer {
        #[command(subcommand)]
        command: SignerCommand,
    },
    /// Inspect, synchronize, or export a signer wallet
    Wallet {
        #[command(subcommand)]
        command: WalletCommand,
    },
    /// Review or sign a PSBT file
    Psbt {
        #[command(subcommand)]
        command: PsbtCommand,
    },
    /// Plan, execute, inspect, or cancel bulk transactions
    Bulk {
        #[command(subcommand)]
        command: BulkCommand,
    },
    /// Send or receive recovery words with COLDCARD Key Teleport
    KeyTeleport {
        #[command(subcommand)]
        command: KeyTeleportCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum KeyTeleportCommand {
    /// Encrypt a signer's recovery words for a receiver request
    Send {
        #[arg(long)]
        signer: SignerId,
        /// Receiver QR text or keyteleport.com link
        #[arg(long, value_name = "TEXT")]
        request: KeyTeleportRequest,
        /// Eight-digit code shown by the receiver
        #[arg(long)]
        code: NumericCode,
    },
    /// Receive recovery words from another device as a new signer
    Receive {
        #[command(subcommand)]
        command: KeyTeleportReceiveCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum KeyTeleportReceiveCommand {
    /// Start a receive session, replacing any earlier one, and print its request
    Start,
    /// Print the request of the active receive session
    Show,
    /// Decrypt a sender response and import its recovery words
    Finish {
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 0)]
        account_index: u32,
        /// Sender QR text or keyteleport.com link
        #[arg(long, value_name = "TEXT")]
        response: KeyTeleportResponse,
        /// Teleport password shown by the sender
        #[arg(long)]
        password: TeleportPassword,
    },
    /// Delete the active receive session
    Cancel,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SignerCommand {
    /// List saved signers as JSON
    List,
    /// Generate a signer and print its recovery words once
    Generate {
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 0)]
        account_index: u32,
        #[arg(long, default_value_t = 12, value_parser = ["12", "24"])]
        words: usize,
    },
    /// Import recovery words or standard seed QR digits from a UTF-8 file
    Import {
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 0)]
        account_index: u32,
        #[arg(long, value_name = "FILE")]
        mnemonic_file: PathBuf,
    },
    /// Reveal recovery words for a saved signer
    Reveal(SignerSelection),
    /// Export a saved signer as standard seed QR digits
    SeedQr(SignerSelection),
    /// Delete signer metadata and its Keychain secret
    Delete(SignerSelection),
}

#[derive(Debug, Clone, Args)]
pub(crate) struct SignerSelection {
    #[arg(long)]
    pub(crate) signer: SignerId,
}

#[derive(Debug, Clone, Args)]
pub(crate) struct WalletSelection {
    #[command(flatten)]
    pub(crate) signer: SignerSelection,
    #[arg(long, value_enum, default_value_t = NetworkArg::Mutinynet)]
    pub(crate) network: NetworkArg,
}

#[derive(Debug, Subcommand)]
pub(crate) enum WalletCommand {
    /// Print all account balances, addresses, and UTXO counts
    Status(WalletSelection),
    /// Synchronize one account or all accounts with Esplora
    Sync {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long, value_enum)]
        address_type: Option<AddressTypeArg>,
    },
    /// Reveal and persist the next receive address
    Address {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long, value_enum, default_value_t = AddressTypeArg::NativeSegwit)]
        address_type: AddressTypeArg,
    },
    /// Write a Cove-compatible wallet export to a file
    Export {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long, value_name = "FILE")]
        output: PathBuf,
        #[command(subcommand)]
        format: ExportCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ExportCommand {
    /// Coldcard generic JSON
    ColdcardJson,
    /// Wasabi JSON
    WasabiJson,
    /// Jade `crypto-account` UR frames
    JadeAccount,
    /// BIP389 multipath descriptor
    Multipath(AddressTypeSelection),
    /// External and internal descriptors on separate lines
    DescriptorPair(AddressTypeSelection),
    /// Electrum single-signature JSON
    ElectrumJson(AddressTypeSelection),
    /// Bare account XPUB
    Xpub(AddressTypeSelection),
    /// BIP380 account-key expression
    Bip380(AddressTypeSelection),
    /// Jade `crypto-hdkey` UR frames
    JadeHdkey(AddressTypeSelection),
}

#[derive(Debug, Clone, Args)]
pub(crate) struct AddressTypeSelection {
    #[arg(long, value_enum, default_value_t = AddressTypeArg::NativeSegwit)]
    pub(crate) address_type: AddressTypeArg,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PsbtCommand {
    /// Decode a PSBT file and print its review facts
    Review {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
    },
    /// Review, sign, and write a PSBT file
    Sign {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
        #[arg(long, value_name = "FILE")]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = PsbtFormatArg::Base64)]
        format: PsbtFormatArg,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum BulkCommand {
    /// Save a plan and reserve its inputs
    Plan {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long, default_value = "0.5")]
        fee_rate: LabFeeRate,
        #[command(subcommand)]
        kind: BulkPlanCommand,
    },
    /// List saved operations for a signer
    List(SignerSelection),
    /// Sign and broadcast a saved operation
    Execute {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long)]
        operation: OperationId,
    },
    /// Dismiss a failed operation without changing its saved state
    Dismiss {
        #[command(flatten)]
        signer: SignerSelection,
        #[arg(long)]
        operation: OperationId,
    },
    /// Cancel an operation and release its inputs
    Cancel {
        #[command(flatten)]
        wallet: WalletSelection,
        #[arg(long)]
        operation: OperationId,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum BulkPlanCommand {
    /// Create one transaction with many outputs
    FanOut(TransferPlanArgs),
    /// Create one independent transaction per output
    Batch(TransferPlanArgs),
    /// Send all available UTXOs to one destination
    Consolidate(ConsolidationPlanArgs),
}

#[derive(Debug, Args)]
pub(crate) struct TransferPlanArgs {
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub(crate) amount: u64,
    #[arg(long, value_parser = destination_count)]
    pub(crate) count: usize,
    #[command(subcommand)]
    pub(crate) destination: DestinationCommand,
}

fn destination_count(value: &str) -> Result<usize, String> {
    let count = value
        .parse::<usize>()
        .map_err(|_| "destination count must be an integer".to_owned())?;

    if !(1..=DestinationScripts::MAX_COUNT).contains(&count) {
        return Err(format!(
            "destination count must be between 1 and {}",
            DestinationScripts::MAX_COUNT
        ));
    }

    Ok(count)
}

#[derive(Debug, Args)]
pub(crate) struct ConsolidationPlanArgs {
    #[command(subcommand)]
    pub(crate) destination: DestinationCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DestinationCommand {
    /// Send to one address, repeated when a count is present
    Address { value: String },
    /// Derive destinations from an account XPUB
    Xpub {
        value: String,
        #[arg(long, default_value_t = 0)]
        start_index: u32,
        #[arg(long, value_enum, default_value_t = AddressTypeArg::NativeSegwit)]
        address_type: AddressTypeArg,
    },
    /// Derive destinations from a ranged external descriptor
    Descriptor {
        value: String,
        #[arg(long, default_value_t = 0)]
        start_index: u32,
    },
    /// Reveal destinations from another saved signer
    Signer {
        signer: SignerId,
        #[arg(long, value_enum, default_value_t = AddressTypeArg::NativeSegwit)]
        address_type: AddressTypeArg,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum NetworkArg {
    Mutinynet,
    Testnet4,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum AddressTypeArg {
    Legacy,
    WrappedSegwit,
    NativeSegwit,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum PsbtFormatArg {
    Base64,
    Binary,
    Bbqr,
    Ur,
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::Cli;

    #[test]
    fn bulk_plan_rejects_invalid_amounts_and_counts_before_wallet_access() {
        for (amount, count) in [
            ("0", "1"),
            ("1000", "0"),
            ("1000", "10001"),
            ("1000", "18446744073709551615"),
        ] {
            let result = Cli::try_parse_from([
                "signer-lab-cli",
                "bulk",
                "plan",
                "--signer",
                "00000000-0000-0000-0000-000000000000",
                "fan-out",
                "--amount",
                amount,
                "--count",
                count,
                "address",
                "destination",
            ]);

            assert!(result.is_err());
        }
    }

    #[test]
    fn bulk_plan_requires_a_typed_destination() {
        let result = Cli::try_parse_from([
            "signer-lab-cli",
            "bulk",
            "plan",
            "--signer",
            "00000000-0000-0000-0000-000000000000",
            "fan-out",
            "--amount",
            "1000",
            "--count",
            "10",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn external_destinations_accept_explicit_indexes_and_default_to_zero() {
        for destination in ["xpub", "descriptor"] {
            for (option, expected) in [(None, 0), (Some("17"), 17)] {
                let mut args = vec![
                    "signer-lab-cli",
                    "bulk",
                    "plan",
                    "--signer",
                    "00000000-0000-0000-0000-000000000000",
                    "consolidate",
                    destination,
                    "public destination",
                ];

                if let Some(index) = option {
                    args.extend(["--start-index", index]);
                }

                let parsed = Cli::try_parse_from(args).expect("destination parses");
                let super::Command::Bulk {
                    command:
                        super::BulkCommand::Plan {
                            kind: super::BulkPlanCommand::Consolidate(args),
                            ..
                        },
                } = parsed.command
                else {
                    panic!("expected consolidation plan")
                };

                let (super::DestinationCommand::Xpub {
                    start_index: index, ..
                }
                | super::DestinationCommand::Descriptor {
                    start_index: index, ..
                }) = args.destination
                else {
                    panic!("expected external destination")
                };

                assert_eq!(index, expected);
            }
        }
    }

    #[test]
    fn dismiss_requires_a_signer_and_operation() {
        let args = [
            "signer-lab-cli",
            "bulk",
            "dismiss",
            "--signer",
            "00000000-0000-0000-0000-000000000000",
            "--operation",
            "00000000-0000-0000-0000-000000000001",
        ];

        assert!(matches!(
            Cli::try_parse_from(args).expect("dismiss parses").command,
            super::Command::Bulk {
                command: super::BulkCommand::Dismiss { .. }
            }
        ));

        assert!(Cli::try_parse_from(&args[..5]).is_err());
        assert!(
            Cli::try_parse_from(["signer-lab-cli", "bulk", "dismiss", "--operation", args[6]])
                .is_err()
        );
    }
}
