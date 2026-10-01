//! Reusable wallet and signing core for Signer Lab

/// Bulk transaction requests, plans, and input reservations
pub mod bulk;
/// Checked destination types for bulk transaction outputs
pub mod destination;
/// Core signer, account, address, and network domain types
pub mod domain;
/// Coldcard, descriptor, and Jade wallet exports
pub mod export;
/// Exact fee-rate parsing and validation
pub mod fee_rate;
/// PSBT and wallet-export file formats
pub mod file;
/// BIP39 mnemonic generation, parsing, and completion
pub mod mnemonic;
/// Recoverable bulk-operation lifecycle types
pub mod operation;
/// BBQR and UR encoding and multipart scanning
pub mod qr;
/// Signer storage and bulk-operation orchestration
pub mod repository;
/// Secret storage abstractions and macOS Keychain support
pub mod secret_store;
/// Standard and compact `SeedQR` secrets
pub mod seed_qr;
/// Reviewed PSBT state transitions and signing
pub mod signing;
/// Toasty and Turso metadata persistence
pub mod storage;
/// Progressive Esplora synchronization
pub mod sync;
/// Persistent BDK wallet accounts and signing support
pub mod wallet;

pub use bulk::{
    BulkPlan, BulkPlanError, BulkPlanner, BulkRequest, DestinationScripts, PlannedTransaction,
};
pub(crate) use destination::Destination;
pub use destination::{DestinationError, ExternalDescriptor, FixedAddress};
pub use domain::{
    AccountIndex, AccountIndexError, AddressType, LabNetwork, SignerId, SignerName,
    SignerNameError, VirtualSigner,
};
pub use export::{ExportArtifact, ExportError, ExportKind, ExportPayload, WalletExporter};
pub use fee_rate::{FeeRateError, LabFeeRate};
pub use file::{
    ArtifactFileError, PsbtDecodeProgress, PsbtFileError, PsbtFileFormat, decode_psbt,
    decode_psbt_text, read_psbt, write_psbt,
};
pub use mnemonic::{MnemonicError, MnemonicImportError, MnemonicSecret};
pub use operation::{
    OperationId, OperationKind, OperationLifecycle, OperationState, StateTransitionError,
};
pub use qr::{QrDensity, QrFrames, QrScanProgress, QrScanner, QrTransport, QrTransportError};
pub use repository::{
    BulkDestinationSpec, BulkExecutionError, BulkExecutionReport, BulkPlanOperationError,
    BulkShape, CreatedSigner, CurrentBulkOperation, InputReservation, OutputCount, PlanBulk,
    PlannedBulkOperation, RepositoryError, ReservationReconciliation, SignerRepository,
};
pub use secret_store::{KeychainSecretStore, SecretStore, SecretStoreError};
pub use seed_qr::{CompactSeedQr, SeedQr, SeedQrError, StandardSeedQr};
pub use signing::{
    PendingPsbt, PsbtOutputReview, PsbtReview, PsbtReviewError, ReviewedPsbt, SignedPsbt,
};
pub use storage::{LabStore, StorageError, StoredOperation};
pub use sync::{SyncControl, SyncError, SyncEvent, SyncProgress, SyncReport, sync_account};
pub use wallet::{
    AccountPublicData, NetworkWallet, SignReport, WalletAccount, WalletAccountSnapshot, WalletError,
};

/// Current library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
