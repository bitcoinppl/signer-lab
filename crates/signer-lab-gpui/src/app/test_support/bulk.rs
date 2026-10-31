use super::{Page, PrepareState, ScreenshotFixtures, ScreenshotState, ShotSetup};
use crate::app::bulk::{BulkWorkflow, DestinationMode, Reservation};
use bitcoin::{Amount, Transaction, TxOut, absolute::LockTime, transaction::Version};
use signer_lab_core::{
    AddressType, BulkDestinationSpec, BulkShape, LabFeeRate, LabNetwork, OperationKind,
    OutputCount, PlanBulk,
};
use std::str::FromStr as _;

/// Keeps screenshot setup with the page that owns it
pub(crate) fn states() -> Vec<ScreenshotState> {
    [
        ("bulk", None),
        ("bulk-xpub", Some(xpub as PrepareState)),
        ("bulk-wallet", Some(wallet as PrepareState)),
        ("bulk-planned", Some(planned as PrepareState)),
        ("bulk-unreserved", Some(unreserved as PrepareState)),
        ("bulk-failed", Some(failed as PrepareState)),
    ]
    .into_iter()
    .map(|(name, prepare)| ScreenshotState {
        name,
        page: Page::Bulk,
        has_wallet: true,
        prepare,
    })
    .collect()
}

fn xpub(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let public = fixtures
        .wallet
        .accounts
        .iter()
        .find(|account| account.address_type == AddressType::NativeSegwit)
        .ok_or_else(|| "native account missing".to_owned())?;
    let xpub = public.account_xpub.to_string();
    Ok(Box::new(move |app, window, cx| {
        app.bulk.form.destination_mode = DestinationMode::AccountXpub;
        app.bulk
            .form
            .destination
            .update(cx, |input, cx| input.set_value(xpub, window, cx));
    }))
}

fn wallet(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let signer = fixtures
        .signers
        .get(1)
        .ok_or_else(|| "destination wallet missing".to_owned())?
        .id;
    Ok(Box::new(move |app, _, _| {
        app.bulk.form.destination_mode = DestinationMode::SavedSigner;
        app.bulk.form.destination_signer = Some(signer);
    }))
}

fn fixture_plan(
    fixtures: &ScreenshotFixtures,
) -> Result<signer_lab_core::PlannedBulkOperation, String> {
    fixtures.runtime.block_on(async {
        let mut repository = fixtures.repository.lock().await;
        let signer = fixtures.signers[0].clone();
        let mut wallet = repository
            .open_wallet(&signer, LabNetwork::MutinyNet)
            .map_err(super::error_message)?;
        let account = wallet.account_mut(AddressType::NativeSegwit);

        let address = bitcoin::Address::from_str(&fixtures.wallet.snapshots[2].receive_address)
            .map_err(super::error_message)?
            .require_network(LabNetwork::MutinyNet.bitcoin_network())
            .map_err(super::error_message)?;
        let funding = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![
                TxOut {
                    value: Amount::from_sat(500_000),
                    script_pubkey: address.script_pubkey(),
                };
                3
            ],
        };

        account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
        account.persist().map_err(super::error_message)?;
        drop(wallet);
        let xpub = fixtures
            .wallet
            .accounts
            .iter()
            .find(|account| account.address_type == AddressType::NativeSegwit)
            .ok_or_else(|| "native account missing".to_owned())?
            .account_xpub;
        let result = repository
            .plan_bulk_operation(PlanBulk {
                signer: signer.clone(),
                network: LabNetwork::MutinyNet,
                shape: BulkShape::batch(
                    OutputCount::try_from(3).map_err(super::error_message)?,
                    Amount::from_sat(1000),
                )
                .map_err(super::error_message)?,
                destination: BulkDestinationSpec::AccountXpub {
                    xpub,
                    address_type: AddressType::NativeSegwit,
                    start_index: 7,
                },
                fee_rate: LabFeeRate::DEFAULT,
            })
            .await
            .map_err(super::error_message)?;
        repository
            .cancel_bulk_operation(&signer, LabNetwork::MutinyNet, result.operation_id)
            .await
            .map_err(super::error_message)?;
        Ok(result)
    })
}

fn planned_form(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let setup = xpub(fixtures)?;
    Ok(Box::new(move |app, window, cx| {
        setup(app, window, cx);
        app.bulk.form.mode = OperationKind::Batch;
        app.bulk
            .form
            .start_index
            .update(cx, |input, cx| input.set_value("7", window, cx));
        app.bulk
            .form
            .count
            .update(cx, |input, cx| input.set_value("3", window, cx));
    }))
}

fn planned(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let operation = fixture_plan(fixtures)?;
    let setup = planned_form(fixtures)?;
    Ok(Box::new(move |app, window, cx| {
        setup(app, window, cx);
        app.bulk.workflow = BulkWorkflow::Planned {
            operation_id: operation.operation_id,
            plan: operation.plan,
            reservation: Reservation::Held,
        };
    }))
}

fn unreserved(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let operation = fixture_plan(fixtures)?;
    let setup = planned_form(fixtures)?;
    Ok(Box::new(move |app, window, cx| {
        setup(app, window, cx);
        app.bulk.workflow = BulkWorkflow::Planned {
            operation_id: operation.operation_id,
            plan: operation.plan,
            reservation: Reservation::Failed("Input reservation failed".into()),
        };
    }))
}

fn failed(fixtures: &ScreenshotFixtures) -> Result<ShotSetup, String> {
    let operation = fixture_plan(fixtures)?;
    let txid = operation.plan.transactions()[0]
        .psbt()
        .unsigned_tx
        .compute_txid();
    let setup = planned_form(fixtures)?;
    Ok(Box::new(move |app, window, cx| {
        setup(app, window, cx);
        app.bulk.workflow = BulkWorkflow::Failed {
            operation_id: operation.operation_id,
            plan: operation.plan,
            accepted_txids: vec![txid],
            error: "Relay rejected the signed transaction".into(),
        };
    }))
}

#[cfg(test)]
mod tests {
    use super::{ScreenshotFixtures, fixture_plan};
    use crate::app::{
        SignerLabApp,
        bulk::{BulkWorkflow, DestinationMode, Reservation},
    };
    use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
    use signer_lab_core::{
        BulkDestinationSpec, CurrentBulkOperation, InputReservation, OperationKind,
    };
    use std::sync::Arc;

    #[gpui_kit::test]
    fn form_parses_external_index_and_ignores_hidden_consolidation_fields(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = SignerLabApp::new_state(
                        Arc::clone(&fixtures.repository),
                        fixtures.runtime.handle().clone(),
                        fixtures.signers.clone(),
                        fixtures.teleport_sessions(),
                        window,
                        cx,
                    );

                    let public = &fixtures.wallet.accounts[2];
                    app.bulk.form.destination_mode = DestinationMode::AccountXpub;
                    app.bulk.form.destination.update(cx, |input, cx| {
                        input.set_value(public.account_xpub.to_string(), window, cx);
                    });
                    app.bulk
                        .form
                        .start_index
                        .update(cx, |input, cx| input.set_value("7", window, cx));
                    let request = app.bulk_request(cx).expect("typed request");

                    assert!(matches!(
                        request.destination,
                        BulkDestinationSpec::AccountXpub { start_index: 7, .. }
                    ));
                    app.bulk
                        .form
                        .start_index
                        .update(cx, |input, cx| input.set_value("-1", window, cx));
                    assert!(app.bulk_request(cx).is_err());
                    app.bulk
                        .form
                        .start_index
                        .update(cx, |input, cx| input.set_value("0", window, cx));
                    app.bulk
                        .form
                        .count
                        .update(cx, |input, cx| input.set_value("0", window, cx));
                    assert!(app.bulk_request(cx).is_err());

                    app.bulk.form.mode = OperationKind::Consolidate;
                    app.bulk
                        .form
                        .amount
                        .update(cx, |input, cx| input.set_value("invalid", window, cx));
                    assert!(app.bulk_request(cx).is_ok());
                    app
                })
            })
            .expect("fixture window");
        });
    }

    #[gpui_kit::test]
    fn reservation_failure_stays_planned_and_stored_failure_keeps_accepted_txids(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let mut operation = fixture_plan(&fixtures).expect("funded plan");
        operation.reservation = InputReservation::Failed("lock failure".to_owned());
        let txid = operation.plan.transactions()[0]
            .psbt()
            .unsigned_tx
            .compute_txid();
        let operation_id = operation.operation_id;

        let plan = operation.plan.clone();
        assert!(matches!(
            BulkWorkflow::from(operation),
            BulkWorkflow::Planned {
                reservation: Reservation::Failed(_),
                ..
            }
        ));

        cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = SignerLabApp::new_state(
                        Arc::clone(&fixtures.repository),
                        fixtures.runtime.handle().clone(),
                        fixtures.signers.clone(),
                        fixtures.teleport_sessions(),
                        window,
                        cx,
                    );

                    app.accept_current_bulk_operation(Some(CurrentBulkOperation::Failed {
                        operation_id,
                        plan,
                        failed_index: 1,
                        accepted_txids: vec![txid],
                        error: "relay failure".to_owned(),
                    }));

                    let workflow = &app.bulk.workflow;
                    assert!(matches!(
                        workflow,
                        BulkWorkflow::Failed { accepted_txids, .. } if accepted_txids == &[txid]
                    ));

                    app.accept_current_bulk_operation(None);
                    assert!(matches!(app.bulk.workflow, BulkWorkflow::Draft));
                    app
                })
            })
            .expect("fixture window");
        });
    }
}
