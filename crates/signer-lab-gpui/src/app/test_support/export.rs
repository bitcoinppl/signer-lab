use super::{ScreenshotFixtures, ShotSetup, error_message};
use crate::app::state::{QrPlayback, WorkflowOutput};
use signer_lab_core::{
    AddressType, ExportKind, LabNetwork, QrDensity, QrTransport, WalletExporter,
};

use super::{Page, ScreenshotState};

/// Keeps screenshot setup with the page that owns it
pub(crate) fn states() -> Vec<ScreenshotState> {
    vec![
        ScreenshotState {
            name: "export-empty",
            page: Page::Export,
            has_wallet: true,
            prepare: None,
        },
        ScreenshotState {
            name: "export-descriptor",
            page: Page::Export,
            has_wallet: true,
            prepare: Some(descriptor_output),
        },
        ScreenshotState {
            name: "export-coldcard",
            page: Page::Export,
            has_wallet: true,
            prepare: Some(coldcard_output),
        },
        ScreenshotState {
            name: "export-jade-ur",
            page: Page::Export,
            has_wallet: true,
            prepare: Some(jade_output),
        },
    ]
}

fn descriptor_output(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    export_output(
        fixtures,
        ExportKind::MultipathDescriptor(AddressType::NativeSegwit),
        QrTransport::Bbqr,
    )
}

fn coldcard_output(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    export_output(fixtures, ExportKind::ColdcardGenericJson, QrTransport::Bbqr)
}

fn jade_output(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    export_output(fixtures, ExportKind::JadeCryptoAccount, QrTransport::Ur)
}

fn export_output(
    fixtures: &ScreenshotFixtures,
    kind: ExportKind,
    transport: QrTransport,
) -> Result<ShotSetup, String> {
    let artifact = WalletExporter::new(LabNetwork::MutinyNet, &fixtures.wallet.accounts)
        .and_then(|exporter| exporter.export(kind))
        .map_err(error_message)?;
    let frames = artifact
        .qr_frames(transport, QrDensity::default())
        .map_err(error_message)?;

    Ok(Box::new(move |app, window, cx| {
        app.export.transport = transport;
        app.set_output(
            Some(WorkflowOutput::Export {
                artifact,
                playback: QrPlayback::animated(frames),
            }),
            window,
            cx,
        );
    }))
}
