use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, Theme, WindowExt as _, h_flex,
    input::{InputState, TextareaState},
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div,
};
use signer_lab_core::{
    KeyTeleportSessionStore, LabNetwork, SignerId, SignerRepository, VirtualSigner,
};
use tokio::{runtime::Handle, sync::Mutex};

mod bulk;
mod components;
pub(crate) mod design;
mod export;
mod key_teleport;
mod seed_qr;
mod shell;
mod sign;
mod state;
mod wallet;

#[cfg(feature = "test-support")]
pub mod test_support;

use bulk::BulkPage;
use export::ExportPage;
use key_teleport::KeyTeleportPage;
use seed_qr::{CameraState, SeedQrPage};
use sign::SignPage;
use state::{Activity, Outcome, Page, QrPreviewSize, ResultScope, WalletScope, WorkflowOutput};
use wallet::{WalletPage, WalletPane};

pub(crate) type AppRepository = Arc<Mutex<SignerRepository<crate::secrets::AppSecretStore>>>;

pub(crate) struct SignerLabApp {
    repository: AppRepository,
    runtime: Handle,
    signers: Vec<VirtualSigner>,
    selected_signer: Option<SignerId>,
    network: LabNetwork,
    page: Page,
    page_lifetime: Arc<()>,
    activity: Activity,
    outcome: Outcome,
    output: Option<WorkflowOutput>,
    display_output: Entity<TextareaState>,
    qr_preview_size: QrPreviewSize,
    camera: Option<CameraState>,
    wallet: WalletPage,
    seed_qr: SeedQrPage,
    export: ExportPage,
    sign: SignPage,
    bulk: BulkPage,
    key_teleport: KeyTeleportPage,
    action: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl SignerLabApp {
    pub(crate) fn new(
        repository: AppRepository,
        runtime: Handle,
        signers: Vec<VirtualSigner>,
        teleport_sessions: KeyTeleportSessionStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut app = Self::new_state(repository, runtime, signers, teleport_sessions, window, cx);
        app.subscriptions
            .push(cx.observe_window_appearance(window, |_, window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                Theme::update(cx, design::apply_brand);
            }));
        cx.on_next_frame(window, Self::load_wallet);
        app
    }

    // fixtures use the same state constructor without starting repository or appearance tasks
    fn new_state(
        repository: AppRepository,
        runtime: Handle,
        signers: Vec<VirtualSigner>,
        teleport_sessions: KeyTeleportSessionStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_signer = signers.first().map(|signer| signer.id);
        let wallet = WalletPage::new(selected_signer.is_none(), window, cx);
        let seed_qr = SeedQrPage::new(window, cx);
        let sign = SignPage::new(window, cx);
        let bulk = BulkPage::new(window, cx);
        let key_teleport = KeyTeleportPage::new(teleport_sessions, window, cx);
        let subscription =
            cx.subscribe_in(&sign.psbt_input, window, |app, _, event, window, cx| {
                if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                    app.sign.reviewed = None;
                    if matches!(app.output, Some(WorkflowOutput::Signed { .. })) {
                        app.set_output(None, window, cx);
                    }

                    cx.notify();
                }
            });
        Self {
            repository,
            runtime,
            signers,
            selected_signer,
            network: LabNetwork::MutinyNet,
            page: Page::Wallet,
            page_lifetime: Arc::new(()),
            activity: Activity::Idle,
            outcome: Outcome::Info("Ready".into()),
            output: None,
            display_output: text_area(window, cx, ""),
            qr_preview_size: QrPreviewSize::default(),
            camera: None,
            wallet,
            seed_qr,
            export: ExportPage::default(),
            sign,
            bulk,
            key_teleport,
            action: None,
            subscriptions: vec![subscription],
        }
    }

    fn selected(&self) -> Option<VirtualSigner> {
        self.selected_signer
            .and_then(|id| self.signers.iter().find(|signer| signer.id == id))
            .cloned()
    }

    fn scope(&self) -> WalletScope {
        WalletScope {
            signer: self.selected_signer,
            network: self.network,
        }
    }

    fn busy(&self) -> bool {
        !matches!(self.activity, Activity::Idle)
    }

    fn select_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }

        self.clear_secret_drafts(window, cx);
        self.stop_camera_scan(cx);
        window.close_dialog(cx);
        if matches!(self.output, Some(WorkflowOutput::SeedQr { .. })) {
            self.set_output(None, window, cx);
        }

        self.key_teleport.clear_sent();
        self.page = page;
        self.page_lifetime = Arc::new(());
        if page == Page::KeyTeleport {
            self.resume_teleport_receive(window, cx);
        }

        cx.notify();
    }

    /// All wallet selection paths reset signer-scoped workflows through this transition
    fn set_wallet_scope(
        &mut self,
        signer: Option<SignerId>,
        network: LabNetwork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_secret_drafts(window, cx);
        self.stop_camera_scan(cx);
        window.close_dialog(cx);
        self.selected_signer = signer;
        self.network = network;
        self.wallet.reset(signer.is_none());
        self.export = ExportPage::default();
        self.sign.reset(window, cx);
        self.bulk.reset_workflow();
        self.key_teleport.clear_sent();
        self.set_output(None, window, cx);
        cx.notify();
    }

    fn clear_secret_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wallet
            .import_mnemonic
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.seed_qr
            .mnemonic
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn cancel_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_secret_drafts(window, cx);
        self.wallet.pane = WalletPane::Selected;
        cx.notify();
    }

    fn select_signer(&mut self, signer: SignerId, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.selected_signer == Some(signer) {
            return;
        }

        self.set_wallet_scope(Some(signer), self.network, window, cx);
        self.load_wallet(window, cx);
    }

    fn set_network(&mut self, network: LabNetwork, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.network == network {
            return;
        }

        self.set_wallet_scope(self.selected_signer, network, window, cx);
        self.load_wallet(window, cx);
    }

    fn start_new_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        self.select_page(Page::Wallet, window, cx);
        self.wallet.pane = WalletPane::Create;
        cx.notify();
    }

    fn set_status(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.outcome = Outcome::Info(text.into());
        cx.notify();
    }

    fn set_error(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        window.push_notification(Notification::error(text.clone()), cx);
        self.outcome = Outcome::Error(text);
        cx.notify();
    }

    /// Repository work always runs to completion off the UI thread
    /// Results are dropped when the view closes or their scope changes; `SyncControl` owns sync cancellation
    fn run<T: Send + 'static>(
        &mut self,
        label: impl Into<SharedString>,
        scope: ResultScope,
        window: &mut Window,
        cx: &mut Context<Self>,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.busy() {
            return;
        }

        let wallet_scope = self.scope();
        let page_lifetime = Arc::clone(&self.page_lifetime);
        let task = self.runtime.spawn(work);
        self.activity = Activity::Working(label.into());
        self.action = Some(cx.spawn_in(window, async move |view, cx| {
            let result = task.await.map_err(error_message).and_then(|result| result);
            let _ = cx.update(|window, app_cx| {
                view.update(app_cx, |app, cx| {
                    app.activity = Activity::Idle;
                    let valid = wallet_scope == app.scope()
                        && match scope {
                            ResultScope::Wallet => true,
                            ResultScope::Page(page) => {
                                app.page == page && Arc::ptr_eq(&page_lifetime, &app.page_lifetime)
                            }
                        };
                    if valid {
                        match result {
                            Ok(value) => apply(app, value, window, cx),
                            Err(error) => {
                                app.camera = None;
                                app.set_error(error, window, cx);
                            }
                        }
                    }

                    cx.notify();
                })
            });
        }));

        cx.notify();
    }

    fn set_output(
        &mut self,
        output: Option<WorkflowOutput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = output
            .as_ref()
            .and_then(WorkflowOutput::text)
            .unwrap_or_default();
        self.display_output
            .update(cx, |input, cx| input.set_value(text, window, cx));
        let digits = match &output {
            Some(WorkflowOutput::SeedQr { playback }) => playback.current_frame(),
            _ => "",
        };
        self.seed_qr.digits.update(cx, |input, cx| {
            input.set_value(digits.to_owned(), window, cx);
        });
        self.output = output;

        self.start_qr_animation(cx);
        cx.notify();
    }

    fn visible_output(&self) -> Option<&WorkflowOutput> {
        self.output
            .as_ref()
            .filter(|output| output.page() == self.page)
    }

    fn start_qr_animation(&mut self, cx: &mut Context<Self>) {
        let Some(playback) = self.output.as_mut().and_then(WorkflowOutput::playback_mut) else {
            return;
        };

        playback.timer = None;
        let Some(interval) = playback.animation_interval_ms() else {
            return;
        };

        playback.timer = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(interval))
                    .await;
                let keep = view
                    .update(cx, |app, cx| {
                        let Some(output) = app
                            .output
                            .as_mut()
                            .filter(|output| output.page() == app.page)
                        else {
                            return true;
                        };

                        let Some(playback) = output.playback_mut() else {
                            return false;
                        };

                        playback.advance();
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    fn next_qr_frame(&mut self, cx: &mut Context<Self>) {
        if let Some(playback) = self.output.as_mut().and_then(WorkflowOutput::playback_mut) {
            playback.advance();
            cx.notify();
        }
    }

    fn toggle_qr_animation(&mut self, cx: &mut Context<Self>) {
        if let Some(playback) = self.output.as_mut().and_then(WorkflowOutput::playback_mut) {
            playback.toggle_animation();
        }

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

    fn copy_output(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.visible_output().and_then(WorkflowOutput::text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.set_status("Copied output", cx);
        }
    }

    fn no_wallet(cx: &Context<Self>) -> impl IntoElement {
        components::empty_state(
            gpui_kit::assets::IconName::Wallet,
            "No wallet selected",
            Some(
                gpui_kit::component::button::Button::new("create-wallet")
                    .label("Create wallet")
                    .on_click(cx.listener(|app, _, window, cx| app.start_new_signer(window, cx))),
            ),
        )
    }
}

impl Render for SignerLabApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.page {
            Page::Wallet => self.wallet_page(cx).into_any_element(),
            Page::SeedQr => self.seed_qr_page(cx).into_any_element(),
            Page::Export => self.export_page(cx).into_any_element(),
            Page::Sign => self.sign_page(cx).into_any_element(),
            Page::Bulk => self.bulk_page(cx).into_any_element(),
            Page::KeyTeleport => self.key_teleport_page(cx).into_any_element(),
        };
        h_flex()
            .size_full()
            .text_size(px(13.0))
            .text_color(cx.theme().foreground)
            .child(self.sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(cx.theme().background)
                    .child(self.toolbar(cx))
                    .child(div().flex_1().min_h_0().overflow_hidden().child(content))
                    .child(self.status_bar(cx)),
            )
    }
}

use gpui_kit::px;

fn input(
    window: &mut Window,
    cx: &mut Context<SignerLabApp>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

fn text_area(
    window: &mut Window,
    cx: &mut Context<SignerLabApp>,
    placeholder: &'static str,
) -> Entity<TextareaState> {
    cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder))
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

fn save_directory() -> Result<PathBuf, String> {
    let dirs =
        directories::UserDirs::new().ok_or_else(|| "Home directory is not available".to_owned())?;
    Ok(dirs
        .download_dir()
        .unwrap_or_else(|| dirs.home_dir())
        .to_path_buf())
}

fn error_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
