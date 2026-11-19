//! Offline application fixtures for screenshot and state verification

use super::{AppRepository, SignerLabApp, error_message, state::Page, wallet::WalletView};
use bitcoin::Amount;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, HeadlessAppContext, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, assets::AllAssets, px, size,
    test::TestWindowExt as _,
};
use signer_lab_core::{
    FileSecretStore, KeyTeleportSessionStore, LabNetwork, LabStore, SignerRepository, VirtualSigner,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const TEST_WORDS: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

mod bulk;
mod export;
mod key_teleport;
mod seed_qr;
mod sign;
mod wallet;

type ShotSetup = Box<dyn FnOnce(&mut SignerLabApp, &mut Window, &mut Context<SignerLabApp>)>;
type PrepareState = fn(&ScreenshotFixtures) -> Result<ShotSetup, String>;

/// Full-window state with page-owned output preparation
#[derive(Clone, Copy)]
pub struct ScreenshotState {
    name: &'static str,
    page: Page,
    has_wallet: bool,
    prepare: Option<PrepareState>,
}

impl ScreenshotState {
    /// Stable file-name prefix
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }
}

/// Collects the page fixtures in stable screenshot order
#[must_use]
pub fn screenshot_states() -> Vec<ScreenshotState> {
    [
        wallet::states(),
        seed_qr::states(),
        export::states(),
        sign::states(),
        bulk::states(),
        key_teleport::states(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Owns temporary databases and the runtime for all captured app windows
pub struct ScreenshotFixtures {
    repository: AppRepository,
    runtime: Arc<tokio::runtime::Runtime>,
    signers: Vec<VirtualSigner>,
    wallet: WalletView,
    directory: PathBuf,
}

impl ScreenshotFixtures {
    /// Builds public-vector wallets without syncing or contacting a server
    ///
    /// # Errors
    /// Returns an error when temporary storage or wallet import fails
    pub fn new() -> Result<Self, String> {
        let directory = std::env::temp_dir().join(format!(
            "signer-lab-shots-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));

        std::fs::create_dir_all(&directory).map_err(error_message)?;
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(error_message)?,
        );

        let (repository, signers, mut wallet) = runtime.block_on(async {
            let store = LabStore::open_or_create(directory.join("metadata.db"))
                .await
                .map_err(error_message)?;
            let mut repository = SignerRepository::new(
                store,
                FileSecretStore::new(directory.join("secrets")),
                directory.join("wallets"),
            );

            let alice = repository
                .import("Alice test", 0, TEST_WORDS)
                .await
                .map_err(error_message)?;
            let bob = repository
                .import("Bob test", 1, TEST_WORDS)
                .await
                .map_err(error_message)?;
            let wallet = WalletView::from_wallet(
                &repository
                    .open_wallet(&alice, LabNetwork::MutinyNet)
                    .map_err(error_message)?,
            )?;
            Ok::<_, String>((
                Arc::new(tokio::sync::Mutex::new(repository)),
                vec![alice, bob],
                wallet,
            ))
        })?;

        for (index, snapshot) in wallet.snapshots.iter_mut().enumerate() {
            snapshot.balance.confirmed = Amount::from_sat([124_000, 52_300, 1_058_267][index]);
            snapshot.utxo_count = [3, 2, 8][index];
            snapshot.locked_utxo_count = [0, 0, 2][index];
        }
        Ok(Self {
            repository,
            runtime,
            signers,
            wallet,
            directory,
        })
    }

    /// Renders a page fixture with the same offline setup in either appearance mode
    ///
    /// # Errors
    /// Returns an error when window creation, frame capture, or PNG output fails
    pub fn render(
        &self,
        state: ScreenshotState,
        mode: ThemeMode,
        path: &Path,
    ) -> Result<(), String> {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(AllAssets),
            gpui_kit::platform::current_headless_renderer,
        );

        cx.update(|cx| {
            gpui_kit::init(cx);
            Theme::change(mode, None, cx);
        });

        let handle = cx.update(|cx| self.open(state, cx))?;
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .map_err(error_message)?;
        let screenshot = cx
            .capture_screenshot(handle)
            .map_err(|error| format!("headless Metal capture failed: {error}"))?;
        let result = screenshot.save(path).map_err(error_message);
        cx.update_window(handle, |_, window, _| window.remove_window())
            .map_err(error_message)?;
        result
    }

    /// Opens the actual app shell with deterministic page-owned state
    ///
    /// # Errors
    /// Returns an error when the window or descriptor export cannot be built
    pub fn open(&self, state: ScreenshotState, cx: &mut App) -> Result<AnyWindowHandle, String> {
        gpui_kit::component::Theme::update(cx, super::design::apply_brand);
        let repository = Arc::clone(&self.repository);
        let runtime = self.runtime.handle().clone();
        let signers = if state.has_wallet {
            self.signers.clone()
        } else {
            Vec::new()
        };

        let wallet = self.wallet.clone();
        let teleport_sessions = self.teleport_sessions();
        let setup = state.prepare.map(|prepare| prepare(self)).transpose()?;
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: gpui_kit::Point::default(),
                    size: size(px(1280.0), px(820.0)),
                })),
                window_min_size: Some(size(px(960.0), px(640.0))),
                window_background: WindowBackgroundAppearance::Blurred,
                focus: false,
                show: false,
                ..TitleBar::window_options()
            },
            cx,
            move |window, cx| {
                cx.new(|cx| {
                    let mut app = SignerLabApp::new_state(
                        repository,
                        runtime,
                        signers,
                        teleport_sessions,
                        window,
                        cx,
                    );
                    app.page = state.page;
                    if state.has_wallet {
                        app.wallet.view = Some(wallet);
                    }

                    if let Some(setup) = setup {
                        setup(&mut app, window, cx);
                    }
                    app
                })
            },
        )
        .map(|(handle, _)| handle)
        .map_err(error_message)
    }
}

impl ScreenshotFixtures {
    fn teleport_sessions(&self) -> KeyTeleportSessionStore {
        KeyTeleportSessionStore::new(&self.directory)
    }
}

impl Drop for ScreenshotFixtures {
    fn drop(&mut self) {
        // windows must close before fixtures drop so all wallet files are no longer in use
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::{ScreenshotFixtures, SignerLabApp};
    use crate::app::{
        bulk::BulkWorkflow,
        design,
        key_teleport::{TeleportMode, TeleportedQr},
        state::{Activity, Page, QrPlayback, ResultScope, WorkflowOutput},
        wallet::WalletPane,
    };
    use bitcoin::{
        Address, Amount, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
        absolute::LockTime, bip32::ChildNumber, secp256k1::Secp256k1, transaction::Version,
    };
    use gpui_kit::{
        AppContext as _, Context, ExternalPaths, FileDropEvent, TestAppContext, Window,
        test::TestAppContextExt as _,
    };
    use signer_lab_core::{
        AddressType, KeyTeleportQr, LabNetwork, PendingPsbt, QrDensity, QrTransport,
    };
    use std::{str::FromStr as _, sync::Arc, time::Duration};
    use tokio::sync::{oneshot, watch};
    use tokio_util::sync::CancellationToken;

    fn app(
        fixtures: &ScreenshotFixtures,
        window: &mut Window,
        cx: &mut Context<SignerLabApp>,
    ) -> SignerLabApp {
        let mut app = SignerLabApp::new_state(
            Arc::clone(&fixtures.repository),
            fixtures.runtime.handle().clone(),
            fixtures.signers.clone(),
            fixtures.teleport_sessions(),
            window,
            cx,
        );

        app.wallet.view = Some(fixtures.wallet.clone());
        app
    }

    fn app_window(
        fixtures: &ScreenshotFixtures,
        cx: &mut TestAppContext,
    ) -> (gpui_kit::Entity<SignerLabApp>, gpui_kit::VisualTestContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| app(fixtures, window, cx))
            })
            .expect("fixture window")
        });
        (view, gpui_kit::VisualTestContext::from_window(handle, cx))
    }

    fn fill_secret_drafts(
        app: &mut SignerLabApp,
        window: &mut Window,
        cx: &mut Context<SignerLabApp>,
    ) {
        app.wallet
            .import_mnemonic
            .update(cx, |input, cx| input.set_value("draft words", window, cx));
        app.seed_qr
            .mnemonic
            .update(cx, |input, cx| input.set_value("00000001", window, cx));
    }

    fn assert_secret_drafts_empty(app: &SignerLabApp, cx: &Context<SignerLabApp>) {
        assert!(app.wallet.import_mnemonic.read(cx).value().is_empty());
        assert!(app.seed_qr.mnemonic.read(cx).value().is_empty());
    }

    #[gpui_kit::test]
    fn page_exit_and_create_cancel_clear_secret_drafts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                for page in [Page::SeedQr, Page::Wallet, Page::Export] {
                    fill_secret_drafts(app, window, cx);
                    app.select_page(page, window, cx);
                    assert_secret_drafts_empty(app, cx);
                }

                app.start_new_signer(window, cx);
                fill_secret_drafts(app, window, cx);
                app.cancel_create(window, cx);
                assert_secret_drafts_empty(app, cx);
                assert!(matches!(app.wallet.pane, WalletPane::Selected));
            });
        });
    }

    #[gpui_kit::test]
    fn key_teleport_resumes_saved_receive_and_drops_sent_secrets(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        fixtures
            .teleport_sessions()
            .start()
            .expect("saved receive session");
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                // a saved session must appear even while other work keeps the app busy
                app.activity = Activity::Working("Syncing".into());
                app.key_teleport.mode = TeleportMode::Receive;
                app.select_page(Page::KeyTeleport, window, cx);
                assert!(app.key_teleport.receiving.is_some());

                // the CLI shares the session, so returning to the page shows what is on disk
                app.select_page(Page::Wallet, window, cx);
                fixtures.teleport_sessions().cancel().expect("cancel");
                app.select_page(Page::KeyTeleport, window, cx);
                assert!(app.key_teleport.receiving.is_none());

                app.key_teleport.sent = Some(TeleportedQr::sent(
                    KeyTeleportQr {
                        bbqr: "B$2S0100".into(),
                        url: "https://keyteleport.com/#B$2S0100".into(),
                    },
                    "AB CD EF GH".into(),
                ));
                app.select_page(Page::Wallet, window, cx);
                assert!(app.key_teleport.sent.is_none());
            });
        });
    }

    #[gpui_kit::test]
    fn wallet_scope_transitions_clear_secret_drafts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                // select, create/import, delete, and network all use this transition
                for (signer, network) in [
                    (Some(fixtures.signers[1].id), LabNetwork::MutinyNet),
                    (Some(fixtures.signers[0].id), LabNetwork::MutinyNet),
                    (None, LabNetwork::MutinyNet),
                    (None, LabNetwork::Testnet4),
                ] {
                    fill_secret_drafts(app, window, cx);
                    app.set_wallet_scope(signer, network, window, cx);
                    assert_secret_drafts_empty(app, cx);
                }
            });
        });
    }

    #[gpui_kit::test]
    fn seed_digits_have_only_a_page_owned_display(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.select_page(Page::SeedQr, window, cx);
                for clear in 0..3 {
                    app.set_output(
                        Some(WorkflowOutput::SeedQr {
                            playback: QrPlayback::static_frame("00000001".into()),
                        }),
                        window,
                        cx,
                    );
                    assert_eq!(app.seed_qr.digits.read(cx).value().as_str(), "00000001");
                    assert!(app.display_output.read(cx).value().is_empty());
                    assert!(app.visible_output().expect("seed output").text().is_none());
                    app.copy_seed_qr_digits(cx);
                    assert_eq!(
                        cx.read_from_clipboard()
                            .expect("digits clipboard")
                            .text()
                            .expect("text"),
                        "00000001"
                    );
                    match clear {
                        0 => app.set_output(None, window, cx),
                        1 => app.select_page(Page::Export, window, cx),
                        _ => app.set_wallet_scope(
                            app.selected_signer,
                            LabNetwork::Testnet4,
                            window,
                            cx,
                        ),
                    }
                    assert!(app.seed_qr.digits.read(cx).value().is_empty());
                    app.select_page(Page::SeedQr, window, cx);
                }
            });
        });
    }

    #[gpui_kit::test]
    async fn wallet_result_applies_after_page_exit(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        let (sender, receiver) = oneshot::channel();
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.run(
                    "Import",
                    ResultScope::Wallet,
                    window,
                    cx,
                    async move { receiver.await.map_err(super::error_message) },
                    SignerLabApp::accept_imported_signer,
                );
                app.select_page(Page::Export, window, cx);
            });
        });
        let signer = fixtures.signers[1].clone();
        sender.send(signer).expect("pending import");
        let handle = cx.update(|window, _| window.window_handle());
        cx.wait_for(handle, Duration::from_secs(3), |_, cx| {
            view.read(cx).signers.len() == 3
        })
        .await;
        cx.update(|_, cx| assert_eq!(view.read(cx).page, Page::Export));
    }

    #[gpui_kit::test]
    fn dropped_ui_action_does_not_abort_repository_work(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        let (sender, receiver) = oneshot::channel();
        let (done, completed) = oneshot::channel();
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.run(
                    "Durable work",
                    ResultScope::Wallet,
                    window,
                    cx,
                    async move {
                        receiver.await.map_err(super::error_message)?;
                        done.send(()).expect("completion receiver");
                        Ok(())
                    },
                    |_, (), _, _| panic!("dropped action must not apply"),
                );
                app.action = None;
            });
        });
        sender.send(()).expect("work remains alive");
        fixtures.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), completed)
                .await
                .expect("work completes")
                .expect("completion");
        });
    }

    #[gpui_kit::test]
    fn scope_change_clears_secret_output_and_cancels_camera(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let bob = fixtures.signers[1].id;
        let (view, mut cx) = app_window(&fixtures, cx);
        let control = CancellationToken::new();
        let (_sender, receiver) = watch::channel(None);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.page = Page::SeedQr;
                app.set_output(
                    Some(WorkflowOutput::SeedQr {
                        playback: QrPlayback::static_frame("00000001".into()),
                    }),
                    window,
                    cx,
                );

                app.install_camera(control.clone(), receiver, cx);
                app.set_wallet_scope(Some(bob), LabNetwork::Testnet4, window, cx);
                assert!(app.output.is_none());
                assert!(app.display_output.read(cx).value().is_empty());
                assert!(app.camera.is_none());
                assert!(matches!(app.bulk.workflow, BulkWorkflow::Draft));
                assert_eq!(app.selected_signer, Some(bob));
            });
        });

        assert!(control.is_cancelled());
    }

    #[gpui_kit::test]
    async fn page_request_cannot_restore_secrets_after_navigation(cx: &mut TestAppContext) {
        // the request completes on the real Tokio executor
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        let (sender, receiver) = oneshot::channel::<String>();
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.page = Page::SeedQr;
                app.run(
                    "Creating SeedQR",
                    ResultScope::Page(Page::SeedQr),
                    window,
                    cx,
                    async move { receiver.await.map_err(super::error_message) },
                    |app, digits, window, cx| {
                        app.set_output(
                            Some(WorkflowOutput::SeedQr {
                                playback: QrPlayback::static_frame(digits),
                            }),
                            window,
                            cx,
                        );
                    },
                );

                app.select_page(Page::Wallet, window, cx);
                app.select_page(Page::SeedQr, window, cx);
            });
        });

        sender.send("00000001".into()).expect("pending work");
        let handle = cx.update(|window, _| window.window_handle());
        cx.wait_for(handle, Duration::from_secs(3), |_, cx| {
            !view.read(cx).busy()
        })
        .await;

        cx.update(|_, cx| {
            assert!(view.read(cx).output.is_none());
        });
    }

    /// Builds a one-input PSBT that the first fixture signer's native segwit account owns
    fn owned_psbt(fixtures: &ScreenshotFixtures) -> Psbt {
        let public = fixtures
            .wallet
            .accounts
            .iter()
            .find(|account| account.address_type == AddressType::NativeSegwit)
            .expect("native account");
        let suffix = [
            ChildNumber::from_normal_idx(0).expect("branch"),
            ChildNumber::from_normal_idx(0).expect("index"),
        ];

        let key = public
            .account_xpub
            .derive_pub(&Secp256k1::verification_only(), &suffix)
            .expect("derived key");
        let address = Address::from_str(&fixtures.wallet.snapshots[2].receive_address)
            .expect("fixture address")
            .require_network(LabNetwork::MutinyNet.bitcoin_network())
            .expect("fixture network");
        let previous_output = TxOut {
            value: Amount::from_sat(2000),
            script_pubkey: address.script_pubkey(),
        };

        let mut psbt = Psbt::from_unsigned_tx(Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::default(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: address.script_pubkey(),
            }],
        })
        .expect("PSBT");

        psbt.inputs[0].witness_utxo = Some(previous_output);
        psbt.inputs[0].bip32_derivation.insert(
            key.public_key,
            (
                public.master_fingerprint,
                public.account_path.extend(suffix),
            ),
        );

        psbt
    }

    #[gpui_kit::test]
    fn psbt_edit_invalidates_review_and_signed_output(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (reviewed, signed, frames) = fixtures.runtime.block_on(async {
            let repository = fixtures.repository.lock().await;
            let wallet = repository
                .open_wallet(&fixtures.signers[0], LabNetwork::MutinyNet)
                .expect("wallet");
            let psbt = owned_psbt(&fixtures);
            let reviewed = PendingPsbt::new(psbt).review(&wallet).expect("review");
            let signed = reviewed.clone().sign(&wallet, true).expect("sign");
            let frames = signed
                .qr_frames(QrTransport::Bbqr, QrDensity::default())
                .expect("frames");
            (reviewed, signed, frames)
        });

        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.page = Page::Sign;
                app.sign.reviewed = Some(reviewed);
                app.set_output(
                    Some(WorkflowOutput::Signed {
                        signed,
                        playback: QrPlayback::animated(frames).expect("playback"),
                    }),
                    window,
                    cx,
                );

                app.sign.psbt_input.update(cx, |input, cx| {
                    input.set_value("changed PSBT", window, cx);
                    // set_value is silent; user edits emit the change event
                    cx.emit(gpui_kit::component::input::InputEvent::Change);
                });
            });
        });

        cx.run_until_parked();
        cx.update(|_, cx| {
            let app = view.read(cx);
            assert!(app.sign.reviewed.is_none());
            assert!(app.output.is_none());
            assert!(app.display_output.read(cx).value().is_empty());
        });
    }

    #[gpui_kit::test]
    async fn dropped_binary_psbt_file_fills_input_and_reviews(cx: &mut TestAppContext) {
        // file reads and review run on the real Tokio executor
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let psbt = owned_psbt(&fixtures);
        let path = fixtures.directory.join("unsigned.psbt");
        std::fs::write(&path, psbt.serialize()).expect("binary PSBT file");

        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|_, cx| view.update(cx, |app, _| app.page = Page::Sign));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let position = cx
            .debug_bounds("psbt-input-panel")
            .expect("PSBT input panel")
            .center();

        // the platform delivers a Finder drag as enter, move, then drop
        cx.simulate_event(FileDropEvent::Entered {
            position,
            paths: ExternalPaths(vec![path.clone()].into()),
        });
        cx.simulate_event(FileDropEvent::Pending { position });
        cx.simulate_event(FileDropEvent::Submit { position });

        let handle = cx.update(|window, _| window.window_handle());
        cx.wait_for(handle, Duration::from_secs(3), |_, cx| {
            view.read(cx).sign.reviewed.is_some()
        })
        .await;

        cx.update(|_, cx| {
            let app = view.read(cx);
            assert_eq!(
                app.sign.psbt_input.read(cx).value().as_ref(),
                psbt.to_string()
            );
            let review = app.sign.reviewed.as_ref().expect("review").review();
            assert_eq!(review.matching_input_count, 1);
        });
    }

    #[gpui_kit::test]
    async fn successful_import_clears_words_and_keeps_create_selection(cx: &mut TestAppContext) {
        // repository I/O completes on the real Tokio executor
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let bob = fixtures.signers[1].clone();
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                let previous = app.selected_signer;
                app.start_new_signer(window, cx);
                assert_eq!(app.selected_signer, previous);
                assert!(matches!(app.wallet.pane, WalletPane::Create));
                app.signers.retain(|signer| signer.id != bob.id);
                app.seed_qr.mnemonic.update(cx, |input, cx| {
                    input.set_value(super::TEST_WORDS, window, cx);
                });

                app.accept_imported_signer(bob.clone(), window, cx);
                assert!(app.seed_qr.mnemonic.read(cx).value().is_empty());
                assert_eq!(app.selected_signer, Some(bob.id));
                assert!(matches!(app.wallet.pane, WalletPane::Selected));
            });
        });

        let handle = cx.update(|window, _| window.window_handle());
        cx.wait_for(handle, Duration::from_secs(3), |_, cx| {
            !view.read(cx).busy()
        })
        .await;
    }

    #[gpui_kit::test]
    fn shell_dimensions_follow_the_window_contract(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (_, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        assert_eq!(
            cx.debug_bounds("app-sidebar").expect("sidebar").size.width,
            gpui_kit::px(design::SIDEBAR_WIDTH)
        );

        assert_eq!(
            cx.debug_bounds("app-toolbar").expect("toolbar").size.height,
            gpui_kit::px(design::TOOLBAR_HEIGHT)
        );
    }
}
