use super::{Page, ScreenshotFixtures, ScreenshotState, ShotSetup, TEST_WORDS, error_message};
use crate::app::key_teleport::{ReceiveRequestData, TeleportMode, TeleportedQr};
use signer_lab_core::{KeyTeleportSessionStore, MnemonicSecret};

/// Keeps screenshot setup with the page that owns it
pub(crate) fn states() -> Vec<ScreenshotState> {
    vec![
        ScreenshotState {
            name: "key-teleport-send",
            page: Page::KeyTeleport,
            has_wallet: true,
            prepare: Some(sent),
        },
        ScreenshotState {
            name: "key-teleport-receive",
            page: Page::KeyTeleport,
            has_wallet: true,
            prepare: Some(receiving),
        },
    ]
}

fn sent(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    // a separate store stands in for the receiving device
    let receiver = KeyTeleportSessionStore::new(&fixtures.directory.join("receiver"));
    let request = receiver
        .start()
        .and_then(|session| session.request())
        .map_err(error_message)?;
    let request_text = request.request.qr().map_err(error_message)?.url;
    let code = request.numeric_code.to_string();
    let secret = MnemonicSecret::parse(TEST_WORDS).map_err(error_message)?;
    let sent = request
        .request
        .send_mnemonic(&secret, &request.numeric_code)
        .map_err(error_message)?;
    let qr = sent.response.qr().map_err(error_message)?;
    let password = sent.password.grouped();

    Ok(Box::new(move |app, window, cx| {
        app.key_teleport.mode = TeleportMode::Send;
        app.key_teleport
            .request_text
            .update(cx, |input, cx| input.set_value(request_text, window, cx));
        app.key_teleport
            .code
            .update(cx, |input, cx| input.set_value(code, window, cx));
        app.key_teleport.sent = Some(TeleportedQr::sent(qr, password));
    }))
}

fn receiving(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let session = fixtures
        .teleport_sessions()
        .start()
        .map_err(error_message)?;
    let request = ReceiveRequestData::new(&session).map_err(error_message)?;

    Ok(Box::new(move |app, _, cx| {
        app.key_teleport.mode = TeleportMode::Receive;
        app.key_teleport.receiving = Some(TeleportedQr::receiving(request));
        cx.notify();
    }))
}
