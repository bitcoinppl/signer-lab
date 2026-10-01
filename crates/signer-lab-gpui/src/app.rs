use std::{path::PathBuf, str::FromStr as _, sync::Arc, time::Duration};

use bitcoin::{Amount, Psbt, ScriptBuf, bip32::Xpub};
use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, Image, ImageFormat, IntoElement,
    ParentElement as _, PathPromptOptions, Render, SharedString, Styled as _, Timer, Window,
};
use gpui_component::{input::InputState, v_flex};
use signer_lab_core::{
    AddressType, BulkPlan, BulkPlanner, BulkRequest, Destination, DestinationScripts,
    ExportArtifact, ExportKind, ExportPayload, ExternalDescriptor, FixedAddress,
    KeychainSecretStore, LabFeeRate, LabNetwork, OperationId, PendingPsbt, PsbtDecodeProgress,
    QrDensity, QrScanner, QrTransport, ReviewedPsbt, SignerId, SignerRepository, StandardSeedQr,
    SyncControl, VirtualSigner, WalletExporter, decode_psbt_text, read_psbt, sync_account,
};
use tokio::{
    runtime::Handle,
    sync::{Mutex, watch},
};

mod components;
pub(crate) mod design;
mod pages;
mod state;

use state::{
    BulkDestinationInput, BulkMode, BulkWorkflow, CameraState, DestinationMode, Page, QrPlayback,
    QrPreviewSize, RenameState, TextAreaHeights, WalletView,
};

use crate::camera;

pub(crate) type AppRepository = Arc<Mutex<SignerRepository<KeychainSecretStore>>>;

pub(crate) struct SignerLabApp {
    repository: AppRepository,
    runtime: Handle,
    signers: Vec<VirtualSigner>,
    selected_signer: Option<SignerId>,
    selected_account_type: AddressType,
    network: LabNetwork,
    page: Page,
    busy: bool,
    delete_armed: bool,
    status: SharedString,
    wallet: Option<WalletView>,
    reviewed_psbt: Option<ReviewedPsbt>,
    qr_scanner: QrScanner,
    qr_transport: QrTransport,
    qr_playback: Option<QrPlayback>,
    qr_generation: u64,
    qr_preview_size: QrPreviewSize,
    export_artifact: Option<ExportArtifact>,
    camera_generation: u64,
    camera: Option<CameraState>,
    sync_control: Option<SyncControl>,
    bulk_mode: BulkMode,
    destination_mode: DestinationMode,
    destination_address_type: AddressType,
    destination_signer: Option<SignerId>,
    bulk: BulkWorkflow,
    signer_name: Entity<InputState>,
    rename_name: Entity<InputState>,
    rename_state: RenameState,
    text_area_heights: TextAreaHeights,
    mnemonic: Entity<InputState>,
    psbt_input: Entity<InputState>,
    output: Entity<InputState>,
    destination: Entity<InputState>,
    count: Entity<InputState>,
    amount: Entity<InputState>,
    fee_rate: Entity<InputState>,
}

impl SignerLabApp {
    pub(crate) fn new(
        repository: AppRepository,
        runtime: Handle,
        signers: Vec<VirtualSigner>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let signer_name = input(window, cx, "Wallet name", false);
        let rename_name = input(window, cx, "Wallet name", false);
        let mnemonic = input(window, cx, "Recovery words or standard SeedQR digits", true);
        let psbt_input = input(window, cx, "Paste a PSBT, BBQR frames, or UR frames", true);
        let output = input(window, cx, "Exports and signed QR frames appear here", true);
        let destination = input(window, cx, "Destination address", true);
        let count = input_with_value(window, cx, "Output count", "10");
        let amount = input_with_value(window, cx, "Satoshis per output", "1000");
        let fee_rate = input_with_value(window, cx, "Fee rate in sat/vB", "0.5");
        let selected_signer = signers.first().map(|signer| signer.id);

        let app = Self {
            repository,
            runtime,
            signers,
            selected_signer,
            selected_account_type: AddressType::NativeSegwit,
            network: LabNetwork::MutinyNet,
            page: Page::Wallet,
            busy: false,
            delete_armed: false,
            status: "Ready".into(),
            wallet: None,
            reviewed_psbt: None,
            qr_scanner: QrScanner::new(),
            qr_transport: QrTransport::Bbqr,
            qr_playback: None,
            qr_generation: 0,
            qr_preview_size: QrPreviewSize::default(),
            export_artifact: None,
            camera_generation: 0,
            camera: None,
            sync_control: None,
            bulk_mode: BulkMode::FanOut,
            destination_mode: DestinationMode::FixedAddress,
            destination_address_type: AddressType::NativeSegwit,
            destination_signer: None,
            bulk: BulkWorkflow::empty(),
            signer_name,
            rename_name,
            rename_state: RenameState::default(),
            text_area_heights: TextAreaHeights::default(),
            mnemonic,
            psbt_input,
            output,
            destination,
            count,
            amount,
            fee_rate,
        };
        cx.on_next_frame(window, SignerLabApp::load_wallet);

        app
    }

    fn selected(&self) -> Option<VirtualSigner> {
        self.selected_signer
            .and_then(|signer_id| self.signers.iter().find(|signer| signer.id == signer_id))
            .cloned()
    }

    fn select_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }

        if self.page == Page::SeedQr || page == Page::SeedQr {
            self.set_output(String::new(), None, None, window, cx);
        }
        self.page = page;
        cx.notify();
    }

    fn set_status(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = value.into();
        cx.notify();
    }

    fn set_output(
        &mut self,
        text: String,
        qr_playback: Option<QrPlayback>,
        export_artifact: Option<ExportArtifact>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.output
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.qr_generation = self.qr_generation.wrapping_add(1);
        self.qr_playback = qr_playback;
        self.export_artifact = export_artifact;
        self.start_qr_animation(cx);
    }

    fn start_qr_animation(&self, cx: &mut Context<Self>) {
        let Some(interval_ms) = self
            .qr_playback
            .as_ref()
            .and_then(QrPlayback::animation_interval_ms)
        else {
            return;
        };
        let generation = self.qr_generation;

        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(interval_ms)).await;
                let should_continue = view
                    .update(cx, |app, cx| {
                        if app.qr_generation != generation {
                            return false;
                        }
                        let Some(playback) = app.qr_playback.as_mut() else {
                            return false;
                        };

                        playback.advance();
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !should_continue {
                    break;
                }
            }
        })
        .detach();
    }

    fn load_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            self.wallet = None;
            return;
        };

        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let signer_name = signer.name.to_string();
        let task = self.runtime.spawn(async move {
            let mut repository = repository.lock().await;
            repository
                .reconcile_bulk_reservations(&signer, network)
                .await
                .map_err(error_message)?;
            let wallet = repository
                .open_wallet(&signer, network)
                .map_err(error_message)?;
            let operation = repository
                .recoverable_bulk_operation(signer.id, network)
                .await
                .map_err(error_message)?;
            let wallet = WalletView::from_wallet(&wallet)?;

            Ok::<_, String>((wallet, operation))
        });
        self.busy = true;
        self.status = format!("Loading {signer_name} on {network}").into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|_, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok((wallet, operation)) => {
                            app.wallet = Some(wallet);
                            if let Some(operation) = operation {
                                app.bulk_mode =
                                    BulkMode::from_operation_kind(operation.plan.kind());
                                app.bulk =
                                    BulkWorkflow::prepared(operation.operation_id, operation.plan);
                            }
                            app.status = format!("Loaded {signer_name} on {network}").into();
                        }
                        Err(error) => {
                            app.wallet = None;
                            app.status = error.into();
                        }
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn select_signer(&mut self, signer_id: SignerId, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.selected_signer == Some(signer_id) {
            return;
        }

        self.selected_signer = Some(signer_id);
        self.delete_armed = false;
        self.rename_state = RenameState::Idle;
        self.wallet = None;
        self.reviewed_psbt = None;
        self.qr_scanner.reset();
        self.bulk = BulkWorkflow::empty();
        self.set_output(String::new(), None, None, window, cx);
        self.load_wallet(window, cx);
    }

    fn select_account(&mut self, address_type: AddressType, cx: &mut Context<Self>) {
        if self.selected_account_type == address_type {
            return;
        }

        self.selected_account_type = address_type;
        cx.notify();
    }

    fn start_new_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        self.page = Page::Wallet;
        self.selected_signer = None;
        self.wallet = None;
        self.reviewed_psbt = None;
        self.delete_armed = false;
        self.rename_state = RenameState::Idle;
        self.set_output(String::new(), None, None, window, cx);
        self.set_status("Create or import a test wallet", cx);
    }

    fn start_rename_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };
        self.rename_name.update(cx, |input, cx| {
            input.set_value(signer.name.to_string(), window, cx);
        });
        self.rename_state = RenameState::Editing(signer.id);
        cx.notify();
    }

    fn cancel_rename_signer(&mut self, cx: &mut Context<Self>) {
        self.rename_state = RenameState::Idle;
        cx.notify();
    }

    fn rename_signer(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let RenameState::Editing(signer_id) = self.rename_state else {
            return;
        };
        let Some(signer) = self
            .signers
            .iter()
            .find(|signer| signer.id == signer_id)
            .cloned()
        else {
            self.rename_state = RenameState::Idle;
            return;
        };
        let name = self.rename_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let task = self.runtime.spawn(async move {
            repository
                .lock()
                .await
                .rename(&signer, name)
                .await
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Renaming wallet".into();

        cx.spawn(async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            view.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(renamed) => {
                        if let Some(signer) = app
                            .signers
                            .iter_mut()
                            .find(|signer| signer.id == renamed.id)
                        {
                            *signer = renamed;
                        }
                        app.rename_state = RenameState::Idle;
                        app.status = "Wallet renamed".into();
                    }
                    Err(error) => app.status = error.into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reveal_secret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };
        let repository = Arc::clone(&self.repository);
        let task = self.runtime.spawn(async move {
            repository
                .lock()
                .await
                .reveal_secret(signer.id)
                .map(|secret| secret.expose_phrase().to_owned())
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Loading recovery words".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(phrase) => {
                            app.set_output(phrase, None, None, window, cx);
                            app.status =
                                "Recovery words are visible. Do not use them outside this test lab"
                                    .into();
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn export_seed_qr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let repository = Arc::clone(&self.repository);
        let task = self.runtime.spawn(async move {
            let secret = repository
                .lock()
                .await
                .reveal_secret(signer.id)
                .map_err(error_message)?;
            StandardSeedQr::from_secret(&secret)
                .and_then(|seed_qr| seed_qr.numeric_text())
                .map(|seed_qr| seed_qr.to_string())
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Creating standard SeedQR".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(seed_qr) => {
                            app.set_output(
                                seed_qr.clone(),
                                Some(QrPlayback::static_frame(seed_qr)),
                                None,
                                window,
                                cx,
                            );
                            app.status =
                                "Standard SeedQR is visible. Keep this test secret private".into();
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn save_seed_qr_image(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(frame) = self
            .qr_playback
            .as_ref()
            .map(QrPlayback::first_frame)
            .map(str::to_owned)
        else {
            self.set_status("Create a SeedQR first", cx);
            return;
        };
        let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory, Some("seedqr.svg"));
        let runtime = self.runtime.clone();
        self.busy = true;
        self.status = "Select a SeedQR image file".into();

        cx.spawn(async move |view, cx| {
            let selected = prompt
                .await
                .map_err(error_message)
                .and_then(|result| result.map_err(error_message));
            let result = match selected {
                Ok(Some(path)) => runtime
                    .spawn_blocking(move || {
                        let svg = crate::qr_image::svg(&frame)?;
                        std::fs::write(&path, svg).map_err(error_message)?;

                        Ok::<_, String>(path)
                    })
                    .await
                    .map_err(error_message)
                    .and_then(identity)
                    .map(Some),
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };

            view.update(cx, |app, cx| {
                app.busy = false;
                app.status = match result {
                    Ok(Some(path)) => format!("Saved SeedQR image to {}", path.display()).into(),
                    Ok(None) => "SeedQR image save cancelled".into(),
                    Err(error) => error.into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn delete_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        if !self.delete_armed {
            self.delete_armed = true;
            self.set_status(
                "Select confirm delete to remove this wallet and its recovery words",
                cx,
            );
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };
        let signer_id = signer.id;
        let repository = Arc::clone(&self.repository);
        let task = self.runtime.spawn(async move {
            repository
                .lock()
                .await
                .delete(signer.id)
                .await
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Deleting wallet and recovery words".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    app.delete_armed = false;
                    match result {
                        Ok(()) => {
                            app.signers.retain(|signer| signer.id != signer_id);
                            app.selected_signer = app.signers.first().map(|signer| signer.id);
                            app.rename_state = RenameState::Idle;
                            if app.destination_signer == Some(signer_id) {
                                app.destination_signer = None;
                            }
                            app.wallet = None;
                            app.reviewed_psbt = None;
                            app.bulk = BulkWorkflow::empty();
                            app.status = "Wallet deleted. BDK wallet files were retained".into();
                            app.load_wallet(window, cx);
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn set_network(&mut self, network: LabNetwork, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.network == network {
            return;
        }

        self.network = network;
        self.wallet = None;
        self.reviewed_psbt = None;
        self.qr_scanner.reset();
        self.bulk = BulkWorkflow::empty();
        self.set_output(String::new(), None, None, window, cx);
        self.load_wallet(window, cx);
    }

    fn sync_wallet(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let control = SyncControl::new();
        let task_control = control.clone();
        let task = self.runtime.spawn(async move {
            let repository = repository.lock().await;
            let mut wallet = repository
                .open_wallet(&signer, network)
                .map_err(error_message)?;

            for address_type in address_types() {
                sync_account(
                    wallet.account_mut(address_type),
                    network,
                    &task_control,
                    |_| {},
                )
                .await
                .map_err(error_message)?;

                if task_control.is_cancelled() {
                    return Err("wallet sync was cancelled".to_owned());
                }
            }

            WalletView::from_wallet(&wallet)
        });
        self.sync_control = Some(control);
        self.busy = true;
        self.status = format!("Syncing all accounts on {network}").into();

        cx.spawn(async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            view.update(cx, |app, cx| {
                app.busy = false;
                app.sync_control = None;
                match result {
                    Ok(wallet) => {
                        app.wallet = Some(wallet);
                        app.status = "Wallet sync is complete".into();
                    }
                    Err(error) => app.status = error.into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn stop_sync(&mut self, cx: &mut Context<Self>) {
        let Some(control) = self.sync_control.take() else {
            return;
        };

        control.cancel();
        self.status = "Stopping wallet sync".into();
        cx.notify();
    }

    fn create_signer(&mut self, word_count: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let name = self.signer_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let task = self.runtime.spawn(async move {
            let mut repository = repository.lock().await;
            let created = repository
                .generate(name, 0, word_count)
                .await
                .map_err(error_message)?;
            let phrase = created.secret.expose_phrase().to_owned();

            Ok::<_, String>((created.signer, phrase))
        });
        self.busy = true;
        self.status = "Creating wallet".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok((signer, phrase)) => {
                            let signer_id = signer.id;
                            app.signers.push(signer);
                            app.selected_signer = Some(signer_id);
                            app.set_output(phrase, None, None, window, cx);
                            app.status = "Wallet created. Store the recovery words now".into();
                            app.load_wallet(window, cx);
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn import_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let name = self.signer_name.read(cx).value().to_string();
        let phrase = self.mnemonic.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let task = self.runtime.spawn(async move {
            repository
                .lock()
                .await
                .import(name, 0, &phrase)
                .await
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Importing wallet".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(signer) => app.accept_imported_signer(signer, window, cx),
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn import_seed_qr_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let name = self.signer_name.read(cx).value().to_string();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open SeedQR image".into()),
        });
        let repository = Arc::clone(&self.repository);
        let runtime = self.runtime.clone();
        self.busy = true;
        self.status = "Select a standard or compact SeedQR image".into();

        cx.spawn_in(window, async move |view, cx| {
            let selected = prompt
                .await
                .map_err(error_message)
                .and_then(|result| result.map_err(error_message));
            let result = match selected {
                Ok(Some(paths)) => match paths.into_iter().next() {
                    Some(path) => {
                        let secret = runtime
                            .spawn_blocking(move || camera::read_seed_qr_image(&path))
                            .await
                            .map_err(error_message)
                            .and_then(identity);
                        match secret {
                            Ok(secret) => runtime
                                .spawn(async move {
                                    repository
                                        .lock()
                                        .await
                                        .import_secret(name, 0, secret)
                                        .await
                                        .map_err(error_message)
                                })
                                .await
                                .map_err(error_message)
                                .and_then(identity)
                                .map(Some),
                            Err(error) => Err(error),
                        }
                    }
                    None => Err("file picker returned no SeedQR image path".to_owned()),
                },
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };

            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(Some(signer)) => app.accept_imported_signer(signer, window, cx),
                        Ok(None) => app.status = "SeedQR image selection cancelled".into(),
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn start_seed_qr_camera_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let name = self.signer_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let camera = self.new_camera_state();
        let camera_generation = camera.generation();
        let scan_control = camera.control();
        let (preview_sender, preview_receiver) = watch::channel(None);
        let task = self.runtime.spawn(async move {
            let secret = tokio::task::spawn_blocking(move || {
                camera::scan_seed_qr(&scan_control, |preview| {
                    preview_sender.send_replace(Some(preview));
                })
            })
            .await
            .map_err(error_message)
            .and_then(identity)?;
            repository
                .lock()
                .await
                .import_secret(name, 0, secret)
                .await
                .map_err(error_message)
        });
        self.camera = Some(camera);
        self.busy = true;
        self.status = "Camera is scanning for standard or compact SeedQR".into();
        Self::watch_camera_preview(preview_receiver, camera_generation, cx);

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    app.camera = None;
                    match result {
                        Ok(signer) => app.accept_imported_signer(signer, window, cx),
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn accept_imported_signer(
        &mut self,
        signer: VirtualSigner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let signer_id = signer.id;
        self.signers.push(signer);
        self.selected_signer = Some(signer_id);
        self.mnemonic.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.status = "Wallet imported".into();
        self.load_wallet(window, cx);
    }

    fn export(&mut self, kind: ExportKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(wallet) = &self.wallet else {
            self.set_status("Select and load a wallet first", cx);
            return;
        };

        let transport = self.qr_transport;
        let result = WalletExporter::new(self.network, &wallet.accounts)
            .and_then(|exporter| exporter.export(kind))
            .map_err(error_message)
            .and_then(|artifact| {
                let qr = artifact
                    .qr_frames(transport, QrDensity::default())
                    .map_err(error_message)?;
                let text = export_document(&artifact);

                Ok((artifact, text, qr))
            });

        match result {
            Ok((artifact, text, qr)) => {
                self.set_output(text, QrPlayback::animated(qr), Some(artifact), window, cx);
                self.set_status("Export is ready", cx);
            }
            Err(error) => self.set_status(error, cx),
        }
    }

    fn set_qr_transport(
        &mut self,
        transport: QrTransport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.qr_transport == transport {
            return;
        }
        self.qr_transport = transport;

        let Some(artifact) = self.export_artifact.clone() else {
            cx.notify();
            return;
        };
        let qr = match artifact.qr_frames(transport, QrDensity::default()) {
            Ok(qr) => qr,
            Err(error) => {
                self.set_status(error.to_string(), cx);
                return;
            }
        };
        let text = export_document(&artifact);
        self.set_output(text, QrPlayback::animated(qr), Some(artifact), window, cx);
        self.set_status(
            format!(
                "Export QR changed to {}",
                match transport {
                    QrTransport::Bbqr => "BBQR",
                    QrTransport::Ur => "UR",
                }
            ),
            cx,
        );
    }

    fn save_export(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(artifact) = self.export_artifact.clone() else {
            self.set_status("Create an export first", cx);
            return;
        };
        let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory, Some(&artifact.file_name));
        let runtime = self.runtime.clone();
        self.busy = true;
        self.status = "Select an export file".into();

        cx.spawn(async move |view, cx| {
            let selected = prompt
                .await
                .map_err(error_message)
                .and_then(|result| result.map_err(error_message));
            let result = match selected {
                Ok(Some(path)) => {
                    let saved_path = path.clone();
                    runtime
                        .spawn_blocking(move || artifact.write_to_path(path))
                        .await
                        .map_err(error_message)
                        .and_then(|result| result.map_err(error_message))
                        .map(|()| Some(saved_path))
                }
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };

            view.update(cx, |app, cx| {
                app.busy = false;
                app.status = match result {
                    Ok(Some(path)) => format!("Saved export to {}", path.display()).into(),
                    Ok(None) => "Export save cancelled".into(),
                    Err(error) => error.into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn import_psbt_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open PSBT".into()),
        });
        let runtime = self.runtime.clone();
        self.busy = true;
        self.status = "Select a PSBT file".into();

        cx.spawn_in(window, async move |view, cx| {
            let selected = prompt
                .await
                .map_err(error_message)
                .and_then(|result| result.map_err(error_message));
            let result = match selected {
                Ok(Some(paths)) => match paths.into_iter().next() {
                    Some(path) => runtime
                        .spawn_blocking(move || read_psbt(path))
                        .await
                        .map_err(error_message)
                        .and_then(|result| result.map_err(error_message))
                        .map(Some),
                    None => Err("file picker returned no PSBT path".to_owned()),
                },
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };

            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(Some(psbt)) => {
                            app.psbt_input.update(cx, |input, cx| {
                                input.set_value(psbt.to_string(), window, cx);
                            });
                            app.review_psbt(psbt, signer, cx);
                        }
                        Ok(None) => app.status = "PSBT file selection cancelled".into(),
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn decode_and_review_psbt(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let value = self.psbt_input.read(cx).value().to_string();
        let psbt = match decode_psbt_text(&value, &mut self.qr_scanner).map_err(error_message) {
            Ok(PsbtDecodeProgress::Complete(psbt)) => psbt,
            Ok(PsbtDecodeProgress::InProgress { percent }) => {
                self.set_status(format!("QR scan is {percent}% complete"), cx);
                return;
            }
            Err(error) => {
                self.qr_scanner.reset();
                self.set_status(error, cx);
                return;
            }
        };
        self.review_psbt(psbt, signer, cx);
    }

    fn review_psbt(&mut self, psbt: Psbt, signer: VirtualSigner, cx: &mut Context<Self>) {
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let task = self.runtime.spawn(async move {
            let repository = repository.lock().await;
            let wallet = repository
                .open_wallet(&signer, network)
                .map_err(error_message)?;

            PendingPsbt::new(psbt)
                .review(&wallet)
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Reviewing PSBT".into();

        cx.spawn(async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            view.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(reviewed) => {
                        app.reviewed_psbt = Some(reviewed);
                        app.qr_scanner.reset();
                        app.status = "Review the transaction, then sign it".into();
                    }
                    Err(error) => app.status = error.into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_camera_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let camera = self.new_camera_state();
        let camera_generation = camera.generation();
        let scan_control = camera.control();
        let (preview_sender, preview_receiver) = watch::channel(None);
        let task = self.runtime.spawn(async move {
            tokio::task::spawn_blocking(move || {
                camera::scan_psbt(&scan_control, |preview| {
                    preview_sender.send_replace(Some(preview));
                })
            })
            .await
            .map_err(error_message)
            .and_then(identity)
        });
        self.camera = Some(camera);
        self.busy = true;
        self.status = "Camera is scanning BBQR and UR frames".into();
        Self::watch_camera_preview(preview_receiver, camera_generation, cx);

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    app.camera = None;
                    match result {
                        Ok(psbt) => {
                            app.psbt_input.update(cx, |input, cx| {
                                input.set_value(psbt.to_string(), window, cx);
                            });
                            app.review_psbt(psbt, signer, cx);
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn stop_camera_scan(&mut self, cx: &mut Context<Self>) {
        let Some(camera) = self.camera.take() else {
            return;
        };

        camera.cancel();
        self.status = "Stopping camera scan".into();
        cx.notify();
    }

    fn watch_camera_preview(
        mut receiver: watch::Receiver<Option<camera::CameraPreview>>,
        camera_generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |view, cx| {
            while receiver.changed().await.is_ok() {
                let Some(preview) = receiver.borrow_and_update().clone() else {
                    continue;
                };
                let image = Arc::new(Image::from_bytes(ImageFormat::Bmp, preview.into_bmp()));
                if view
                    .update(cx, |app, cx| {
                        let Some(camera) = app
                            .camera
                            .as_mut()
                            .filter(|camera| camera.generation() == camera_generation)
                        else {
                            return;
                        };

                        camera.set_preview(image);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn new_camera_state(&mut self) -> CameraState {
        self.camera_generation = self.camera_generation.wrapping_add(1);

        CameraState::new(self.camera_generation)
    }

    fn sign_psbt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let Some(reviewed) = self.reviewed_psbt.clone() else {
            self.set_status("Decode and review a PSBT first", cx);
            return;
        };
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let transport = self.qr_transport;
        let task = self.runtime.spawn(async move {
            let repository = repository.lock().await;
            let wallet = repository
                .open_wallet(&signer, network)
                .map_err(error_message)?;
            let signed_psbt = reviewed.sign(&wallet, true).map_err(error_message)?;
            let report = signed_psbt.report();
            let frames = signed_psbt
                .qr_frames(transport, QrDensity::default())
                .map_err(error_message)?;

            Ok::<_, String>((frames, report))
        });
        self.busy = true;
        self.status = "Signing PSBT".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok((frames, report)) => {
                            let text = frames.frames.join("\n");
                            app.set_output(text, QrPlayback::animated(frames), None, window, cx);
                            app.status = format!(
                                "Signed {} of {} inputs; {} QR output",
                                report.signed_inputs,
                                report.total_inputs,
                                if report.finalized { "final" } else { "partial" }
                            )
                            .into();
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn plan_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let destination = match self.bulk_destination(cx) {
            Ok(destination) => destination,
            Err(error) => {
                self.set_status(error, cx);
                return;
            }
        };
        let count = match self.bulk_count(cx) {
            Ok(count) => count,
            Err(error) => {
                self.set_status(error, cx);
                return;
            }
        };
        let amount = match self.bulk_amount(cx) {
            Ok(amount) => amount,
            Err(error) => {
                self.set_status(error, cx);
                return;
            }
        };
        let fee_rate = match LabFeeRate::from_str(self.fee_rate.read(cx).value().as_ref()) {
            Ok(fee_rate) => fee_rate,
            Err(error) => {
                self.set_status(error.to_string(), cx);
                return;
            }
        };
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let mode = self.bulk_mode;
        let address_type = self.destination_address_type;
        let task = self.runtime.spawn(plan_bulk_task(
            repository,
            BulkPlanningInput {
                signer,
                destination,
                count,
                amount,
                fee_rate,
                network,
                mode,
                address_type,
            },
        ));
        self.busy = true;
        self.status = "Planning bulk transactions".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok((plan, operation_id, reservation_error)) => {
                            let transaction_count = plan.transactions().len();
                            let total_fee = plan
                                .transactions()
                                .iter()
                                .map(|transaction| transaction.fee().to_sat())
                                .sum::<u64>();
                            let json = serde_json::to_string_pretty(&plan).unwrap_or_default();
                            app.set_output(json, None, None, window, cx);
                            app.bulk = BulkWorkflow::prepared(operation_id, plan);
                            app.status = reservation_error.map_or_else(
                                || {
                                    format!(
                                        "Planned {transaction_count} transaction(s) with {total_fee} sats in fees"
                                    )
                                },
                                |error| {
                                    format!(
                                        "Plan saved, but input reservation failed: {error}. Retry execution or cancel"
                                    )
                                },
                            ).into();
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn execute_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let Some(operation_id) = self.bulk.operation_id() else {
            self.set_status("Plan a bulk operation first", cx);
            return;
        };
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let task = self.runtime.spawn(async move {
            let mut repository = repository.lock().await;
            match repository
                .execute_bulk_operation(&signer, network, operation_id)
                .await
            {
                Ok(report) => Ok(report),
                Err(error) => {
                    let saved = repository
                        .saved_bulk_plan(signer.id, operation_id)
                        .await
                        .ok()
                        .map(|(_, plan)| plan);

                    Err((error_message(error), saved))
                }
            }
        });
        self.busy = true;
        self.status = "Signing and broadcasting the bulk operation".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = match task.await {
                Ok(result) => result,
                Err(error) => Err((error_message(error), None)),
            };
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(report) => {
                            app.bulk = BulkWorkflow::empty();
                            app.status = format!(
                                "Broadcast {} transaction(s) from operation {}",
                                report.txids.len(),
                                report.operation_id
                            )
                            .into();
                            app.load_wallet(window, cx);
                        }
                        Err((error, saved)) => {
                            if let Some(plan) = saved {
                                let json = serde_json::to_string_pretty(&plan).unwrap_or_default();
                                app.set_output(json, None, None, window, cx);
                                app.bulk = BulkWorkflow::failed(operation_id, Some(plan));
                            }
                            if !app.bulk.is_failed() {
                                app.bulk =
                                    BulkWorkflow::failed(operation_id, app.bulk.plan().cloned());
                            }
                            app.status =
                                format!("{error}. The signed operation remains saved").into();
                        }
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn cancel_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }

        let Some(signer) = self.selected() else {
            self.set_status("Select a wallet first", cx);
            return;
        };
        let Some(operation_id) = self.bulk.operation_id() else {
            return;
        };
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let task = self.runtime.spawn(async move {
            repository
                .lock()
                .await
                .cancel_bulk_operation(&signer, network, operation_id)
                .await
                .map_err(error_message)
        });
        self.busy = true;
        self.status = "Cancelling the bulk operation".into();

        cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(identity);
            cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.busy = false;
                    match result {
                        Ok(()) => {
                            app.bulk = BulkWorkflow::empty();
                            app.status = "Bulk operation cancelled and inputs released".into();
                            app.load_wallet(window, cx);
                        }
                        Err(error) => app.status = error.into(),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn bulk_destination(&self, cx: &App) -> Result<BulkDestinationInput, String> {
        let value = self.destination.read(cx).value();
        let destination = match self.destination_mode {
            DestinationMode::FixedAddress => Destination::Fixed(
                FixedAddress::parse(value.as_ref(), self.network).map_err(error_message)?,
            ),
            DestinationMode::AccountXpub => Destination::AccountXpub {
                xpub: Xpub::from_str(value.as_ref()).map_err(error_message)?,
                address_type: self.destination_address_type,
            },
            DestinationMode::Descriptor => Destination::Descriptor(
                ExternalDescriptor::parse(value.as_ref(), self.network).map_err(error_message)?,
            ),
            DestinationMode::SavedSigner => {
                let signer_id = self
                    .destination_signer
                    .ok_or_else(|| "Select a destination wallet".to_owned())?;
                let signer = self
                    .signers
                    .iter()
                    .find(|signer| signer.id == signer_id)
                    .cloned()
                    .ok_or_else(|| "Destination wallet is no longer available".to_owned())?;

                return Ok(BulkDestinationInput::SavedSigner(signer));
            }
        };

        Ok(BulkDestinationInput::Public(destination))
    }

    fn bulk_count(&self, cx: &App) -> Result<usize, String> {
        if self.bulk_mode == BulkMode::Consolidate {
            return Ok(1);
        }

        self.count
            .read(cx)
            .value()
            .parse::<usize>()
            .map_err(error_message)
    }

    fn bulk_amount(&self, cx: &App) -> Result<Amount, String> {
        if self.bulk_mode == BulkMode::Consolidate {
            return Ok(Amount::ZERO);
        }

        self.amount
            .read(cx)
            .value()
            .parse::<u64>()
            .map(Amount::from_sat)
            .map_err(error_message)
    }

    fn copy_output(&mut self, cx: &mut Context<Self>) {
        let text = self.output.read(cx).value().to_string();
        if text.is_empty() {
            self.set_status("There is no output to copy", cx);
            return;
        }

        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.set_status("Copied output", cx);
    }

    fn next_qr_frame(&mut self, cx: &mut Context<Self>) {
        if let Some(playback) = self.qr_playback.as_mut() {
            playback.advance();
            cx.notify();
        }
    }

    fn toggle_qr_animation(&mut self, cx: &mut Context<Self>) {
        let Some(playback) = self.qr_playback.as_mut() else {
            return;
        };
        if playback.toggle_animation().is_none() {
            return;
        }

        self.qr_generation = self.qr_generation.wrapping_add(1);
        self.start_qr_animation(cx);
        cx.notify();
    }

    fn shrink_qr_preview(&mut self, cx: &mut Context<Self>) {
        self.qr_preview_size = self.qr_preview_size.smaller();
        cx.notify();
    }

    fn grow_qr_preview(&mut self, cx: &mut Context<Self>) {
        self.qr_preview_size = self.qr_preview_size.larger();
        cx.notify();
    }
}

impl Render for SignerLabApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.page {
            Page::Wallet => self.wallet_page(cx).into_any_element(),
            Page::Export => self.export_page(cx).into_any_element(),
            Page::Sign => self.sign_page(cx).into_any_element(),
            Page::Bulk => self.bulk_page(cx).into_any_element(),
            Page::SeedQr => self.seed_qr_page(cx).into_any_element(),
        };

        let workspace =
            components::workspace_surface(self.workspace_header(cx), content, self.status.clone());

        v_flex()
            .size_full()
            .bg(design::window_field())
            .text_color(design::text())
            .child(components::app_shell(self.sidebar(cx), workspace))
    }
}

struct BulkPlanningInput {
    signer: VirtualSigner,
    destination: BulkDestinationInput,
    count: usize,
    amount: Amount,
    fee_rate: LabFeeRate,
    network: LabNetwork,
    mode: BulkMode,
    address_type: AddressType,
}

async fn plan_bulk_task(
    repository: AppRepository,
    input: BulkPlanningInput,
) -> Result<(BulkPlan, OperationId, Option<String>), String> {
    let BulkPlanningInput {
        signer,
        destination,
        count,
        amount,
        fee_rate,
        network,
        mode,
        address_type,
    } = input;
    let mut repository = repository.lock().await;
    let scripts = match destination {
        BulkDestinationInput::Public(destination) => destination
            .derive_scripts(network, 0, count)
            .map_err(error_message)?,
        BulkDestinationInput::SavedSigner(destination_signer) => {
            let mut destination_wallet = repository
                .open_wallet(&destination_signer, network)
                .map_err(error_message)?;
            let destination_account = destination_wallet.account_mut(address_type);
            let mut scripts = Vec::with_capacity(count);

            for _ in 0..count {
                let address = destination_account
                    .reveal_receive_address()
                    .map_err(error_message)?;
                let address = FixedAddress::parse(&address, network).map_err(error_message)?;
                scripts.extend(
                    Destination::Fixed(address)
                        .derive_scripts(network, 0, 1)
                        .map_err(error_message)?,
                );
            }

            scripts
        }
    };
    let request = bulk_request(mode, amount, scripts)?;
    let mut wallet = repository
        .open_wallet(&signer, network)
        .map_err(error_message)?;
    let mut planner = BulkPlanner::new(wallet.account_mut(AddressType::NativeSegwit));
    let plan = planner.plan(&request, fee_rate).map_err(error_message)?;
    let operation = repository
        .prepare_bulk_operation(signer.id, network, &plan)
        .await
        .map_err(error_message)?;
    let reservation_error = plan
        .reserve_inputs(wallet.account_mut(AddressType::NativeSegwit))
        .err()
        .map(error_message);

    Ok((plan, operation.lifecycle.id, reservation_error))
}

fn bulk_request(
    mode: BulkMode,
    amount: Amount,
    scripts: Vec<ScriptBuf>,
) -> Result<BulkRequest, String> {
    if mode == BulkMode::Consolidate {
        return scripts
            .into_iter()
            .next()
            .map(BulkRequest::consolidate_all)
            .ok_or_else(|| "a consolidation destination is required".to_owned());
    }

    let destinations = DestinationScripts::new(scripts).map_err(error_message)?;
    match mode {
        BulkMode::FanOut => BulkRequest::fan_out(amount, destinations).map_err(error_message),
        BulkMode::Batch => BulkRequest::batch(amount, destinations).map_err(error_message),
        BulkMode::Consolidate => unreachable!("consolidation returns above"),
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<SignerLabApp>,
    placeholder: &'static str,
    multi_line: bool,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .multi_line(multi_line)
    })
}

fn input_with_value(
    window: &mut Window,
    cx: &mut Context<SignerLabApp>,
    placeholder: &'static str,
    value: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

fn export_document(artifact: &ExportArtifact) -> String {
    match &artifact.payload {
        ExportPayload::Text(text) => text.clone(),
        ExportPayload::Ur { frames, .. } => frames.join("\n"),
    }
}

const fn address_types() -> [AddressType; 3] {
    [
        AddressType::Legacy,
        AddressType::WrappedSegwit,
        AddressType::NativeSegwit,
    ]
}

fn error_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn identity<T>(value: T) -> T {
    value
}
