use std::path::PathBuf;

use jiff::Timestamp;
use serde::Serialize;
use serde_json::Value;
use signer_lab_core::{
    BulkPlan, ExportKind, KeyTeleportQr, LabNetwork, OperationId, OperationLifecycle, PsbtReview,
    SignReport, SignerId, StoredOperation, VirtualSigner,
};

#[derive(Serialize)]
pub(super) struct GeneratedSignerOutput {
    pub(super) signer: VirtualSigner,
    pub(super) mnemonic: String,
}

#[derive(Serialize)]
pub(super) struct RevealedSecretOutput {
    pub(super) signer_id: SignerId,
    pub(super) mnemonic: String,
}

#[derive(Serialize)]
pub(super) struct SeedQrOutput {
    pub(super) signer_id: SignerId,
    pub(super) seed_qr: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DeletedSignerOutput {
    pub(super) deleted_signer_id: SignerId,
}

#[derive(Debug, Serialize)]
pub(super) struct AddressOutput {
    pub(super) address: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WalletExportOutput {
    pub(super) kind: ExportKind,
    pub(super) media_type: &'static str,
    pub(super) output: PathBuf,
}

#[derive(Debug, Serialize)]
pub(super) struct PsbtSignOutput {
    pub(super) output: PathBuf,
    pub(super) format: &'static str,
    pub(super) report: SignReport,
    pub(super) review: PsbtReview,
    /// Raw transaction hex, or `null` while any input still needs a signature
    pub(super) transaction: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BulkOperationOutput {
    pub(super) signer_id: SignerId,
    pub(super) lifecycle: OperationLifecycle,
    pub(super) plan: Value,
}

impl From<StoredOperation> for BulkOperationOutput {
    fn from(operation: StoredOperation) -> Self {
        Self {
            signer_id: operation.signer_id,
            lifecycle: operation.lifecycle,
            plan: operation.plan,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct BulkExecutionOutput {
    pub(super) operation_id: OperationId,
    pub(super) txids: Vec<bitcoin::Txid>,
}

#[derive(Debug, Serialize)]
pub(super) struct BulkCancellationOutput {
    pub(super) cancelled_operation_id: OperationId,
}

#[derive(Debug, Serialize)]
pub(super) struct BulkPlanOutput {
    pub(super) destination_range: Option<std::ops::RangeInclusive<u32>>,
    pub(super) operation_id: OperationId,
    pub(super) kind: signer_lab_core::OperationKind,
    pub(super) network: LabNetwork,
    pub(super) plan: BulkPlan,
    pub(super) reservation_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BulkDismissalOutput {
    pub(super) acknowledged_operation_id: OperationId,
}

#[derive(Debug, Serialize)]
pub(super) struct KeyTeleportSendOutput {
    pub(super) signer_id: SignerId,
    pub(super) response: KeyTeleportQr,
    pub(super) password: String,
}

#[derive(Debug, Serialize)]
pub(super) struct KeyTeleportRequestOutput {
    pub(super) request: KeyTeleportQr,
    pub(super) numeric_code: String,
    pub(super) expires_at: Timestamp,
}

#[derive(Debug, Serialize)]
pub(super) struct KeyTeleportCancelOutput {
    pub(super) cancelled: bool,
}
