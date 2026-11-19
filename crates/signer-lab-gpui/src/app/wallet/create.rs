use crate::app::{SignerLabApp, components, design, error_message, state::ResultScope};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _, StyledExt as _,
    button::{Button, ButtonVariants},
    form::{field, v_form},
    h_flex,
    input::{Input, Textarea},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use std::sync::Arc;

impl SignerLabApp {
    pub(in crate::app) fn import_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.seed_qr.signer_name.read(cx).value().to_string();
        let phrase = self.wallet.import_mnemonic.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        self.run(
            "Importing wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                repository
                    .lock()
                    .await
                    .import(name, 0, &phrase)
                    .await
                    .map_err(error_message)
            },
            Self::accept_imported_signer,
        );
    }

    fn wallet_import_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let scanning = self.camera.is_some();
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("wallet-import")
                    .primary()
                    .label("Import")
                    .disabled(self.busy())
                    .on_click(cx.listener(|app, _, window, cx| app.import_wallet(window, cx))),
            )
            .child(
                Button::new("wallet-open-seed")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label("Open SeedQR…")
                    .disabled(self.busy())
                    .on_click(
                        cx.listener(|app, _, window, cx| app.import_seed_qr_image(window, cx)),
                    ),
            )
            .child(
                Button::new("wallet-scan-seed")
                    .outline()
                    .icon(IconName::Camera)
                    .label(if scanning {
                        "Stop camera"
                    } else {
                        "Scan SeedQR"
                    })
                    .when(scanning, ButtonVariants::danger)
                    .disabled(self.busy() && !scanning)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        if scanning {
                            app.stop_camera_scan(cx);
                        } else {
                            app.start_seed_qr_camera_scan(window, cx);
                        }
                    })),
            )
    }

    pub(in crate::app) fn create_wallet_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut title = h_flex().justify_between().child(
            div()
                .text_size(px(20.0))
                .font_semibold()
                .child("New wallet"),
        );

        if !self.signers.is_empty() {
            title = title.child(
                Button::new("create-cancel")
                    .ghost()
                    .label("Cancel")
                    .disabled(self.busy())
                    .on_click(cx.listener(|app, _, window, cx| app.cancel_create(window, cx))),
            );
        }

        let generate = components::group_box("Generate").flex_1().min_w_0().child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("generate-12")
                        .primary()
                        .label("12 words")
                        .disabled(self.busy())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.create_signer(12, window, cx);
                        })),
                )
                .child(
                    Button::new("generate-24")
                        .outline()
                        .label("24 words")
                        .disabled(self.busy())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.create_signer(24, window, cx);
                        })),
                ),
        );

        let import = components::group_box("Import").flex_1().min_w_0().child(
            v_flex()
                .gap_3()
                .child(
                    Textarea::new(&self.wallet.import_mnemonic)
                        .h(px(96.0))
                        .disabled(self.busy()),
                )
                .child(self.wallet_import_actions(cx))
                .child(self.camera_preview(cx)),
        );

        v_flex()
            .id("create-wallet-page")
            .size_full()
            .p(px(design::CONTENT_PADDING))
            .gap_5()
            .overflow_y_scrollbar()
            .child(title)
            .child(
                v_form().child(
                    field()
                        .label("Name")
                        .child(Input::new(&self.seed_qr.signer_name).disabled(self.busy())),
                ),
            )
            .child(h_flex().gap_6().items_start().child(generate).child(import))
    }
}
