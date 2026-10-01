use super::{
    SignerLabApp, components, error_message, save_directory,
    state::{AddressTypeLabels, Page, QrPlayback, ResultScope, WorkflowOutput, address_types},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
    input::Textarea,
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div, px,
};
use signer_lab_core::{AddressType, ExportKind, QrDensity, QrTransport, WalletExporter};

pub(super) struct ExportPage {
    pub(super) address_type: AddressType,
    pub(super) transport: QrTransport,
}

impl Default for ExportPage {
    fn default() -> Self {
        Self {
            address_type: AddressType::NativeSegwit,
            transport: QrTransport::Bbqr,
        }
    }
}

impl SignerLabApp {
    pub(super) fn export(&mut self, kind: ExportKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wallet) = &self.wallet.view else {
            return;
        };

        let accounts = wallet.accounts.clone();
        let network = self.network;
        let transport = self.export.transport;
        self.run(
            "Creating export",
            ResultScope::Page(Page::Export),
            window,
            cx,
            async move {
                let artifact = WalletExporter::new(network, &accounts)
                    .and_then(|exporter| exporter.export(kind))
                    .map_err(error_message)?;
                let frames = artifact
                    .qr_frames(transport, QrDensity::default())
                    .map_err(error_message)?;
                Ok((artifact, frames))
            },
            |app, (artifact, frames), window, cx| {
                app.set_output(
                    Some(WorkflowOutput::Export {
                        artifact,
                        playback: QrPlayback::animated(frames),
                    }),
                    window,
                    cx,
                );

                app.set_status("Export is ready", cx);
            },
        );
    }

    fn set_export_transport(
        &mut self,
        transport: QrTransport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || self.export.transport == transport {
            return;
        }

        self.export.transport = transport;
        let Some(WorkflowOutput::Export { artifact, .. }) = self.visible_output() else {
            cx.notify();
            return;
        };

        let artifact = artifact.clone();
        self.run(
            "Creating export QR",
            ResultScope::Page(Page::Export),
            window,
            cx,
            async move {
                let frames = artifact
                    .qr_frames(transport, QrDensity::default())
                    .map_err(error_message)?;
                Ok((artifact, frames))
            },
            |app, (artifact, frames), window, cx| {
                app.set_output(
                    Some(WorkflowOutput::Export {
                        artifact,
                        playback: QrPlayback::animated(frames),
                    }),
                    window,
                    cx,
                );
            },
        );
    }

    fn save_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let Some(WorkflowOutput::Export { artifact, .. }) = self.visible_output() else {
            return;
        };

        let artifact = artifact.clone();
        let directory = match save_directory() {
            Ok(directory) => directory,
            Err(error) => {
                self.set_error(error, window, cx);
                return;
            }
        };

        let prompt = cx.prompt_for_new_path(&directory, Some(&artifact.file_name));
        self.run(
            "Save…",
            ResultScope::Page(Page::Export),
            window,
            cx,
            async move {
                let Some(path) = prompt
                    .await
                    .map_err(error_message)?
                    .map_err(error_message)?
                else {
                    return Ok(None);
                };

                tokio::task::spawn_blocking(move || {
                    artifact.write_to_path(&path).map_err(error_message)?;
                    Ok(Some(path))
                })
                .await
                .map_err(error_message)?
            },
            |app, path, _, cx| {
                if let Some(path) = path {
                    app.set_status(format!("Saved {}", path.display()), cx);
                }
            },
        );
    }

    pub(super) fn export_page(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.selected().is_none() {
            return Self::no_wallet(cx).into_any_element();
        }
        h_flex()
            .size_full()
            .child(self.export_formats(cx))
            .child(self.export_result(cx))
            .into_any_element()
    }

    fn export_formats(&self, cx: &Context<Self>) -> impl IntoElement {
        let kind = self.export.address_type;
        let formats = [
            ExportKind::ColdcardGenericJson,
            ExportKind::WasabiJson,
            ExportKind::JadeCryptoAccount,
            ExportKind::MultipathDescriptor(kind),
            ExportKind::DescriptorPair(kind),
            ExportKind::ElectrumJson(kind),
            ExportKind::AccountXpub(kind),
            ExportKind::Bip380KeyExpression(kind),
            ExportKind::JadeCryptoHdkey(kind),
        ];

        let mut list = v_flex()
            .id("export-formats")
            .w(px(280.0))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .p_3()
            .gap_4()
            .overflow_y_scrollbar()
            .child(components::caption("Wallet", cx));
        for (index, kind) in formats.into_iter().enumerate() {
            if index == 3 {
                list = list.child(components::caption("Account", cx)).child(
                    components::segmented(
                        "export-account",
                        address_types().into_iter().map(|kind| components::Segment {
                            value: kind,
                            label: (kind.short_label()).into(),
                            dot: None,
                        }),
                        self.export.address_type,
                        self.busy(),
                        |app, value, _, cx| {
                            app.export.address_type = value;
                            cx.notify();
                        },
                        cx,
                    )
                    .full_width(),
                );
            }

            let selected = matches!(self.visible_output(), Some(WorkflowOutput::Export { artifact, .. }) if artifact.kind == kind);
            list = list.child(
                Button::new(("format", index))
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(if selected {
                                cx.theme().list_active
                            } else {
                                cx.theme().background
                            })
                            .foreground(cx.theme().foreground)
                            .hover(cx.theme().list_hover)
                            .active(cx.theme().list_active),
                    )
                    // custom button variants reduce idle opacity; list selection needs the exact token
                    .bg(if selected {
                        cx.theme().list_active
                    } else {
                        cx.theme().background
                    })
                    .w_full()
                    .with_size(px(30.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .accessibility_label(export_label(kind))
                    .disabled(self.busy() || self.wallet.view.is_none())
                    .child(
                        h_flex()
                            .w_full()
                            .text_size(px(13.0))
                            .child(export_label(kind)),
                    )
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.export(kind, window, cx);
                    })),
            );
        }

        list
    }

    fn export_result(&self, cx: &Context<Self>) -> gpui_kit::AnyElement {
        match self.visible_output() {
            Some(WorkflowOutput::Export { artifact, playback }) => {
                let mut body = h_flex().flex_1().min_h_0().gap_6().items_start();
                if let Some(playback) = playback {
                    body = body.child(components::qr_output(playback, self.qr_preview_size, cx));
                }

                body = body.child(
                    Textarea::new(&self.display_output)
                        .readonly(true)
                        .font_family(cx.theme().mono_font_family.clone())
                        .flex_1()
                        .min_w_0()
                        .h_full(),
                );
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .p_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(15.0))
                                            .font_semibold()
                                            .child(export_label(artifact.kind)),
                                    )
                                    .children(
                                        export_account_type(artifact.kind)
                                            .map(|kind| Tag::secondary().child(kind.short_label())),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(components::segmented(
                                        "export-transport",
                                        [QrTransport::Bbqr, QrTransport::Ur].into_iter().map(
                                            |transport| components::Segment {
                                                value: transport,
                                                label: (if transport == QrTransport::Bbqr {
                                                    "BBQR"
                                                } else {
                                                    "UR"
                                                })
                                                .into(),
                                                dot: None,
                                            },
                                        ),
                                        self.export.transport,
                                        self.busy(),
                                        |app, value, window, cx| {
                                            app.set_export_transport(value, window, cx);
                                        },
                                        cx,
                                    ))
                                    .child(
                                        Button::new("export-copy")
                                            .outline()
                                            .small()
                                            .label("Copy")
                                            .on_click(
                                                cx.listener(|app, _, _, cx| app.copy_output(cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("export-save")
                                            .outline()
                                            .small()
                                            .label("Save…")
                                            .disabled(self.busy())
                                            .on_click(cx.listener(|app, _, window, cx| {
                                                app.save_export(window, cx);
                                            })),
                                    ),
                            ),
                    )
                    .child(body)
                    .into_any_element()
            }
            _ => components::empty_state(IconName::FileOutput, "Choose a format", None)
                .into_any_element(),
        }
    }
}

pub(super) const fn export_label(kind: ExportKind) -> &'static str {
    match kind {
        ExportKind::ColdcardGenericJson => "Coldcard JSON",
        ExportKind::WasabiJson => "Wasabi JSON",
        ExportKind::JadeCryptoAccount => "Jade account UR",
        ExportKind::MultipathDescriptor(_) => "Multipath descriptor",
        ExportKind::DescriptorPair(_) => "Descriptor pair",
        ExportKind::ElectrumJson(_) => "Electrum JSON",
        ExportKind::AccountXpub(_) => "Account XPUB",
        ExportKind::Bip380KeyExpression(_) => "BIP380 key",
        ExportKind::JadeCryptoHdkey(_) => "Jade HDKey UR",
    }
}

fn export_account_type(kind: ExportKind) -> Option<AddressType> {
    match kind {
        ExportKind::MultipathDescriptor(kind)
        | ExportKind::DescriptorPair(kind)
        | ExportKind::ElectrumJson(kind)
        | ExportKind::AccountXpub(kind)
        | ExportKind::Bip380KeyExpression(kind)
        | ExportKind::JadeCryptoHdkey(kind) => Some(kind),
        ExportKind::ColdcardGenericJson
        | ExportKind::WasabiJson
        | ExportKind::JadeCryptoAccount => None,
    }
}
