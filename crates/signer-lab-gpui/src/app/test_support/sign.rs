use super::{Page, ScreenshotFixtures, ScreenshotState, ShotSetup, error_message};
use crate::app::state::{EncodedSigned, SignedFormat, SignedOutput, WorkflowOutput};
use bitcoin::{Amount, Psbt, Transaction, TxOut, absolute::LockTime, transaction::Version};
use signer_lab_core::{AddressType, LabNetwork, NetworkWallet, PendingPsbt};

/// Keeps screenshot setup with the page that owns it
pub(crate) fn states() -> Vec<ScreenshotState> {
    [
        ("sign-empty", None),
        ("sign-review", Some(review as super::PrepareState)),
        ("sign-review-partial", Some(partial as super::PrepareState)),
        ("sign-signed", Some(signed as super::PrepareState)),
    ]
    .into_iter()
    .map(|(name, prepare)| ScreenshotState {
        name,
        page: Page::Sign,
        has_wallet: true,
        prepare,
    })
    .collect()
}

#[derive(Clone, Copy)]
enum ReviewFixture {
    Review,
    Partial,
    Signed,
}

fn review(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    prepare(fixtures, ReviewFixture::Review)
}

fn partial(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    prepare(fixtures, ReviewFixture::Partial)
}

fn signed(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    prepare(fixtures, ReviewFixture::Signed)
}

fn funded_psbt(
    wallet: &mut NetworkWallet,
    destination: bitcoin::ScriptBuf,
) -> Result<Psbt, String> {
    let account = wallet.account_mut(AddressType::NativeSegwit);
    let receive = account.reveal_receive_address().map_err(error_message)?;
    let receive = receive
        .parse::<bitcoin::Address<_>>()
        .map_err(error_message)?
        .require_network(LabNetwork::MutinyNet.bitcoin_network())
        .map_err(error_message)?;
    let funding = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: Vec::new(),
        output: vec![TxOut {
            value: Amount::from_sat(125_000),
            script_pubkey: receive.script_pubkey(),
        }],
    };

    account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
    let mut builder = account.bdk_mut().build_tx();
    builder.add_recipient(destination, Amount::from_sat(40_000));
    builder.fee_absolute(Amount::from_sat(250));
    builder.finish().map_err(error_message)
}

fn prepare(fixtures: &ScreenshotFixtures, state: ReviewFixture) -> Result<ShotSetup, String> {
    let partial = matches!(state, ReviewFixture::Partial);
    let signed = matches!(state, ReviewFixture::Signed);
    let (input, reviewed, output) = fixtures.runtime.block_on(async {
        let mut repository = fixtures.repository.lock().await;
        // the partial fixture needs a different seed because review matches the master fingerprint
        let bob = repository
            .import(
                "Bob test",
                0,
                "legal winner thank year wave sausage worth useful legal winner thank yellow",
            )
            .await
            .map_err(error_message)?;
        let mut bob = repository
            .open_wallet(&bob, LabNetwork::MutinyNet)
            .map_err(error_message)?;
        let destination = bob
            .account_mut(AddressType::NativeSegwit)
            .reveal_receive_address()
            .map_err(error_message)?;
        let destination = destination
            .parse::<bitcoin::Address<_>>()
            .map_err(error_message)?
            .require_network(LabNetwork::MutinyNet.bitcoin_network())
            .map_err(error_message)?;
        let mut wallet = repository
            .open_wallet(&fixtures.signers[0], LabNetwork::MutinyNet)
            .map_err(error_message)?;
        let mut psbt = funded_psbt(&mut wallet, destination.script_pubkey())?;

        if partial {
            let other = funded_psbt(&mut bob, destination.script_pubkey())?;
            psbt.unsigned_tx
                .input
                .push(other.unsigned_tx.input[0].clone());
            psbt.inputs.push(other.inputs[0].clone());

            psbt.unsigned_tx.output[0].value += Amount::from_sat(125_000);
        }

        let input = psbt.to_string();
        let reviewed = PendingPsbt::new(psbt)
            .review(&wallet)
            .map_err(error_message)?;
        let facts = reviewed.review();

        assert_eq!(facts.input_count, if partial { 2 } else { 1 });
        assert_eq!(facts.matching_input_count, 1);
        assert_eq!(facts.outputs.len(), 2);
        assert_eq!(
            facts
                .outputs
                .iter()
                .filter(|output| output.owned_by_signer)
                .count(),
            1
        );

        let output = if signed {
            let signed = reviewed
                .clone()
                .sign(&wallet, true)
                .map_err(error_message)?;
            let encoded = EncodedSigned::new(signed, SignedFormat::default())?;
            Some(WorkflowOutput::Signed(Box::new(
                SignedOutput::new(encoded).ok_or("missing signed QR")?,
            )))
        } else {
            None
        };
        Ok::<_, String>((input, reviewed, output))
    })?;

    Ok(Box::new(move |app, window, cx| {
        app.sign
            .psbt_input
            .update(cx, |state, cx| state.set_value(input, window, cx));
        app.sign.reviewed = Some(reviewed);

        if let Some(output) = output {
            app.set_output(Some(output), window, cx);
        }
    }))
}
