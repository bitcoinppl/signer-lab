use std::fs;

use color_eyre::eyre::{Result, WrapErr as _};
use signer_lab_core::{SignerId, StandardSeedQr};
use zeroize::Zeroizing;

use super::{
    output::{DeletedSignerOutput, GeneratedSignerOutput, RevealedSecretOutput, SeedQrOutput},
    support,
};
use crate::cli::SignerCommand;

pub(super) async fn run(
    repository: &mut support::Repository,
    command: SignerCommand,
) -> Result<()> {
    match command {
        SignerCommand::List => support::print_json(&repository.signers().await?),
        SignerCommand::Generate {
            name,
            account_index,
            words,
        } => {
            let created = repository
                .generate(name, account_index, words.count())
                .await?;
            let output = GeneratedSignerOutput {
                signer: created.signer,
                mnemonic: created.secret.expose_phrase().to_owned(),
            };

            support::print_json(&output)
        }
        SignerCommand::Import {
            name,
            account_index,
            mnemonic_file,
        } => {
            let phrase =
                Zeroizing::new(fs::read_to_string(&mnemonic_file).wrap_err_with(|| {
                    format!("cannot read mnemonic file {}", mnemonic_file.display())
                })?);
            let signer = repository.import(name, account_index, &phrase).await?;

            support::print_json(&signer)
        }
        SignerCommand::Reveal(selection) => {
            let signer = support::find_signer(repository, selection.signer).await?;
            let secret = repository.reveal_secret(signer.id)?;
            let output = RevealedSecretOutput {
                signer_id: signer.id,
                mnemonic: secret.expose_phrase().to_owned(),
            };

            support::print_json(&output)
        }
        SignerCommand::SeedQr(selection) => {
            let signer = support::find_signer(repository, selection.signer).await?;
            let secret = repository.reveal_secret(signer.id)?;
            let seed_qr = StandardSeedQr::from_secret(&secret)?.numeric_text()?;

            support::print_json(&SeedQrOutput {
                signer_id: signer.id,
                seed_qr: seed_qr.to_string(),
            })
        }
        SignerCommand::Delete(selection) => delete(repository, selection.signer).await,
    }
}

async fn delete(repository: &mut support::Repository, signer_id: SignerId) -> Result<()> {
    let signer = support::find_signer(repository, signer_id).await?;
    repository.delete(signer.id).await?;

    support::print_json(&DeletedSignerOutput {
        deleted_signer_id: signer.id,
    })
}
