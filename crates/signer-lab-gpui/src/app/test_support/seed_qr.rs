use super::{Page, ScreenshotFixtures, ScreenshotState, ShotSetup, TEST_WORDS, error_message};
use crate::app::state::{QrPlayback, WorkflowOutput};
use signer_lab_core::{MnemonicSecret, StandardSeedQr};

/// Keeps screenshot setup with the page that owns it
pub(crate) fn states() -> Vec<ScreenshotState> {
    vec![
        ScreenshotState {
            name: "seed-qr",
            page: Page::SeedQr,
            has_wallet: true,
            prepare: None,
        },
        ScreenshotState {
            name: "seed-qr-shown",
            page: Page::SeedQr,
            has_wallet: true,
            prepare: Some(shown_output),
        },
    ]
}

fn shown_output(_: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let secret = MnemonicSecret::parse(TEST_WORDS).map_err(error_message)?;
    let digits = StandardSeedQr::from_secret(&secret)
        .and_then(|qr| qr.numeric_text())
        .map_err(error_message)?;

    Ok(Box::new(move |app, window, cx| {
        app.set_output(
            Some(WorkflowOutput::SeedQr {
                playback: QrPlayback::static_frame(digits.to_string()),
            }),
            window,
            cx,
        );
    }))
}
