use std::path::Path;

use color_eyre::eyre::Result;
use signer_lab_core::{KeyTeleportError, KeyTeleportReceiveSession, KeyTeleportSessionStore};

use super::{
    output::{KeyTeleportCancelOutput, KeyTeleportRequestOutput, KeyTeleportSendOutput},
    support,
};
use crate::cli::{KeyTeleportCommand, KeyTeleportReceiveCommand};

pub(super) async fn run(
    repository: &mut support::Repository,
    data_dir: &Path,
    command: KeyTeleportCommand,
) -> Result<()> {
    let sessions = KeyTeleportSessionStore::new(data_dir);
    match command {
        KeyTeleportCommand::Send {
            signer,
            request,
            code,
        } => {
            let signer = support::find_signer(repository, signer).await?;
            let secret = repository.reveal_secret(signer.id)?;
            let sent = request.send_mnemonic(&secret, &code)?;

            support::print_json(&KeyTeleportSendOutput {
                signer_id: signer.id,
                response: sent.response.qr()?,
                password: sent.password.to_string(),
            })
        }
        KeyTeleportCommand::Receive { command } => receive(repository, &sessions, command).await,
    }
}

async fn receive(
    repository: &mut support::Repository,
    sessions: &KeyTeleportSessionStore,
    command: KeyTeleportReceiveCommand,
) -> Result<()> {
    match command {
        KeyTeleportReceiveCommand::Start => print_request(&sessions.start()?),
        KeyTeleportReceiveCommand::Show => {
            let session = sessions
                .active()?
                .ok_or(KeyTeleportError::NoReceiveSession)?;

            print_request(&session)
        }
        KeyTeleportReceiveCommand::Finish {
            name,
            account_index,
            response,
            password,
        } => {
            let signer = sessions
                .import(repository, name, account_index, &response, &password)
                .await?;

            support::print_json(&signer)
        }
        KeyTeleportReceiveCommand::Cancel => {
            sessions.cancel()?;

            support::print_json(&KeyTeleportCancelOutput { cancelled: true })
        }
    }
}

fn print_request(session: &KeyTeleportReceiveSession) -> Result<()> {
    let request = session.request()?;

    support::print_json(&KeyTeleportRequestOutput {
        request: request.request.qr()?,
        numeric_code: request.numeric_code.to_string(),
        expires_at: request.expires_at,
    })
}
