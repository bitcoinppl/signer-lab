use super::{
    SignerLabApp, components, design, error_message, input, save_directory,
    state::{Page, QrPlayback, ResultScope, WorkflowOutput},
    text_area,
};
use crate::camera;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    form::{field, v_form},
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    ClipboardItem, Context, Entity, Image, ImageFormat, IntoElement, ParentElement as _,
    PathPromptOptions, Styled as _, StyledImage as _, Task, Window, div, img, px,
};
use signer_lab_core::{StandardSeedQr, VirtualSigner};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(super) struct SeedQrPage {
    pub(super) signer_name: Entity<InputState>,
    pub(super) mnemonic: Entity<TextareaState>,
    pub(super) digits: Entity<InputState>,
}

impl SeedQrPage {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SignerLabApp>) -> Self {
        Self {
            signer_name: input(window, cx, "Wallet name"),
            mnemonic: text_area(window, cx, "SeedQR digits"),
            digits: input(window, cx, ""),
        }
    }
}

/// Closing or replacing a camera session cancels capture and its preview subscription
pub(super) struct CameraState {
    pub(super) control: CancellationToken,
    pub(super) preview: Option<Arc<Image>>,
    _watch: Task<()>,
}

impl Drop for CameraState {
    fn drop(&mut self) {
        self.control.cancel();
    }
}

impl SignerLabApp {
    pub(super) fn create_signer(
        &mut self,
        word_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self.seed_qr.signer_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let page_lifetime = Arc::clone(&self.page_lifetime);
        self.run(
            "Creating wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let created = repository
                    .lock()
                    .await
                    .generate(name, 0, word_count)
                    .await
                    .map_err(error_message)?;
                Ok((created.signer, created.secret.expose_phrase().to_owned()))
            },
            move |app, (signer, words), window, cx| {
                app.accept_imported_signer(signer, window, cx);
                app.set_status("Wallet created", cx);
                if app.page == Page::Wallet && Arc::ptr_eq(&page_lifetime, &app.page_lifetime) {
                    Self::show_recovery_words(words, window, cx);
                }
            },
        );
    }

    pub(super) fn import_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.seed_qr.signer_name.read(cx).value().to_string();
        let phrase = self.seed_qr.mnemonic.read(cx).value().to_string();
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

    pub(super) fn accept_imported_signer(
        &mut self,
        signer: VirtualSigner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = signer.id;
        self.signers.push(signer);
        self.set_wallet_scope(Some(id), self.network, window, cx);
        self.set_status("Wallet imported", cx);

        self.load_wallet(window, cx);
    }

    pub(super) fn import_seed_qr_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let name = self.seed_qr.signer_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open SeedQR image".into()),
        });
        self.run(
            "Open SeedQR…",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let paths = prompt
                    .await
                    .map_err(error_message)?
                    .map_err(error_message)?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };

                let secret = tokio::task::spawn_blocking(move || camera::read_seed_qr_image(&path))
                    .await
                    .map_err(error_message)??;
                repository
                    .lock()
                    .await
                    .import_secret(name, 0, secret)
                    .await
                    .map(Some)
                    .map_err(error_message)
            },
            |app, signer, window, cx| {
                if let Some(signer) = signer {
                    app.accept_imported_signer(signer, window, cx);
                }
            },
        );
    }

    pub(super) fn start_seed_qr_camera_scan(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }

        let control = CancellationToken::new();
        let scan_control = control.clone();
        let import_control = control.clone();
        let name = self.seed_qr.signer_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        let (sender, receiver) = watch::channel(None);
        self.install_camera(control, receiver, cx);
        self.run(
            "Scan",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let secret = tokio::task::spawn_blocking(move || {
                    camera::scan_seed_qr(&scan_control, |preview| {
                        sender.send_replace(Some(preview));
                    })
                })
                .await
                .map_err(error_message)??;

                if import_control.is_cancelled() {
                    return Err("Camera scan stopped".to_owned());
                }

                let mut repository = repository.lock().await;
                if import_control.is_cancelled() {
                    return Err("Camera scan stopped".to_owned());
                }
                repository
                    .import_secret(name, 0, secret)
                    .await
                    .map_err(error_message)
            },
            |app, signer, window, cx| {
                app.camera = None;
                app.accept_imported_signer(signer, window, cx);
            },
        );
    }

    pub(super) fn install_camera(
        &mut self,
        control: CancellationToken,
        mut receiver: watch::Receiver<Option<camera::CameraPreview>>,
        cx: &mut Context<Self>,
    ) {
        let watch = cx.spawn(async move |view, cx| {
            while receiver.changed().await.is_ok() {
                let preview = receiver.borrow_and_update().clone();
                let Some(preview) = preview else {
                    continue;
                };

                let image = Arc::new(Image::from_bytes(ImageFormat::Bmp, preview.into_bmp()));
                if view
                    .update(cx, |app, cx| {
                        if let Some(camera) = &mut app.camera {
                            camera.preview = Some(image);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        self.camera = Some(CameraState {
            control,
            preview: None,
            _watch: watch,
        });
    }

    pub(super) fn stop_camera_scan(&mut self, cx: &mut Context<Self>) {
        if self.camera.take().is_some() {
            self.set_status("Stopping camera scan", cx);
        }
    }

    pub(super) fn export_seed_qr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        self.run(
            "Creating SeedQR",
            ResultScope::Page(Page::SeedQr),
            window,
            cx,
            async move {
                let secret = repository
                    .lock()
                    .await
                    .reveal_secret(signer.id)
                    .map_err(error_message)?;
                StandardSeedQr::from_secret(&secret)
                    .and_then(|qr| qr.numeric_text())
                    .map(|qr| qr.to_string())
                    .map_err(error_message)
            },
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
    }

    fn save_seed_qr_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let Some(WorkflowOutput::SeedQr { playback }) = self.visible_output() else {
            return;
        };

        let frame = playback.current_frame().to_owned();
        let directory = match save_directory() {
            Ok(directory) => directory,
            Err(error) => {
                self.set_error(error, window, cx);
                return;
            }
        };

        let prompt = cx.prompt_for_new_path(&directory, Some("seedqr.svg"));
        self.run(
            "Save image…",
            ResultScope::Page(Page::SeedQr),
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
                    std::fs::write(&path, crate::qr_image::svg(&frame)?).map_err(error_message)?;
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

    pub(super) fn seed_qr_page(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.selected().is_none() {
            return Self::no_wallet(cx).into_any_element();
        }

        let import =
            components::group_box("Import")
                .flex_1()
                .min_w_0()
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            v_form()
                                .child(field().label("Name").child(
                                    Input::new(&self.seed_qr.signer_name).disabled(self.busy()),
                                ))
                                .child(
                                    field().label("Digits").child(
                                        Textarea::new(&self.seed_qr.mnemonic)
                                            .h(px(96.0))
                                            .disabled(self.busy()),
                                    ),
                                ),
                        )
                        .child(self.seed_import_actions(cx))
                        .child(self.camera_preview(cx)),
                )
                .footer("Standard and Compact SeedQR");
        h_flex()
            .size_full()
            .p(px(design::CONTENT_PADDING))
            .gap_6()
            .items_start()
            .child(import)
            .child(
                components::group_box("Export")
                    .flex_1()
                    .min_w_0()
                    .child(self.seed_qr_export_panel(cx)),
            )
            .into_any_element()
    }

    pub(super) fn copy_seed_qr_digits(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.visible_output(), Some(WorkflowOutput::SeedQr { .. })) {
            return;
        }

        cx.write_to_clipboard(ClipboardItem::new_string(
            self.seed_qr.digits.read(cx).value().to_string(),
        ));
        self.set_status("Copied digits", cx);
    }

    fn seed_qr_export_panel(&self, cx: &Context<Self>) -> gpui_kit::AnyElement {
        match self.visible_output() {
            Some(WorkflowOutput::SeedQr { playback }) => v_flex()
                .gap_4()
                .child(
                    gpui_kit::component::alert::Alert::warning(
                        "seed-warning",
                        "Anyone who scans this code controls this wallet.",
                    )
                    .small(),
                )
                .child(components::qr_output(playback, self.qr_preview_size, cx))
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .justify_between()
                                .child(components::caption("Digits", cx))
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .child(
                                            Button::new("seed-copy")
                                                .outline()
                                                .small()
                                                .label("Copy")
                                                .on_click(cx.listener(|app, _, _, cx| {
                                                    app.copy_seed_qr_digits(cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("seed-save")
                                                .outline()
                                                .small()
                                                .label("Save image…")
                                                .disabled(self.busy())
                                                .on_click(cx.listener(|app, _, window, cx| {
                                                    app.save_seed_qr_image(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("seed-hide")
                                                .ghost()
                                                .small()
                                                .label("Hide")
                                                .on_click(cx.listener(|app, _, window, cx| {
                                                    app.set_output(None, window, cx);
                                                })),
                                        ),
                                ),
                        )
                        .child(
                            Input::new(&self.seed_qr.digits)
                                .readonly(true)
                                .font_family(cx.theme().mono_font_family.clone()),
                        ),
                )
                .into_any_element(),
            _ => components::empty_state(
                IconName::QrCode,
                "SeedQR is hidden",
                Some(
                    Button::new("show-seedqr")
                        .primary()
                        .label("Show SeedQR")
                        .disabled(self.busy())
                        .on_click(cx.listener(|app, _, window, cx| app.export_seed_qr(window, cx))),
                ),
            )
            .into_any_element(),
        }
    }

    pub(super) fn seed_import_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let scanning = self.camera.is_some();
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("seed-import")
                    .primary()
                    .label("Import")
                    .disabled(self.busy())
                    .on_click(cx.listener(|app, _, window, cx| app.import_signer(window, cx))),
            )
            .child(
                Button::new("seed-open")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label("Open image…")
                    .disabled(self.busy())
                    .on_click(
                        cx.listener(|app, _, window, cx| app.import_seed_qr_image(window, cx)),
                    ),
            )
            .child(
                Button::new("seed-scan")
                    .outline()
                    .icon(IconName::Camera)
                    .label(if scanning { "Stop camera" } else { "Scan" })
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

    pub(super) fn camera_preview(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(camera) = &self.camera else {
            return div();
        };

        let content = camera.preview.as_ref().map_or_else(
            || {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("Starting camera…")
                    .into_any_element()
            },
            |image| {
                img(Arc::clone(image))
                    .size_full()
                    .object_fit(gpui_kit::ObjectFit::Contain)
                    .into_any_element()
            },
        );
        div()
            .h(px(240.0))
            .w_full()
            .rounded(px(8.0))
            .bg(cx.theme().secondary)
            .overflow_hidden()
            .child(content)
    }
}
