use bdk_esplora::esplora_client;
use bdk_wallet::{KeychainKind, Update};
use cove_bdk_progressive_scan::{ProgressiveScanner, ScanEvent};
use cove_util::ResultExt as _;
use tokio_util::sync::CancellationToken;

use crate::{LabNetwork, WalletAccount, WalletAccountSnapshot};

const STOP_GAP: usize = 20;
const PARALLEL_REQUESTS: usize = 4;

/// User-controlled cancellation for one account scan.
#[derive(Debug, Clone, Default)]
pub struct SyncControl(CancellationToken);

impl SyncControl {
    /// Creates a new scan control.
    #[must_use]
    pub fn new() -> Self {
        Self(CancellationToken::new())
    }

    /// Requests cancellation of the active scan.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    /// Returns true after cancellation is requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

/// Progress for one BDK keychain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncProgress {
    /// External receive or internal change keychain.
    pub keychain: KeychainKind,
    /// Total scripts checked for this keychain.
    pub checked: u32,
    /// Current consecutive unused script count.
    pub gap: u32,
    /// Unused-script count that completes this keychain.
    pub stop_gap: u32,
}

/// A progressive synchronization event for UI updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEvent {
    /// Address scanning advanced.
    Progress(SyncProgress),
    /// A partial wallet update was applied and persisted.
    WalletUpdated(WalletAccountSnapshot),
}

/// Final account synchronization result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Account state after the final response was applied.
    pub snapshot: WalletAccountSnapshot,
}

/// Runs Cove's progressive Esplora scan and persists each wallet update.
///
/// # Errors
///
/// Returns an error when client setup, scanning, update application, or persistence fails.
pub async fn sync_account(
    wallet: &mut WalletAccount,
    network: LabNetwork,
    control: &SyncControl,
    mut on_event: impl FnMut(SyncEvent),
) -> Result<SyncReport, SyncError> {
    let client = esplora_client::Builder::new(network.esplora_url())
        .build_async()
        .map_err_str(SyncError::Esplora)?;
    let request = wallet.bdk().start_full_scan().build();
    let last_revealed_indices = wallet.bdk().spk_index().last_revealed_indices();
    let (events, receiver) = flume::unbounded();
    let scanner = ProgressiveScanner::builder()
        .request(request)
        .last_revealed_indices(last_revealed_indices)
        .stop_gap(STOP_GAP)
        .events(events)
        .cancel_token(control.0.clone())
        .esplora(client)
        .map_err_str(SyncError::Scanner)?
        .parallel_requests(PARALLEL_REQUESTS);
    let scan = scanner.run();
    tokio::pin!(scan);

    let response = loop {
        tokio::select! {
            result = &mut scan => break result.map_err_str(SyncError::Scanner)?,
            event = receiver.recv_async() => {
                let event = event.map_err(|_| SyncError::EventChannelClosed)?;
                apply_event(wallet, event, &mut on_event)?;
            }
        }
    };

    wallet
        .bdk_mut()
        .apply_update(response)
        .map_err_str(SyncError::ApplyUpdate)?;
    wallet.persist().map_err(SyncError::Wallet)?;

    Ok(SyncReport {
        snapshot: wallet.snapshot(),
    })
}

fn apply_event(
    wallet: &mut WalletAccount,
    event: ScanEvent<KeychainKind>,
    on_event: &mut impl FnMut(SyncEvent),
) -> Result<(), SyncError> {
    match event {
        ScanEvent::Progress(progress) => {
            on_event(SyncEvent::Progress(SyncProgress {
                keychain: progress.keychain,
                checked: progress.checked,
                gap: progress.gap,
                stop_gap: progress.stop_gap,
            }));
        }
        ScanEvent::Update(update) => {
            wallet
                .bdk_mut()
                .apply_update(Update {
                    last_active_indices: update.last_active_indices,
                    tx_update: update.tx_update,
                    chain: update.chain_update,
                })
                .map_err_str(SyncError::ApplyUpdate)?;
            wallet.persist().map_err(SyncError::Wallet)?;
            on_event(SyncEvent::WalletUpdated(wallet.snapshot()));
        }
        ScanEvent::Complete(_) => {}
    }

    Ok(())
}

/// Wallet synchronization errors.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The Esplora client could not be created.
    #[error("Esplora client setup failed: {0}")]
    Esplora(String),
    /// Cove's progressive scanner failed.
    #[error("progressive wallet scan failed: {0}")]
    Scanner(String),
    /// The progressive event channel closed before scan completion.
    #[error("progressive scan event channel closed")]
    EventChannelClosed,
    /// BDK rejected a scan update.
    #[error("wallet scan update failed: {0}")]
    ApplyUpdate(String),
    /// BDK wallet persistence failed.
    #[error("wallet persistence failed: {0}")]
    Wallet(#[source] crate::WalletError),
}
