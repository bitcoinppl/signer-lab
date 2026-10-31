use std::{str::FromStr as _, sync::Arc};

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    form::{field, v_form},
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use jiff::{Timestamp, tz::TimeZone};
use signer_lab_core::{
    KeyTeleportError, KeyTeleportQr, KeyTeleportReceiveSession, KeyTeleportRequest,
    KeyTeleportResponse, KeyTeleportSessionStore, NumericCode, TeleportPassword,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{
    SignerLabApp, components, design, error_message, input, input_with_value,
    state::{Page, QrPlayback, ResultScope},
    text_area,
};
use crate::camera;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TeleportMode {
    Send,
    Receive,
}

/// The text field a camera scan fills in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TeleportScan {
    Request,
    Response,
}

pub(super) struct KeyTeleportPage {
    pub(super) sessions: KeyTeleportSessionStore,
    pub(super) mode: TeleportMode,
    pub(super) request_text: Entity<TextareaState>,
    pub(super) code: Entity<InputState>,
    pub(super) response_text: Entity<TextareaState>,
    pub(super) password: Entity<InputState>,
    pub(super) signer_name: Entity<InputState>,
    pub(super) sent: Option<TeleportedQr>,
    pub(super) receiving: Option<TeleportedQr>,
    pub(super) scan: Option<TeleportScan>,
}

/// The short value typed on the other device, which differs by transfer direction
enum TeleportSecret {
    /// Decrypts a sent response, so together with the QR it reveals the wallet
    Password(SharedString),
    /// Unlocks a receiver request for the sender
    ReceiverCode {
        code: SharedString,
        expires_at: Timestamp,
    },
}

/// A rendered packet with the short secret typed on the other device
pub(super) struct TeleportedQr {
    playback: QrPlayback,
    url: String,
    secret: TeleportSecret,
}

impl TeleportedQr {
    pub(super) fn sent(qr: KeyTeleportQr, password: String) -> Self {
        Self::new(qr, TeleportSecret::Password(password.into()))
    }

    pub(super) fn receiving(request: ReceiveRequestData) -> Self {
        Self::new(
            request.qr,
            TeleportSecret::ReceiverCode {
                code: request.code.into(),
                expires_at: request.expires_at,
            },
        )
    }

    fn new(qr: KeyTeleportQr, secret: TeleportSecret) -> Self {
        Self {
            playback: QrPlayback::static_frame(qr.bbqr),
            url: qr.url,
            secret,
        }
    }
}

/// Request data built off the UI thread, since playback owns UI-only state
pub(super) struct ReceiveRequestData {
    qr: KeyTeleportQr,
    code: String,
    expires_at: Timestamp,
}

impl ReceiveRequestData {
    pub(super) fn new(session: &KeyTeleportReceiveSession) -> Result<Self, KeyTeleportError> {
        let request = session.request()?;

        Ok(Self {
            qr: request.request.qr()?,
            code: request.numeric_code.grouped(),
            expires_at: request.expires_at,
        })
    }
}

impl KeyTeleportPage {
    pub(super) fn new(
        sessions: KeyTeleportSessionStore,
        window: &mut Window,
        cx: &mut Context<SignerLabApp>,
    ) -> Self {
        Self {
            sessions,
            mode: TeleportMode::Send,
            request_text: text_area(window, cx, "B$2R… or https://keyteleport.com/#…"),
            code: input(window, cx, "Eight-digit receiver code"),
            response_text: text_area(window, cx, "B$2S… or https://keyteleport.com/#…"),
            password: input(window, cx, "Teleport password"),
            signer_name: input_with_value(window, cx, "Wallet name", "Teleported wallet"),
            sent: None,
            receiving: None,
            scan: None,
        }
    }

    /// The sent QR and password together reveal the wallet, so they never outlive their view
    pub(super) fn clear_sent(&mut self) {
        self.sent = None;
    }
}

impl SignerLabApp {
    pub(super) fn set_teleport_mode(
        &mut self,
        mode: TeleportMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || self.key_teleport.mode == mode {
            return;
        }

        self.stop_camera_scan(cx);
        self.key_teleport.mode = mode;
        self.key_teleport.clear_sent();
        self.resume_teleport_receive(window, cx);
        cx.notify();
    }

    /// Shows the saved receive session, which an earlier app run or the CLI may have changed
    ///
    /// Import always uses the session on disk, so the shown request is reloaded rather than cached
    pub(super) fn resume_teleport_receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.key_teleport.mode != TeleportMode::Receive {
            return;
        }

        // the session file is tiny, so read it inline rather than skip it while other work runs
        let request = self
            .key_teleport
            .sessions
            .active()
            .and_then(|session| session.as_ref().map(ReceiveRequestData::new).transpose());
        match request {
            Ok(request) => self.key_teleport.receiving = request.map(TeleportedQr::receiving),
            Err(error) => {
                self.key_teleport.receiving = None;
                self.set_error(error_message(error), window, cx);
            }
        }
    }

    fn start_teleport_receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sessions = self.key_teleport.sessions.clone();
        self.run(
            "Starting receive",
            ResultScope::Page(Page::KeyTeleport),
            window,
            cx,
            async move {
                let session = sessions.start().map_err(error_message)?;
                ReceiveRequestData::new(&session).map_err(error_message)
            },
            |app, request, window, cx| {
                app.key_teleport.receiving = Some(TeleportedQr::receiving(request));
                app.key_teleport
                    .response_text
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.key_teleport
                    .password
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.set_status("Receive session started", cx);
            },
        );
    }

    fn cancel_teleport_receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sessions = self.key_teleport.sessions.clone();
        self.stop_camera_scan(cx);
        self.run(
            "Cancelling receive",
            ResultScope::Page(Page::KeyTeleport),
            window,
            cx,
            async move { sessions.cancel().map_err(error_message) },
            |app, (), _, cx| {
                app.key_teleport.receiving = None;
                app.set_status("Receive session cancelled", cx);
            },
        );
    }

    fn send_key_teleport(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            return;
        };

        let request = self.key_teleport.request_text.read(cx).value().to_string();
        let code = self.key_teleport.code.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        self.stop_camera_scan(cx);
        self.run(
            "Teleporting wallet",
            ResultScope::Page(Page::KeyTeleport),
            window,
            cx,
            async move {
                let request = KeyTeleportRequest::from_str(&request).map_err(error_message)?;
                let code = NumericCode::from_str(&code).map_err(error_message)?;
                let secret = repository
                    .lock()
                    .await
                    .reveal_secret(signer.id)
                    .map_err(error_message)?;
                let sent = request
                    .send_mnemonic(&secret, &code)
                    .map_err(error_message)?;
                let qr = sent.response.qr().map_err(error_message)?;

                Ok((qr, sent.password.grouped()))
            },
            |app, (qr, password), _, cx| {
                app.key_teleport.sent = Some(TeleportedQr::sent(qr, password));
                app.set_status("Wallet encrypted for the receiver", cx);
            },
        );
    }

    fn finish_teleport_receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.key_teleport.signer_name.read(cx).value().to_string();
        let response = self.key_teleport.response_text.read(cx).value().to_string();
        let password = self.key_teleport.password.read(cx).value().to_string();
        let sessions = self.key_teleport.sessions.clone();
        let repository = Arc::clone(&self.repository);
        self.stop_camera_scan(cx);
        self.run(
            "Receiving wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let response = KeyTeleportResponse::from_str(&response).map_err(error_message)?;
                let password = TeleportPassword::from_str(&password).map_err(error_message)?;
                let mut repository = repository.lock().await;
                sessions
                    .import(&mut repository, name, 0, &response, &password)
                    .await
                    .map_err(error_message)
            },
            |app, signer, window, cx| {
                app.key_teleport.receiving = None;
                app.key_teleport
                    .response_text
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.key_teleport
                    .password
                    .update(cx, |input, cx| input.set_value("", window, cx));
                app.accept_imported_signer(signer, window, cx);
                app.set_status("Wallet received", cx);
            },
        );
    }

    fn start_teleport_scan(
        &mut self,
        target: TeleportScan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }

        let control = CancellationToken::new();
        let scan_control = control.clone();
        let (sender, receiver) = watch::channel(None);
        self.install_camera(control, receiver, cx);
        self.key_teleport.scan = Some(target);
        self.run(
            "Scan",
            ResultScope::Page(Page::KeyTeleport),
            window,
            cx,
            async move {
                tokio::task::spawn_blocking(move || {
                    let preview = |preview| {
                        sender.send_replace(Some(preview));
                    };
                    match target {
                        TeleportScan::Request => {
                            camera::scan_key_teleport::<KeyTeleportRequest>(&scan_control, preview)
                        }
                        TeleportScan::Response => {
                            camera::scan_key_teleport::<KeyTeleportResponse>(&scan_control, preview)
                        }
                    }
                })
                .await
                .map_err(error_message)?
            },
            move |app, text, window, cx| {
                app.camera = None;
                app.key_teleport.scan = None;
                let field = match target {
                    TeleportScan::Request => &app.key_teleport.request_text,
                    TeleportScan::Response => &app.key_teleport.response_text,
                };
                field.update(cx, |input, cx| input.set_value(text, window, cx));
                app.set_status("Scanned Key Teleport QR", cx);
            },
        );
    }

    fn copy_teleport_text(&mut self, text: String, label: &'static str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.set_status(label, cx);
    }

    pub(super) fn teleport_toolbar_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        components::segmented(
            "teleport-mode",
            [
                components::Segment {
                    value: TeleportMode::Send,
                    label: "Send".into(),
                    dot: None,
                },
                components::Segment {
                    value: TeleportMode::Receive,
                    label: "Receive".into(),
                    dot: None,
                },
            ],
            self.key_teleport.mode,
            self.busy(),
            Self::set_teleport_mode,
            cx,
        )
    }

    pub(super) fn key_teleport_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let panels = match self.key_teleport.mode {
            TeleportMode::Send => self.teleport_send_panels(cx),
            TeleportMode::Receive => self.teleport_receive_panels(cx),
        };

        div()
            .size_full()
            .p(px(design::CONTENT_PADDING))
            .child(panels)
    }

    fn teleport_send_panels(&self, cx: &Context<Self>) -> AnyElement {
        let Some(signer) = self.selected() else {
            return Self::no_wallet(cx).into_any_element();
        };

        let scanning = self.teleport_scanning(TeleportScan::Request);
        let request =
            components::group_box("Receiver request")
                .flex_1()
                .min_w_0()
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            v_form()
                                .child(
                                    field().label("Request").child(
                                        Textarea::new(&self.key_teleport.request_text)
                                            .h(px(96.0))
                                            .disabled(self.busy()),
                                    ),
                                )
                                .child(field().label("Code").child(
                                    Input::new(&self.key_teleport.code).disabled(self.busy()),
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("teleport-send")
                                        .primary()
                                        .label(format!("Send {}", signer.name))
                                        .disabled(self.busy())
                                        .on_click(cx.listener(|app, _, window, cx| {
                                            app.send_key_teleport(window, cx);
                                        })),
                                )
                                .child(self.teleport_scan_button(TeleportScan::Request, cx)),
                        )
                        .children(scanning.then(|| self.camera_preview(cx))),
                )
                .footer("Scan or paste the receiver's QR, then type the code it shows");

        let response = components::group_box("Sender response")
            .flex_1()
            .min_w_0()
            .child(match &self.key_teleport.sent {
                Some(sent) => self
                    .teleport_qr_panel(sent, cx)
                    .child(
                        h_flex().child(
                            Button::new("teleport-hide")
                                .ghost()
                                .small()
                                .label("Hide")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.key_teleport.clear_sent();
                                    cx.notify();
                                })),
                        ),
                    )
                    .into_any_element(),
                None => components::empty_state(IconName::ArrowLeftRight, "Nothing sent", None)
                    .into_any_element(),
            });

        h_flex()
            .gap_6()
            .items_start()
            .child(request)
            .child(response)
            .into_any_element()
    }

    fn teleport_receive_panels(&self, cx: &Context<Self>) -> AnyElement {
        let request = components::group_box("Receiver request")
            .flex_1()
            .min_w_0()
            .child(self.teleport_receive_request(cx))
            .footer("The sender scans this QR and types the receiver code");

        let receiving = self.key_teleport.receiving.is_some();
        let scanning = self.teleport_scanning(TeleportScan::Response);
        let response = components::group_box("Sender response")
            .flex_1()
            .min_w_0()
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        v_form()
                            .child(
                                field().label("Response").child(
                                    Textarea::new(&self.key_teleport.response_text)
                                        .h(px(96.0))
                                        .disabled(self.busy() || !receiving),
                                ),
                            )
                            .child(
                                field().label("Password").child(
                                    Input::new(&self.key_teleport.password)
                                        .disabled(self.busy() || !receiving),
                                ),
                            )
                            .child(
                                field().label("Name").child(
                                    Input::new(&self.key_teleport.signer_name)
                                        .disabled(self.busy() || !receiving),
                                ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("teleport-import")
                                    .primary()
                                    .label("Import wallet")
                                    .disabled(self.busy() || !receiving)
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.finish_teleport_receive(window, cx);
                                    })),
                            )
                            .child(
                                self.teleport_scan_button(TeleportScan::Response, cx)
                                    .disabled(!receiving || (self.busy() && !scanning)),
                            ),
                    )
                    .children(scanning.then(|| self.camera_preview(cx))),
            )
            .footer("Scan or paste the sender's QR, then type its password");

        h_flex()
            .gap_6()
            .items_start()
            .child(request)
            .child(response)
            .into_any_element()
    }

    fn teleport_receive_request(&self, cx: &Context<Self>) -> AnyElement {
        match &self.key_teleport.receiving {
            Some(receiving) => self
                .teleport_qr_panel(receiving, cx)
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("teleport-restart")
                                .outline()
                                .small()
                                .label("New request")
                                .disabled(self.busy())
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.start_teleport_receive(window, cx);
                                })),
                        )
                        .child(
                            Button::new("teleport-cancel")
                                .ghost()
                                .small()
                                .label("Cancel receive")
                                .disabled(self.busy())
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.cancel_teleport_receive(window, cx);
                                })),
                        ),
                )
                .into_any_element(),
            None => components::empty_state(
                IconName::ArrowLeftRight,
                "No receive session",
                Some(
                    Button::new("teleport-start")
                        .primary()
                        .label("Start receive")
                        .disabled(self.busy())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.start_teleport_receive(window, cx);
                        })),
                ),
            )
            .into_any_element(),
        }
    }

    /// A stopped or failed scan leaves its target behind, so the live camera decides
    fn teleport_scanning(&self, target: TeleportScan) -> bool {
        self.key_teleport.scan == Some(target) && self.camera.is_some()
    }

    fn teleport_scan_button(&self, target: TeleportScan, cx: &Context<Self>) -> Button {
        let scanning = self.teleport_scanning(target);
        Button::new(match target {
            TeleportScan::Request => "teleport-scan-request",
            TeleportScan::Response => "teleport-scan-response",
        })
        .outline()
        .icon(IconName::Camera)
        .label(if scanning { "Stop camera" } else { "Scan" })
        .disabled(self.busy() && !scanning)
        .on_click(cx.listener(move |app, _, window, cx| {
            if scanning {
                app.stop_camera_scan(cx);
            } else {
                app.start_teleport_scan(target, window, cx);
            }
        }))
    }

    fn teleport_qr_panel(&self, qr: &TeleportedQr, cx: &Context<Self>) -> gpui_kit::Div {
        let (label, secret, warning, expires_at) = match &qr.secret {
            TeleportSecret::Password(password) => (
                "Password",
                password,
                Some("Anyone with this QR and password controls this wallet."),
                None,
            ),
            TeleportSecret::ReceiverCode { code, expires_at } => {
                ("Receiver code", code, None, Some(*expires_at))
            }
        };
        let copied = secret.replace(' ', "");
        let url = qr.url.clone();
        v_flex()
            .gap_4()
            .children(warning.map(|warning| Alert::warning("teleport-warning", warning).small()))
            .child(components::qr_output(
                &qr.playback,
                self.qr_preview_size,
                cx,
            ))
            .child(
                v_flex()
                    .gap_1()
                    .child(components::caption(label, cx))
                    .child(
                        div()
                            .text_size(px(22.0))
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(secret.clone()),
                    ),
            )
            .children(expires_at.map(|expires_at| {
                components::caption(format!("Expires {}", local_time(expires_at)), cx)
            }))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("teleport-copy-secret")
                            .outline()
                            .small()
                            .label(format!("Copy {}", label.to_lowercase()))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.copy_teleport_text(copied.clone(), "Copied", cx);
                            })),
                    )
                    .child(
                        Button::new("teleport-copy-link")
                            .outline()
                            .small()
                            .icon(IconName::Link)
                            .label("Copy link")
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.copy_teleport_text(url.clone(), "Copied link", cx);
                            })),
                    ),
            )
    }
}

fn local_time(timestamp: Timestamp) -> String {
    timestamp
        .to_zoned(TimeZone::system())
        .strftime("%b %-d, %-I:%M %p")
        .to_string()
}
