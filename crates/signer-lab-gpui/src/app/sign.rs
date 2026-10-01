use super::components::{Align, ColumnWidth};
use super::{
    SignerLabApp, components, error_message,
    state::{Page, QrPlayback, ResultScope, WorkflowOutput},
    text_area,
};
use crate::camera;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Textarea, TextareaState},
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, FontFeatures, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Styled as _, Window, div, px,
};
use signer_lab_core::{
    PendingPsbt, PsbtDecodeProgress, PsbtOutputReview, PsbtReview, QrDensity, QrScanner,
    QrTransport, ReviewedPsbt, decode_psbt_text, read_psbt,
};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(super) struct SignPage {
    pub(super) psbt_input: Entity<TextareaState>,
    pub(super) reviewed: Option<ReviewedPsbt>,
    pub(super) scanner: QrScanner,
    pub(super) transport: QrTransport,
}

impl SignPage {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SignerLabApp>) -> Self {
        Self {
            psbt_input: text_area(window, cx, "Paste a PSBT, BBQR, or UR frames"),
            reviewed: None,
            scanner: QrScanner::new(),
            transport: QrTransport::Bbqr,
        }
    }

    pub(super) fn reset(&mut self, window: &mut Window, cx: &mut Context<SignerLabApp>) {
        self.reviewed = None;
        self.scanner.reset();
        self.transport = QrTransport::Bbqr;
        self.psbt_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
}

impl SignerLabApp {
    fn clear_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sign.reviewed = None;
        if matches!(self.output, Some(WorkflowOutput::Signed { .. })) {
            self.set_output(None, window, cx);
        }
    }

    fn decode_and_review_psbt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let Some(wallet_owner) = self.selected() else {
            return;
        };

        let value = self.sign.psbt_input.read(cx).value().to_string();
        self.clear_review(window, cx);
        let mut scanner = std::mem::replace(&mut self.sign.scanner, QrScanner::new());
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let captured = value.clone();
        self.run(
            "Reviewing PSBT",
            ResultScope::Page(Page::Sign),
            window,
            cx,
            async move {
                let decoded = decode_psbt_text(&value, &mut scanner).map_err(error_message)?;
                let reviewed = match decoded {
                    PsbtDecodeProgress::Complete(psbt) => {
                        let repository = repository.lock().await;
                        let wallet = repository
                            .open_wallet(&wallet_owner, network)
                            .map_err(error_message)?;
                        Some(
                            PendingPsbt::new(psbt)
                                .review(&wallet)
                                .map_err(error_message)?,
                        )
                    }
                    PsbtDecodeProgress::InProgress { .. } => None,
                };
                Ok((reviewed, scanner))
            },
            move |app, (reviewed, scanner), _, cx| {
                if app.sign.psbt_input.read(cx).value().as_ref() != captured {
                    return;
                }

                app.sign.scanner = scanner;
                app.sign.reviewed = reviewed;
                cx.notify();
            },
        );
    }

    fn import_psbt_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        self.clear_review(window, cx);
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open PSBT".into()),
        });
        self.run(
            "Open file…",
            ResultScope::Page(Page::Sign),
            window,
            cx,
            async move {
                let Some(path) = prompt
                    .await
                    .map_err(error_message)?
                    .map_err(error_message)?
                    .and_then(|paths| paths.into_iter().next())
                else {
                    return Ok(None);
                };

                tokio::task::spawn_blocking(move || {
                    read_psbt(path).map(Some).map_err(error_message)
                })
                .await
                .map_err(error_message)?
            },
            |app, psbt, window, cx| {
                if let Some(psbt) = psbt {
                    app.sign.psbt_input.update(cx, |input, cx| {
                        input.set_value(psbt.to_string(), window, cx);
                    });

                    app.decode_and_review_psbt(window, cx);
                }
            },
        );
    }

    fn start_camera_scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        self.clear_review(window, cx);
        let control = CancellationToken::new();
        let scan_control = control.clone();
        let (sender, receiver) = watch::channel(None);
        self.install_camera(control.clone(), receiver, cx);
        self.run(
            "Scan QR",
            ResultScope::Page(Page::Sign),
            window,
            cx,
            async move {
                let psbt = tokio::task::spawn_blocking(move || {
                    camera::scan_psbt(&scan_control, |preview| {
                        sender.send_replace(Some(preview));
                    })
                })
                .await
                .map_err(error_message)??;

                if control.is_cancelled() {
                    return Err("Camera scan stopped".to_owned());
                }
                Ok(psbt)
            },
            |app, psbt, window, cx| {
                app.camera = None;
                app.sign.psbt_input.update(cx, |input, cx| {
                    input.set_value(psbt.to_string(), window, cx);
                });

                app.decode_and_review_psbt(window, cx);
            },
        );
    }

    fn sign_psbt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wallet_owner) = self.selected() else {
            return;
        };

        let Some(reviewed) = self.sign.reviewed.clone() else {
            return;
        };

        let captured = self.sign.psbt_input.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let transport = self.sign.transport;
        self.run(
            "Signing PSBT",
            ResultScope::Page(Page::Sign),
            window,
            cx,
            async move {
                let repository = repository.lock().await;
                let wallet = repository
                    .open_wallet(&wallet_owner, network)
                    .map_err(error_message)?;
                let signed = reviewed.sign(&wallet, true).map_err(error_message)?;

                let frames = signed
                    .qr_frames(transport, QrDensity::default())
                    .map_err(error_message)?;
                Ok((signed, frames))
            },
            move |app, (signed, frames), window, cx| {
                if app.sign.psbt_input.read(cx).value().as_ref() != captured {
                    return;
                }

                if let Some(playback) = QrPlayback::animated(frames) {
                    app.set_output(
                        Some(WorkflowOutput::Signed { signed, playback }),
                        window,
                        cx,
                    );
                }
            },
        );
    }

    fn set_signed_transport(
        &mut self,
        transport: QrTransport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || self.sign.transport == transport {
            return;
        }

        self.sign.transport = transport;
        let Some(WorkflowOutput::Signed { signed, .. }) = self.visible_output() else {
            cx.notify();
            return;
        };

        let signed = signed.clone();
        let captured = self.sign.psbt_input.read(cx).value().to_string();
        self.run(
            "Creating signed QR",
            ResultScope::Page(Page::Sign),
            window,
            cx,
            async move {
                let frames = signed
                    .qr_frames(transport, QrDensity::default())
                    .map_err(error_message)?;
                Ok((signed, frames))
            },
            move |app, (signed, frames), window, cx| {
                if app.sign.psbt_input.read(cx).value().as_ref() != captured {
                    return;
                }

                if let Some(playback) = QrPlayback::animated(frames) {
                    app.set_output(
                        Some(WorkflowOutput::Signed { signed, playback }),
                        window,
                        cx,
                    );
                }
            },
        );
    }

    pub(super) fn sign_toolbar_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.selected().is_none() {
            return h_flex();
        }

        let scanning = self.camera.is_some();
        h_flex()
            .gap_2()
            .child(
                Button::new("psbt-open")
                    .outline()
                    .small()
                    .icon(IconName::FolderOpen)
                    .label("Open file…")
                    .disabled(self.busy())
                    .on_click(cx.listener(|app, _, window, cx| app.import_psbt_file(window, cx))),
            )
            .child(
                Button::new("psbt-camera")
                    .outline()
                    .small()
                    .icon(IconName::Camera)
                    .label(if scanning { "Stop camera" } else { "Scan QR" })
                    .disabled(self.busy() && !scanning)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        if scanning {
                            app.stop_camera_scan(cx);
                        } else {
                            app.start_camera_scan(window, cx);
                        }
                    })),
            )
    }

    pub(super) fn sign_page(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.selected().is_none() {
            return Self::no_wallet(cx).into_any_element();
        }

        let input = self.psbt_input_panel(cx);
        let mut review = v_flex()
            .id("psbt-review-pane")
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .gap_5()
            .overflow_y_scrollbar();
        if let Some(reviewed) = &self.sign.reviewed {
            let facts = reviewed.review();
            review = review.child(Self::sign_summary(facts, cx));
            if facts.matching_input_count == 0 {
                review = review.child(Alert::error(
                    "psbt-no-inputs",
                    "No inputs belong to this wallet",
                ));
            } else if facts.matching_input_count < facts.input_count {
                review = review.child(Alert::info(
                    "psbt-partial",
                    format!(
                        "Signs {} of {} inputs",
                        facts.matching_input_count, facts.input_count
                    ),
                ));
            }

            review = review.child(Self::sign_outputs(facts, cx));
            review = review.child(self.sign_actions(cx));
        } else {
            review = review.child(components::empty_state(
                IconName::ScanLine,
                "Paste or scan a PSBT",
                None,
            ));
        }

        if let Some(WorkflowOutput::Signed { playback, .. }) = self.visible_output() {
            review = review.child(
                h_flex()
                    .gap_6()
                    .w_full()
                    .min_h(px(320.0))
                    .items_stretch()
                    .flex_shrink_0()
                    .child(components::qr_output(playback, self.qr_preview_size, cx))
                    .child(components::payload_panel(
                        "Signed PSBT",
                        &self.display_output,
                        Button::new("signed-copy")
                            .outline()
                            .small()
                            .label("Copy")
                            .on_click(cx.listener(|app, _, _, cx| app.copy_output(cx))),
                        cx,
                    )),
            );
        }
        h_flex()
            .size_full()
            .child(input)
            .child(review)
            .into_any_element()
    }

    fn sign_summary(facts: &PsbtReview, cx: &Context<Self>) -> impl IntoElement {
        let value = || {
            div()
                .text_size(px(15.0))
                .font_semibold()
                .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
        };
        h_flex()
            .gap_8()
            .child(
                v_flex()
                    .gap_1()
                    .child(components::caption("Inputs", cx))
                    .child(
                        h_flex()
                            .child(value().child(facts.input_count.to_string()))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(" · {} yours", facts.matching_input_count)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(components::caption("Outputs", cx))
                    .child(value().child(facts.outputs.len().to_string())),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(components::caption("Fee", cx))
                    .child(value().child(match facts.fee {
                        Some(fee) => components::amount_sats(fee.to_sat()).into_any_element(),
                        None => div().child("Unknown").into_any_element(),
                    })),
            )
    }

    fn sign_outputs(facts: &PsbtReview, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_shrink_0()
            .child(components::table_header(
                &[
                    ("#", ColumnWidth::Fixed(px(40.0)), Align::Left),
                    ("Address", ColumnWidth::Flex, Align::Left),
                    ("Amount (sats)", ColumnWidth::Fixed(px(140.0)), Align::Right),
                ],
                cx,
            ))
            .children(
                facts
                    .outputs
                    .iter()
                    .map(|output| Self::sign_output_row(output, cx)),
            )
    }

    fn sign_output_row(output: &PsbtOutputReview, cx: &Context<Self>) -> impl IntoElement {
        let mut address = v_flex().flex_1().min_w_0().gap_1().items_start();
        address = address.child(match &output.address {
            Some(address) => {
                components::mono_value(("psbt-output", output.index), address.clone(), cx)
                    .into_any_element()
            }
            None => div()
                .text_color(cx.theme().muted_foreground)
                .child("Non-standard script")
                .into_any_element(),
        });

        if output.owned_by_signer {
            address = address.child(Tag::success().outline().small().child("Change"));
        }

        h_flex()
            .min_h(px(44.0))
            .px_4()
            .py_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().table_row_border)
            .hover(|style| style.bg(cx.theme().table_hover))
            .child(
                components::table_cell(ColumnWidth::Fixed(px(40.0)), Align::Left)
                    .child(output.index.to_string()),
            )
            .child(address)
            .child(
                components::table_cell(ColumnWidth::Fixed(px(140.0)), Align::Right)
                    .child(components::format_sats(output.amount.to_sat())),
            )
    }

    fn psbt_input_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(360.0))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .p_4()
            .gap_3()
            .child(components::caption("PSBT", cx))
            .child(
                Textarea::new(&self.sign.psbt_input)
                    .font_family(cx.theme().mono_font_family.clone())
                    .flex_1()
                    .min_h(px(120.0))
                    .disabled(self.busy()),
            )
            .child(self.camera_preview(cx))
            .child(
                Button::new("psbt-review")
                    .when(self.sign.reviewed.is_some(), Button::outline)
                    .when(self.sign.reviewed.is_none(), Button::primary)
                    .w_full()
                    .label("Review")
                    .disabled(self.busy() || self.sign.psbt_input.read(cx).value().is_empty())
                    .on_click(
                        cx.listener(|app, _, window, cx| app.decode_and_review_psbt(window, cx)),
                    ),
            )
    }

    fn sign_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .justify_between()
            .items_center()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_4()
            .child(
                h_flex()
                    .gap_3()
                    .child(components::caption("Signed QR", cx))
                    .child(components::segmented(
                        "signed-transport",
                        [QrTransport::Bbqr, QrTransport::Ur]
                            .into_iter()
                            .map(|transport| components::Segment {
                                value: transport,
                                label: (if transport == QrTransport::Bbqr {
                                    "BBQR"
                                } else {
                                    "UR"
                                })
                                .into(),
                                dot: None,
                            }),
                        self.sign.transport,
                        self.busy(),
                        |app, value, window, cx| {
                            app.set_signed_transport(value, window, cx);
                        },
                        cx,
                    )),
            )
            .child(
                Button::new("sign")
                    .primary()
                    .label("Sign")
                    .disabled(self.busy())
                    .on_click(cx.listener(|app, _, window, cx| app.sign_psbt(window, cx))),
            )
    }
}
