use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Corner, DragMoveEvent, Image, ImageFormat, InteractiveElement as _,
    IntoElement, ObjectFit, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, div, img, px, rgb, rgba,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    resizable::{h_resizable, resizable_panel, v_resizable},
    scroll::ScrollableElement,
    v_flex,
};
use signer_lab_core::{
    AddressType, ExportKind, LabNetwork, PlannedTransaction, PsbtReview, QrTransport, VirtualSigner,
};

use super::{
    SignerLabApp,
    components::{
        ActionRole, SegmentId, SegmentedControl, action_button, activity_row, field_label,
        form_action_row, form_action_stack, form_field, form_fields, form_section, inspector_field,
        metric, mono_text, page_heading, section_label, segmented_button, segmented_group,
        timeline_row, workflow_step,
    },
    design,
    state::{
        BulkMode, CameraState, DestinationMode, Page, QrAnimation, QrPlayback, QrPreviewSize,
        RenameState, ResizableTextArea, TextAreaDrag,
    },
};

impl SignerLabApp {
    pub(super) fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut navigation = v_flex().gap_1().w_full();
        for (index, page) in Page::ALL.into_iter().enumerate() {
            let selected = self.page == page;
            navigation = navigation.child(
                h_flex()
                    .id(("page", index))
                    .w_full()
                    .h(px(34.0))
                    .gap_3()
                    .px_2()
                    .rounded(px(design::CONTENT_RADIUS))
                    .cursor_pointer()
                    .text_color(if selected {
                        design::accent()
                    } else {
                        design::sidebar_text()
                    })
                    .when(selected, |row| row.bg(design::sidebar_selected()))
                    .when(!selected, |row| {
                        row.hover(|row| row.bg(design::sidebar_hover()))
                    })
                    .child(Icon::new(page_icon(page)).small())
                    .child(div().text_sm().child(page.label()))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.select_page(page, window, cx);
                    })),
            );
        }

        v_flex()
            .w(px(design::SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .p_4()
            .gap_3()
            .bg(design::sidebar())
            .rounded(px(design::SURFACE_RADIUS))
            .border_1()
            .border_color(design::sidebar_border())
            .shadow_sm()
            .text_color(design::sidebar_text())
            .child(Self::sidebar_brand())
            .child(
                v_flex()
                    .gap_2()
                    .pb_3()
                    .border_b_1()
                    .border_color(design::sidebar_border())
                    .child(Self::sidebar_section_label("Workflows"))
                    .child(navigation),
            )
            .child(div().flex_1())
    }

    fn sidebar_section_label(label: &'static str) -> impl IntoElement {
        div()
            .pt_2()
            .text_xs()
            .font_semibold()
            .text_color(design::sidebar_muted())
            .child(label.to_uppercase())
    }

    fn sidebar_brand() -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div().size(px(32.0)).rounded(px(8.0)).shadow_sm().child(
                    img(Arc::new(Image::from_bytes(
                        ImageFormat::Png,
                        include_bytes!("../../assets/app-icon.png").to_vec(),
                    )))
                    .size(px(32.0)),
                ),
            )
            .child(
                v_flex()
                    .gap_0()
                    .child(div().font_semibold().child("Signer Lab"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(design::sidebar_muted())
                            .child("Bitcoin test workbench"),
                    ),
            )
    }

    pub(super) fn workspace_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let network_group = segmented_group(SegmentedControl::Network)
            .disabled(self.busy)
            .child(segmented_button(
                SegmentId::NetworkMutinyNet,
                "MutinyNet",
                self.network == LabNetwork::MutinyNet,
            ))
            .child(segmented_button(
                SegmentId::NetworkTestnet4,
                "Testnet4",
                self.network == LabNetwork::Testnet4,
            ))
            .on_click(cx.listener(|app, selected: &Vec<usize>, window, cx| {
                let network = if selected.contains(&1) {
                    LabNetwork::Testnet4
                } else {
                    LabNetwork::MutinyNet
                };

                app.set_network(network, window, cx);
            }));

        h_flex()
            .w_full()
            .h(px(design::WORKSPACE_HEADER_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_6()
            .px_5()
            .border_b_1()
            .border_color(design::border())
            .bg(design::toolbar())
            .child(page_heading(self.page.label(), page_description(self.page)))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_4()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(design::muted())
                                    .child("Wallet"),
                            )
                            .child(self.wallet_switcher(cx)),
                    )
                    .child(network_group),
            )
    }

    fn wallet_switcher(&self, cx: &Context<Self>) -> gpui::AnyElement {
        if self.signers.is_empty() {
            return action_button(ActionRole::Primary, "create-first-wallet", "New wallet")
                .disabled(self.busy)
                .on_click(cx.listener(|app, _, window, cx| {
                    app.start_new_signer(window, cx);
                }))
                .into_any_element();
        }

        let selected = self.selected_signer;
        let current_name = self.selected().map_or_else(
            || "Select wallet".to_owned(),
            |signer| signer.name.to_string(),
        );
        let signers = self.signers.clone();
        let app = cx.entity();

        action_button(ActionRole::Secondary, "wallet-switcher", current_name)
            .disabled(self.busy)
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Corner::TopRight, move |mut menu, _, _| {
                menu = menu.item(PopupMenuItem::label("Switch wallet"));
                for signer in &signers {
                    let signer_id = signer.id;
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(signer.name.to_string())
                            .checked(selected == Some(signer_id))
                            .on_click(move |_, window, cx| {
                                app.update(cx, |app, cx| {
                                    app.select_signer(signer_id, window, cx);
                                });
                            }),
                    );
                }

                let app = app.clone();
                menu.separator()
                    .item(
                        PopupMenuItem::new("New wallet…").on_click(move |_, window, cx| {
                            app.update(cx, |app, cx| {
                                app.start_new_signer(window, cx);
                            });
                        }),
                    )
            })
            .into_any_element()
    }

    pub(super) fn wallet_page(&self, cx: &Context<Self>) -> impl IntoElement {
        if let Some(signer) = self.selected() {
            h_resizable("wallet-workbench")
                .child(
                    resizable_panel().size_range(px(520.0)..px(1_600.0)).child(
                        v_flex()
                            .size_full()
                            .min_w_0()
                            .child(self.wallet_signer_header(&signer, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scrollbar()
                                    .child(self.account_cards(cx)),
                            ),
                    ),
                )
                .child(
                    resizable_panel()
                        .size(px(design::INSPECTOR_WIDTH))
                        .size_range(px(300.0)..px(520.0))
                        .child(self.account_inspector(cx)),
                )
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .p_6()
                .gap_6()
                .overflow_y_scrollbar()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_2xl()
                                .font_semibold()
                                .child("Create a test wallet"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Emulate an air-gapped hardware wallet for testing"),
                        ),
                )
                .child(self.create_signer_panel(cx))
                .into_any_element()
        }
    }

    fn wallet_signer_header(&self, signer: &VirtualSigner, cx: &Context<Self>) -> impl IntoElement {
        let (accounts, balance, utxos) = self.wallet.as_ref().map_or((0, 0, 0), |wallet| {
            let balance = wallet
                .snapshots
                .iter()
                .map(|snapshot| snapshot.balance.total().to_sat())
                .sum::<u64>();
            let utxos = wallet
                .snapshots
                .iter()
                .map(|snapshot| snapshot.utxo_count)
                .sum::<usize>();

            (wallet.snapshots.len(), balance, utxos)
        });
        let name = if self.rename_state == RenameState::Editing(signer.id) {
            h_flex()
                .gap_2()
                .child(div().w(px(280.0)).child(Input::new(&self.rename_name)))
                .child(
                    action_button(ActionRole::Primary, "save-wallet-name", "Save")
                        .disabled(self.busy)
                        .on_click(cx.listener(|app, _, _, cx| app.rename_signer(cx))),
                )
                .child(
                    action_button(ActionRole::Secondary, "cancel-wallet-name", "Cancel")
                        .disabled(self.busy)
                        .on_click(cx.listener(|app, _, _, cx| app.cancel_rename_signer(cx))),
                )
                .into_any_element()
        } else {
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xl()
                        .font_semibold()
                        .child(signer.name.to_string()),
                )
                .child(
                    action_button(ActionRole::Secondary, "rename-wallet", "Rename")
                        .disabled(self.busy)
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.start_rename_signer(window, cx);
                        })),
                )
                .into_any_element()
        };

        v_flex()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(design::border())
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .child(
                        v_flex().gap_1().child(name).child(
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .text_color(design::muted())
                                .child("Fingerprint")
                                .child(mono_text(signer.fingerprint.to_string(), cx))
                                .child("·")
                                .child(self.network.to_string()),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(design::muted())
                            .child("Select an account to inspect its public data"),
                    ),
            )
            .child(self.wallet_actions(cx))
            .child(
                h_flex()
                    .mt_3()
                    .pt_3()
                    .border_t_1()
                    .border_color(design::border())
                    .child(metric("Accounts", accounts.to_string()))
                    .child(metric("Balance", format!("{balance} sats")))
                    .child(metric("UTXOs", utxos.to_string())),
            )
    }

    fn wallet_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        form_action_row()
            .mt_3()
            .child(
                action_button(ActionRole::Primary, "sync-wallet", "Sync")
                    .disabled(self.busy)
                    .on_click(cx.listener(|app, _, _, cx| app.sync_wallet(cx))),
            )
            .child(
                action_button(ActionRole::Secondary, "reveal-secret", "Reveal words")
                    .disabled(self.busy)
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.reveal_secret(window, cx);
                    })),
            )
            .child(
                action_button(ActionRole::Secondary, "export-seed-qr", "SeedQR")
                    .disabled(self.busy)
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.export_seed_qr(window, cx);
                    })),
            )
            .child(
                action_button(
                    ActionRole::Destructive,
                    "delete-signer",
                    if self.delete_armed {
                        "Confirm delete"
                    } else {
                        "Delete"
                    },
                )
                .disabled(self.busy || self.bulk.operation_id().is_some())
                .on_click(cx.listener(|app, _, window, cx| app.delete_signer(window, cx))),
            )
            .when(self.sync_control.is_some(), |row| {
                row.child(
                    action_button(ActionRole::Destructive, "stop-sync", "Stop sync")
                        .on_click(cx.listener(|app, _, _, cx| app.stop_sync(cx))),
                )
            })
    }

    fn account_cards(&self, cx: &Context<Self>) -> impl IntoElement {
        let account_index = self
            .selected()
            .map_or(0, |signer| signer.account_index.get());
        let mut rows = v_flex().w_full();
        if let Some(wallet) = &self.wallet {
            for (index, snapshot) in wallet.snapshots.iter().enumerate() {
                let address_type = snapshot.address_type;
                let selected = address_type == self.selected_account_type;
                rows = rows.child(
                    h_flex()
                        .id(("account-row", index))
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_3()
                        .cursor_pointer()
                        .when(selected, |row| {
                            row.bg(design::tint())
                                .border_l_2()
                                .border_color(design::accent())
                        })
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(design::border())
                        })
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.select_account(address_type, cx);
                        }))
                        .child(
                            div()
                                .w(px(190.0))
                                .text_sm()
                                .font_semibold()
                                .child(address_type_label(snapshot.address_type)),
                        )
                        .child(
                            mono_text(account_path(snapshot.address_type, account_index), cx)
                                .w(px(132.0))
                                .text_sm()
                                .text_color(design::muted()),
                        )
                        .child(
                            mono_text(snapshot.receive_address.clone(), cx)
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .truncate(),
                        )
                        .child(
                            div()
                                .w(px(112.0))
                                .text_sm()
                                .font_semibold()
                                .child(format!("{} sats", snapshot.balance.total().to_sat())),
                        )
                        .child(
                            div()
                                .w(px(96.0))
                                .text_xs()
                                .text_color(design::muted())
                                .child(format!(
                                    "{} UTXOs · {} reserved",
                                    snapshot.utxo_count, snapshot.locked_utxo_count
                                )),
                        ),
                );
            }
        } else {
            rows = rows.child(
                div()
                    .p_4()
                    .text_color(cx.theme().muted_foreground)
                    .child("Wallet accounts are not loaded"),
            );
        }

        v_flex()
            .w_full()
            .child(
                h_flex()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .bg(design::toolbar())
                    .border_b_1()
                    .border_color(design::border())
                    .text_xs()
                    .text_color(design::muted())
                    .child(section_label("ACCOUNT TYPE").w(px(190.0)))
                    .child(section_label("DERIVATION").w(px(132.0)))
                    .child(section_label("RECEIVE ADDRESS").flex_1().min_w_0())
                    .child(section_label("BALANCE").w(px(112.0)))
                    .child(section_label("UTXOS").w(px(96.0))),
            )
            .child(rows)
    }

    fn account_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let content = self.account_inspector_content(cx);

        v_flex()
            .size_full()
            .min_w_0()
            .bg(design::inspector())
            .child(
                h_flex()
                    .justify_between()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(design::border())
                    .child(div().text_lg().font_semibold().child("Account Inspector"))
                    .child(
                        action_button(ActionRole::Secondary, "open-exports", "Export").on_click(
                            cx.listener(|app, _, window, cx| {
                                app.select_page(Page::Export, window, cx);
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scrollbar()
                    .child(content),
            )
            .when(
                !self.output.read(cx).value().is_empty() || self.qr_playback.is_some(),
                |view| {
                    view.child(
                        div()
                            .border_t_1()
                            .border_color(design::border())
                            .child(self.output_panel(cx)),
                    )
                },
            )
    }

    fn account_inspector_content(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let Some(wallet) = &self.wallet else {
            return empty_account_inspector(cx);
        };
        let Some(snapshot) = wallet
            .snapshots
            .iter()
            .find(|snapshot| snapshot.address_type == self.selected_account_type)
        else {
            return empty_account_inspector(cx);
        };
        let Some(public) = wallet
            .accounts
            .iter()
            .find(|account| account.address_type == self.selected_account_type)
        else {
            return empty_account_inspector(cx);
        };

        v_flex()
            .w_full()
            .min_w_0()
            .gap_0()
            .child(inspector_field(
                "Account",
                address_type_label(snapshot.address_type).to_owned(),
                cx,
            ))
            .child(inspector_field(
                "Fingerprint",
                public.master_fingerprint.to_string(),
                cx,
            ))
            .child(inspector_field(
                "Derivation",
                public.account_path.to_string(),
                cx,
            ))
            .child(inspector_field(
                "Account XPUB",
                public.account_xpub.to_string(),
                cx,
            ))
            .child(inspector_field(
                "Receive address",
                snapshot.receive_address.clone(),
                cx,
            ))
            .child(inspector_field(
                "Balance",
                format!("{} sats", snapshot.balance.total().to_sat()),
                cx,
            ))
            .child(inspector_field(
                "UTXOs",
                format!(
                    "{} available · {} reserved",
                    snapshot.utxo_count, snapshot.locked_utxo_count
                ),
                cx,
            ))
            .into_any_element()
    }

    fn create_signer_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_5()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(div().font_semibold().child("New wallet"))
                    .child(div().flex_1().h(px(1.0)).bg(cx.theme().border)),
            )
            .child(form_field("Wallet name", Input::new(&self.signer_name)))
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .child(self.generate_signer_card(cx))
                    .child(self.import_signer_card(cx)),
            )
    }

    fn generate_signer_card(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .h(px(232.0))
            .gap_3()
            .p_4()
            .rounded_xl()
            .bg(design::canvas())
            .border_1()
            .border_color(design::border())
            .child(div().font_semibold().child("Generate a wallet"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Create new recovery words for a test wallet"),
            )
            .child(div().flex_1())
            .child(
                form_action_row()
                    .child(
                        action_button(ActionRole::Primary, "generate-12", "12 words")
                            .disabled(self.busy)
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.create_signer(12, window, cx);
                            })),
                    )
                    .child(
                        action_button(ActionRole::Secondary, "generate-24", "24 words")
                            .disabled(self.busy)
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.create_signer(24, window, cx);
                            })),
                    ),
            )
    }

    fn import_signer_card(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .h(px(232.0))
            .min_w_0()
            .gap_3()
            .p_4()
            .rounded_xl()
            .bg(design::canvas())
            .border_1()
            .border_color(design::border())
            .child(div().font_semibold().child("Import a wallet"))
            .child(form_field(
                "Recovery data",
                Input::new(&self.mnemonic).h(px(82.0)),
            ))
            .child(div().flex_1())
            .child(
                form_action_row()
                    .child(
                        action_button(ActionRole::Primary, "import", "Import text")
                            .disabled(self.busy)
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.import_signer(window, cx);
                            })),
                    )
                    .child(
                        action_button(ActionRole::Secondary, "import-seedqr-image", "Open SeedQR")
                            .disabled(self.busy)
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.import_seed_qr_image(window, cx);
                            })),
                    )
                    .child(
                        action_button(ActionRole::Secondary, "scan-seedqr", "Scan SeedQR")
                            .disabled(self.busy)
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.start_seed_qr_camera_scan(window, cx);
                            })),
                    ),
            )
    }

    pub(super) fn seed_qr_page(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("seedqr-workbench")
                        .child(
                            resizable_panel()
                                .size_range(px(360.0)..px(900.0))
                                .child(self.seed_qr_import_panel(cx)),
                        )
                        .child(
                            resizable_panel()
                                .size_range(px(360.0)..px(900.0))
                                .child(self.seed_qr_export_panel(cx)),
                        ),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(0x00fd_ba74))
                    .bg(rgba(0xfff7_ede6))
                    .text_sm()
                    .text_color(rgb(0x009c_2f0b))
                    .child(Icon::new(IconName::TriangleAlert).small())
                    .child("Recovery data")
                    .child("·")
                    .child("Keep this test secret private and offline"),
            )
    }

    fn seed_qr_import_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .gap_3()
            .p_4()
            .overflow_y_scrollbar()
            .border_r_1()
            .border_color(design::border())
            .child(div().text_lg().font_semibold().child("Import SeedQR"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Add a wallet name, then paste digits or select a QR image"),
            )
            .child(
                form_fields()
                    .child(form_field("Wallet name", Input::new(&self.signer_name)))
                    .child(form_field(
                        "Recovery data",
                        Input::new(&self.mnemonic).h(px(120.0)),
                    )),
            )
            .child(
                form_action_stack()
                    .child(
                        form_action_row()
                            .child(
                                action_button(
                                    ActionRole::Primary,
                                    "seedqr-import-text",
                                    "Import digits",
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |app, _, window, cx| {
                                        app.import_signer(window, cx);
                                    },
                                )),
                            )
                            .child(
                                action_button(
                                    ActionRole::Secondary,
                                    "seedqr-open-image",
                                    "Open image",
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |app, _, window, cx| {
                                        app.import_seed_qr_image(window, cx);
                                    },
                                )),
                            )
                            .child(
                                action_button(
                                    ActionRole::Secondary,
                                    "seedqr-scan-camera",
                                    "Scan camera",
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |app, _, window, cx| {
                                        app.start_seed_qr_camera_scan(window, cx);
                                    },
                                )),
                            )
                            .when(self.camera.is_some(), |row| {
                                row.child(
                                    action_button(
                                        ActionRole::Destructive,
                                        "seedqr-stop-camera",
                                        "Stop camera",
                                    )
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.stop_camera_scan(cx)),
                                    ),
                                )
                            }),
                    )
                    .when(self.camera.is_some(), |stack| {
                        stack.child(self.camera_preview_panel(cx))
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .pt_2()
                    .border_t_1()
                    .border_color(design::border())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Supported formats")
                    .child("Standard numeric")
                    .child("Compact binary"),
            )
    }

    fn seed_qr_export_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .gap_3()
            .p_4()
            .child(
                h_flex()
                    .justify_between()
                    .child(div().text_lg().font_semibold().child("Export SeedQR"))
                    .child(
                        action_button(ActionRole::Primary, "seedqr-create-export", "Create SeedQR")
                            .disabled(self.busy || self.selected_signer.is_none())
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.export_seed_qr(window, cx);
                            })),
                    ),
            )
            .child(self.qr_preview_panel(
                "No SeedQR yet",
                "Select a wallet and create a standard SeedQR",
                cx,
            ))
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child("Standard numeric payload"),
            )
            .child(self.resizable_text_area(ResizableTextArea::SeedQrOutput, &self.output, cx))
            .child(
                form_action_stack().child(
                    form_action_row()
                        .child(
                            action_button(
                                ActionRole::Secondary,
                                "seedqr-copy-digits",
                                "Copy digits",
                            )
                            .disabled(self.qr_playback.is_none())
                            .on_click(cx.listener(|app, _, _, cx| app.copy_output(cx))),
                        )
                        .child(
                            action_button(ActionRole::Secondary, "seedqr-save-image", "Save SVG")
                                .disabled(self.busy || self.qr_playback.is_none())
                                .on_click(cx.listener(|app, _, _, cx| app.save_seed_qr_image(cx))),
                        ),
                ),
            )
    }

    pub(super) fn export_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let address_types = address_types();
        let mut formats = h_flex().gap_0();
        for (index, address_type) in address_types.into_iter().enumerate() {
            formats = formats.child(self.export_format_card(
                address_type,
                index + 1 < address_types.len(),
                cx,
            ));
        }

        let content = v_flex()
            .w_full()
            .gap_4()
            .p_5()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Create Cove-compatible imports for Coldcard, Jade, and descriptor wallets",
                    ),
            )
            .child(
                v_flex()
                    .gap_0()
                    .overflow_hidden()
                    .rounded(px(design::CONTENT_RADIUS))
                    .bg(design::canvas())
                    .border_1()
                    .border_color(design::border())
                    .child(div().p_4().font_semibold().child("Hardware wallet formats"))
                    .child(self.hardware_export_row(
                        "coldcard-json",
                        "Coldcard JSON",
                        "Generic account export for Coldcard-compatible imports",
                        ExportKind::ColdcardGenericJson,
                        cx,
                    ))
                    .child(self.hardware_export_row(
                        "wasabi-json",
                        "Wasabi JSON",
                        "Account export for Wasabi-compatible imports",
                        ExportKind::WasabiJson,
                        cx,
                    ))
                    .child(self.hardware_export_row(
                        "jade-account",
                        "Jade account UR",
                        "Animated crypto-account UR for Blockstream Jade",
                        ExportKind::JadeCryptoAccount,
                        cx,
                    )),
            )
            .child(
                div()
                    .overflow_hidden()
                    .rounded(px(design::CONTENT_RADIUS))
                    .border_1()
                    .border_color(design::border())
                    .bg(design::canvas())
                    .child(formats),
            )
            .child(
                v_flex()
                    .overflow_hidden()
                    .rounded(px(design::CONTENT_RADIUS))
                    .border_1()
                    .border_color(design::border())
                    .bg(design::canvas())
                    .child(self.output_panel(cx)),
            );

        div().size_full().overflow_y_scrollbar().child(content)
    }

    fn hardware_export_row(
        &self,
        id: &'static str,
        label: &'static str,
        description: &'static str,
        kind: ExportKind,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .gap_3()
            .justify_between()
            .items_center()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(design::border())
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_semibold().child(label))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    ),
            )
            .child(
                action_button(ActionRole::Secondary, id, "Export")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.export(kind, window, cx);
                    })),
            )
    }

    fn export_format_card(
        &self,
        address_type: AddressType,
        show_divider: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let label = address_type_label(address_type);
        let buttons = [
            ("Multipath", ExportKind::MultipathDescriptor(address_type)),
            ("Descriptor pair", ExportKind::DescriptorPair(address_type)),
            ("Electrum JSON", ExportKind::ElectrumJson(address_type)),
            ("Account XPUB", ExportKind::AccountXpub(address_type)),
            ("BIP380 key", ExportKind::Bip380KeyExpression(address_type)),
            ("Jade HDKey UR", ExportKind::JadeCryptoHdkey(address_type)),
        ];
        let mut actions = v_flex();
        for (index, (button_label, kind)) in buttons.into_iter().enumerate() {
            actions = actions.child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_3()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(design::border())
                    })
                    .child(div().text_sm().child(button_label))
                    .child(
                        action_button(
                            ActionRole::Secondary,
                            ("format-export", address_type as usize * 10 + index),
                            "Copy",
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(
                            move |app, _, window, cx| {
                                app.export(kind, window, cx);
                            },
                        )),
                    ),
            );
        }

        v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .p_4()
            .when(show_divider, |card| {
                card.border_r_1().border_color(design::border())
            })
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(design::accent())
                    .child(label.to_uppercase()),
            )
            .child(actions)
    }

    pub(super) fn sign_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let review = self.reviewed_psbt.as_ref().map_or_else(
            || {
                div()
                    .p_4()
                    .text_color(cx.theme().muted_foreground)
                    .child("No reviewed PSBT")
            },
            |reviewed| review_panel(reviewed.review()),
        );

        let workspace = h_resizable("sign-columns")
            .child(
                resizable_panel()
                    .size(px(320.0))
                    .size_range(px(280.0)..px(480.0))
                    .child(
                        v_flex()
                            .size_full()
                            .min_w_0()
                            .p_4()
                            .child(self.psbt_input_panel(cx)),
                    ),
            )
            .child(
                resizable_panel().size_range(px(360.0)..px(900.0)).child(
                    v_flex()
                        .size_full()
                        .min_w_0()
                        .p_4()
                        .overflow_y_scrollbar()
                        .child(review),
                ),
            )
            .child(
                resizable_panel()
                    .size(px(design::INSPECTOR_WIDTH))
                    .size_range(px(300.0)..px(500.0))
                    .child(
                        v_flex()
                            .size_full()
                            .min_w_0()
                            .gap_3()
                            .p_4()
                            .bg(design::inspector())
                            .child(self.psbt_sign_actions(cx))
                            .child(self.output_panel(cx)),
                    ),
            );

        v_flex().size_full().child(Self::sign_progress_bar()).child(
            div().flex_1().min_h_0().child(
                v_resizable("sign-workbench")
                    .child(
                        resizable_panel()
                            .size_range(px(360.0)..px(1_200.0))
                            .child(workspace),
                    )
                    .child(
                        resizable_panel()
                            .size(px(132.0))
                            .size_range(px(96.0)..px(240.0))
                            .child(self.sign_activity_panel(cx)),
                    ),
            ),
        )
    }

    fn sign_progress_bar() -> impl IntoElement {
        h_flex()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(design::border())
            .child(div().flex_1().child(workflow_step("1", "Input")))
            .child(div().flex_1().child(workflow_step("2", "Review")))
            .child(div().flex_1().child(workflow_step("3", "Signed output")))
    }

    fn sign_activity_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let reviewed = self.reviewed_psbt.is_some();
        let signed = self.qr_playback.is_some();
        let rows = v_flex()
            .child(activity_row("PSBT decoded", reviewed, cx))
            .child(activity_row(
                "Structure and signer policy valid",
                reviewed,
                cx,
            ))
            .child(activity_row("Signed output created", signed, cx));

        v_flex()
            .size_full()
            .bg(design::inspector())
            .child(
                h_flex()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_b_1()
                    .border_color(design::border())
                    .child(div().font_semibold().child("Activity"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if reviewed { "Checks passed" } else { "Waiting" }),
                    ),
            )
            .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(rows))
    }

    fn psbt_input_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .overflow_y_scrollbar()
            .child(div().text_sm().font_semibold().child("PSBT input"))
            .child(self.resizable_text_area(ResizableTextArea::PsbtInput, &self.psbt_input, cx))
            .child(
                form_action_stack()
                    .child(
                        form_action_row()
                            .child(
                                action_button(ActionRole::Secondary, "psbt-file", "Open file")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.import_psbt_file(window, cx);
                                    })),
                            )
                            .child(
                                action_button(ActionRole::Secondary, "camera-scan", "Scan QR")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.start_camera_scan(window, cx);
                                    })),
                            )
                            .when(self.camera.is_some(), |row| {
                                row.child(
                                    action_button(
                                        ActionRole::Destructive,
                                        "camera-stop",
                                        "Stop camera",
                                    )
                                    .on_click(
                                        cx.listener(|app, _, _, cx| app.stop_camera_scan(cx)),
                                    ),
                                )
                            }),
                    )
                    .when(self.camera.is_some(), |stack| {
                        stack.child(self.camera_preview_panel(cx))
                    })
                    .child(
                        action_button(ActionRole::Primary, "decode", "Decode and review")
                            .disabled(self.busy)
                            .on_click(cx.listener(|app, _, _, cx| app.decode_and_review_psbt(cx))),
                    ),
            )
    }

    fn camera_preview_panel(&self, cx: &Context<Self>) -> gpui::AnyElement {
        self.camera
            .as_ref()
            .and_then(CameraState::preview)
            .map_or_else(
                || {
                    v_flex()
                        .h(px(240.0))
                        .items_center()
                        .justify_center()
                        .rounded(px(design::CONTENT_RADIUS))
                        .bg(design::toolbar())
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Starting camera…")
                        .into_any_element()
                },
                |preview| {
                    div()
                        .h(px(240.0))
                        .w_full()
                        .overflow_hidden()
                        .rounded(px(design::CONTENT_RADIUS))
                        .bg(rgb(0x0012_171c))
                        .child(
                            img(preview.clone())
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        )
                        .into_any_element()
                },
            )
    }

    fn psbt_sign_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(div().text_sm().font_semibold().child("Signed QR format"))
            .child(self.qr_transport_control(cx))
            .child(
                action_button(ActionRole::Primary, "sign", "Sign PSBT")
                    .disabled(self.busy || self.reviewed_psbt.is_none())
                    .on_click(cx.listener(|app, _, window, cx| app.sign_psbt(window, cx))),
            )
    }

    fn qr_transport_control(&self, cx: &Context<Self>) -> impl IntoElement {
        segmented_group(SegmentedControl::QrTransport)
            .child(segmented_button(
                SegmentId::QrTransportBbqr,
                "BBQR",
                self.qr_transport == QrTransport::Bbqr,
            ))
            .child(segmented_button(
                SegmentId::QrTransportUr,
                "UR",
                self.qr_transport == QrTransport::Ur,
            ))
            .on_click(cx.listener(|app, selected: &Vec<usize>, window, cx| {
                let transport = if selected.contains(&1) {
                    QrTransport::Ur
                } else {
                    QrTransport::Bbqr
                };

                app.set_qr_transport(transport, window, cx);
            }))
    }

    pub(super) fn bulk_page(&self, cx: &Context<Self>) -> impl IntoElement {
        let plan_summary = self.bulk_plan_summary();
        let controls = self.bulk_controls(cx);
        let summary = self.bulk_summary(plan_summary, cx);

        let workspace = h_resizable("bulk-columns")
            .child(
                resizable_panel()
                    .size_range(px(520.0)..px(1_300.0))
                    .child(controls),
            )
            .child(
                resizable_panel()
                    .size(px(design::INSPECTOR_WIDTH))
                    .size_range(px(320.0)..px(520.0))
                    .child(summary),
            );

        v_resizable("bulk-workbench")
            .child(
                resizable_panel()
                    .size_range(px(380.0)..px(1_200.0))
                    .child(workspace),
            )
            .child(
                resizable_panel()
                    .size(px(148.0))
                    .size_range(px(100.0)..px(260.0))
                    .child(self.bulk_activity_panel(cx)),
            )
    }

    fn bulk_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let destination_source = v_flex()
            .gap_3()
            .child(self.destination_mode_buttons(cx))
            .when(
                matches!(
                    self.destination_mode,
                    DestinationMode::AccountXpub | DestinationMode::SavedSigner
                ),
                |view| view.child(self.destination_address_type_buttons(cx)),
            )
            .when(
                self.destination_mode == DestinationMode::SavedSigner,
                |view| view.child(self.signer_destinations(cx)),
            );

        div().size_full().min_w_0().overflow_y_scrollbar().child(
            v_flex()
                .w_full()
                .max_w(px(960.0))
                .gap_6()
                .p_6()
                .child(form_section(
                    "Transaction shape",
                    self.bulk_mode_buttons(cx),
                ))
                .child(form_section("Destination source", destination_source))
                .when(
                    self.destination_mode != DestinationMode::SavedSigner,
                    |view| {
                        view.child(form_field(
                            "Destination",
                            Input::new(&self.destination).h(px(64.0)),
                        ))
                    },
                )
                .when(self.bulk_mode != BulkMode::Consolidate, |view| {
                    view.child(form_section(
                        "Output configuration",
                        h_flex()
                            .gap_4()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(field_label("Output count"))
                                    .child(Input::new(&self.count)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(field_label("Sats per output"))
                                    .child(Input::new(&self.amount)),
                            ),
                    ))
                })
                .child(form_field("Fee rate", Input::new(&self.fee_rate)))
                .child(
                    form_action_stack().child(
                        form_action_row()
                            .child(
                                action_button(ActionRole::Primary, "plan-bulk", "Plan transaction")
                                    .disabled(self.busy || self.bulk.operation_id().is_some())
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.plan_bulk(window, cx);
                                    })),
                            )
                            .when(self.bulk.operation_id().is_some(), |row| {
                                row.child(
                                    action_button(
                                        ActionRole::Primary,
                                        "execute-bulk",
                                        "Sign and broadcast",
                                    )
                                    .disabled(self.busy || self.bulk.is_failed())
                                    .on_click(cx.listener(
                                        |app, _, window, cx| {
                                            app.execute_bulk(window, cx);
                                        },
                                    )),
                                )
                                .child(
                                    action_button(
                                        ActionRole::Destructive,
                                        "cancel-bulk",
                                        "Cancel and release",
                                    )
                                    .disabled(self.busy)
                                    .on_click(cx.listener(
                                        |app, _, window, cx| {
                                            app.cancel_bulk(window, cx);
                                        },
                                    )),
                                )
                            }),
                    ),
                ),
        )
    }

    fn bulk_summary(&self, plan_summary: Option<String>, cx: &Context<Self>) -> impl IntoElement {
        let summary = plan_summary.map_or_else(
            || {
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().font_semibold().child("Plan summary"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No plan"),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(design::border()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "Configure the transaction, then create a plan to reserve inputs",
                            ),
                    )
                    .into_any_element()
            },
            |summary| {
                v_flex()
                    .gap_3()
                    .child(div().font_semibold().child("Plan summary"))
                    .child(div().h(px(1.0)).bg(design::border()))
                    .child(div().text_sm().child(summary))
                    .into_any_element()
            },
        );

        v_flex()
            .size_full()
            .min_w_0()
            .gap_3()
            .p_4()
            .bg(design::inspector())
            .child(summary)
            .child(self.bulk_state_timeline(cx))
            .when(
                !self.output.read(cx).value().is_empty() || self.qr_playback.is_some(),
                |view| view.child(self.output_panel(cx)),
            )
    }

    fn bulk_state_timeline(&self, cx: &Context<Self>) -> impl IntoElement {
        let planned = self.bulk.operation_id().is_some();
        let completed = planned && !self.busy && !self.output.read(cx).value().is_empty();

        v_flex()
            .gap_1()
            .pt_3()
            .border_t_1()
            .border_color(design::border())
            .child(div().text_sm().font_semibold().child("State timeline"))
            .child(timeline_row("Planned", planned, cx))
            .child(timeline_row("Reserved", planned, cx))
            .child(timeline_row("Signed and broadcast", completed, cx))
    }

    fn bulk_activity_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let planned = self.bulk.operation_id().is_some();
        let completed = planned && !self.busy && !self.output.read(cx).value().is_empty();
        let rows = v_flex()
            .child(activity_row("Draft configured", true, cx))
            .child(activity_row(
                "Plan created and inputs reserved",
                planned,
                cx,
            ))
            .child(activity_row(
                "Transaction signed and broadcast",
                completed,
                cx,
            ));

        v_flex()
            .size_full()
            .bg(design::inspector())
            .child(
                h_flex()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_b_1()
                    .border_color(design::border())
                    .child(div().font_semibold().child("Operation log"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if completed {
                                "Completed"
                            } else {
                                "Current session"
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(rows))
    }

    fn bulk_mode_buttons(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut modes = segmented_group(SegmentedControl::BulkMode)
            .disabled(self.bulk.operation_id().is_some());
        for (segment, mode) in [
            (SegmentId::BulkModeFanOut, BulkMode::FanOut),
            (SegmentId::BulkModeBatch, BulkMode::Batch),
            (SegmentId::BulkModeConsolidate, BulkMode::Consolidate),
        ] {
            modes = modes.child(segmented_button(
                segment,
                mode.label(),
                self.bulk_mode == mode,
            ));
        }

        modes.on_click(cx.listener(|app, selected: &Vec<usize>, _, cx| {
            if let Some(mode) = selected.first().and_then(|index| BulkMode::ALL.get(*index)) {
                app.bulk_mode = *mode;
                cx.notify();
            }
        }))
    }

    fn bulk_plan_summary(&self) -> Option<String> {
        self.bulk.plan().map(|plan| {
            let inputs = plan
                .transactions()
                .iter()
                .map(PlannedTransaction::input_count)
                .sum::<usize>();
            let outputs = plan
                .transactions()
                .iter()
                .map(PlannedTransaction::output_count)
                .sum::<usize>();

            format!(
                "{} transaction(s) · {inputs} inputs · {outputs} outputs · {} sat/vB requested",
                plan.transactions().len(),
                plan.requested_fee_rate()
            )
        })
    }

    fn destination_mode_buttons(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut destination_modes = segmented_group(SegmentedControl::DestinationMode)
            .disabled(self.bulk.operation_id().is_some());
        for (segment, mode) in [
            (
                SegmentId::DestinationModeFixedAddress,
                DestinationMode::FixedAddress,
            ),
            (
                SegmentId::DestinationModeAccountXpub,
                DestinationMode::AccountXpub,
            ),
            (
                SegmentId::DestinationModeDescriptor,
                DestinationMode::Descriptor,
            ),
            (
                SegmentId::DestinationModeSavedSigner,
                DestinationMode::SavedSigner,
            ),
        ] {
            destination_modes = destination_modes.child(segmented_button(
                segment,
                mode.label(),
                self.destination_mode == mode,
            ));
        }

        destination_modes.on_click(cx.listener(|app, selected: &Vec<usize>, _, cx| {
            if let Some(mode) = selected
                .first()
                .and_then(|index| DestinationMode::ALL.get(*index))
            {
                app.destination_mode = *mode;
                cx.notify();
            }
        }))
    }

    fn destination_address_type_buttons(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut address_type_buttons = segmented_group(SegmentedControl::DestinationAddressType)
            .disabled(self.bulk.operation_id().is_some());
        for (segment, address_type) in [
            (SegmentId::DestinationAddressLegacy, AddressType::Legacy),
            (
                SegmentId::DestinationAddressWrappedSegwit,
                AddressType::WrappedSegwit,
            ),
            (
                SegmentId::DestinationAddressNativeSegwit,
                AddressType::NativeSegwit,
            ),
        ] {
            address_type_buttons = address_type_buttons.child(segmented_button(
                segment,
                address_type_short_label(address_type),
                self.destination_address_type == address_type,
            ));
        }

        address_type_buttons.on_click(cx.listener(|app, selected: &Vec<usize>, _, cx| {
            if let Some(address_type) = selected
                .first()
                .and_then(|index| address_types().get(*index).copied())
            {
                app.destination_address_type = address_type;
                cx.notify();
            }
        }))
    }

    fn signer_destinations(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut signer_destinations = h_flex().gap_2().flex_wrap();
        for (index, signer) in self.signers.iter().enumerate() {
            let signer_id = signer.id;
            if self.selected_signer == Some(signer_id) {
                continue;
            }

            signer_destinations = signer_destinations.child(
                action_button(
                    ActionRole::Secondary,
                    ("destination-signer", index),
                    signer.name.to_string(),
                )
                .selected(self.destination_signer == Some(signer_id))
                .disabled(self.bulk.operation_id().is_some())
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.destination_signer = Some(signer_id);
                    cx.notify();
                })),
            );
        }

        signer_destinations
    }

    fn output_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .p_4()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_4()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_3()
                            .items_center()
                            .child(div().font_semibold().child("Output"))
                            .when(self.export_artifact.is_some(), |row| {
                                row.child(self.qr_transport_control(cx))
                            }),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_2()
                            .items_center()
                            .when(self.export_artifact.is_some(), |row| {
                                row.child(
                                    action_button(
                                        ActionRole::Secondary,
                                        "save-export",
                                        "Save file",
                                    )
                                    .disabled(self.busy)
                                    .on_click(cx.listener(
                                        |app, _, _, cx| {
                                            app.save_export(cx);
                                        },
                                    )),
                                )
                            })
                            .child(
                                action_button(ActionRole::Secondary, "copy-output", "Copy")
                                    .on_click(cx.listener(|app, _, _, cx| app.copy_output(cx))),
                            ),
                    ),
            )
            .child(self.resizable_text_area(ResizableTextArea::WorkflowOutput, &self.output, cx))
            .when(self.qr_playback.is_some(), |view| {
                view.child(self.qr_preview_panel(
                    "No QR output",
                    "Create an export or sign a PSBT to show QR frames",
                    cx,
                ))
            })
    }

    fn qr_preview_panel(
        &self,
        empty_title: &'static str,
        empty_description: &'static str,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let Some(playback) = self.qr_playback.as_ref() else {
            return v_flex()
                .h(px(272.0))
                .items_center()
                .justify_center()
                .rounded(px(design::CONTENT_RADIUS))
                .bg(design::toolbar())
                .gap_2()
                .child(div().text_color(design::muted()).child(empty_title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(empty_description),
                )
                .into_any_element();
        };
        let Ok(qr_image) = crate::qr_image::render(
            playback.current_frame(),
            u32::from(self.qr_preview_size.side()),
        ) else {
            return div()
                .text_sm()
                .text_color(design::muted())
                .child("QR frame could not be rendered")
                .into_any_element();
        };
        let label = playback.transport().map_or_else(
            || "QR code".to_owned(),
            |transport| {
                format!(
                    "{} · Frame {} of {}",
                    qr_transport_label(transport),
                    playback.frame_index() + 1,
                    playback.frame_count()
                )
            },
        );
        v_flex()
            .items_center()
            .gap_3()
            .p_4()
            .rounded(px(design::CONTENT_RADIUS))
            .bg(design::toolbar())
            .child(qr_preview_stage(
                qr_image,
                f32::from(self.qr_preview_size.side()),
            ))
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().font_semibold().child(label))
                    .child(self.qr_preview_controls(playback, cx)),
            )
            .into_any_element()
    }

    fn qr_preview_controls(&self, playback: &QrPlayback, cx: &Context<Self>) -> gpui::Div {
        h_flex()
            .w_full()
            .gap_2()
            .flex_wrap()
            .justify_center()
            .child(
                action_button(ActionRole::Secondary, "smaller-qr", "Smaller")
                    .disabled(self.qr_preview_size == QrPreviewSize::Small)
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.shrink_qr_preview(cx);
                    })),
            )
            .child(
                div()
                    .w(px(52.0))
                    .text_center()
                    .text_xs()
                    .text_color(design::muted())
                    .child(self.qr_preview_size.label()),
            )
            .child(
                action_button(ActionRole::Secondary, "larger-qr", "Larger")
                    .disabled(self.qr_preview_size == QrPreviewSize::Large)
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.grow_qr_preview(cx);
                    })),
            )
            .when(playback.frame_count() > 1, |row| {
                let animation_label = match playback.animation() {
                    Some(QrAnimation::Playing) => "Pause",
                    Some(QrAnimation::Paused) | None => "Play",
                };

                row.child(
                    action_button(
                        ActionRole::Secondary,
                        "toggle-qr-animation",
                        animation_label,
                    )
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.toggle_qr_animation(cx);
                    })),
                )
                .child(
                    action_button(ActionRole::Secondary, "next-qr-frame", "Next frame").on_click(
                        cx.listener(|app, _, _, cx| {
                            app.next_qr_frame(cx);
                        }),
                    ),
                )
            })
    }

    fn resizable_text_area(
        &self,
        area: ResizableTextArea,
        input: &gpui::Entity<InputState>,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let height = self.text_area_heights.height(area);
        let drag = TextAreaDrag::new(cx.entity_id(), area, height);
        let handle_group = format!("text-area-resize-handle-{}", area as usize);

        div()
            .relative()
            .h(height)
            .flex_shrink_0()
            .child(Input::new(input).size_full())
            .child(
                div()
                    .id(("text-area-resize", area as usize))
                    .group(handle_group.clone())
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w_full()
                    .h(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_row_resize()
                    .child(
                        div()
                            .text_color(design::border())
                            .group_hover(handle_group, |handle| handle.text_color(design::muted()))
                            .child(Icon::new(IconName::ChevronsUpDown).xsmall()),
                    )
                    .on_drag(drag, |drag, start, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.started_at(start.y))
                    })
                    .on_drag_move(cx.listener(
                        move |app, event: &DragMoveEvent<TextAreaDrag>, _, cx| {
                            let drag = event.drag(cx);
                            if !drag.belongs_to(cx.entity_id(), area) {
                                return;
                            }

                            app.text_area_heights
                                .resize(area, drag.height_at(event.event.position.y));
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }
}

fn empty_account_inspector(cx: &Context<SignerLabApp>) -> gpui::AnyElement {
    div()
        .p_4()
        .text_color(cx.theme().muted_foreground)
        .child("Select a wallet account")
        .into_any_element()
}

fn qr_preview_stage(image: Arc<Image>, stage_side: f32) -> gpui::Div {
    qr_preview_stage_surface(img(image).size_full(), stage_side)
}

fn qr_preview_stage_surface(content: impl IntoElement, stage_side: f32) -> gpui::Div {
    div()
        .debug_selector(|| "qr-preview-stage".to_owned())
        .flex()
        .size(px(stage_side))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(design::border())
        .bg(rgb(0x00ff_ffff))
        .shadow_xs()
        .child(
            div()
                .debug_selector(|| "qr-image-surface".to_owned())
                .size(px(stage_side - 2.0))
                .flex_shrink_0()
                .child(content),
        )
}

fn qr_transport_label(transport: QrTransport) -> &'static str {
    match transport {
        QrTransport::Bbqr => "BBQR",
        QrTransport::Ur => "UR",
    }
}

fn review_panel(review: &PsbtReview) -> gpui::Div {
    let fee = review.fee.map_or_else(
        || "unknown".to_owned(),
        |fee| format!("{} sats", fee.to_sat()),
    );
    let mut outputs = v_flex().gap_1();
    for output in &review.outputs {
        let destination = output.address.as_deref().unwrap_or("non-standard script");
        let ownership = if output.owned_by_signer {
            " · change"
        } else {
            ""
        };
        outputs = outputs.child(div().text_sm().child(format!(
            "#{} · {} sats · {destination}{ownership}",
            output.index,
            output.amount.to_sat()
        )));
    }

    v_flex()
        .gap_2()
        .child(div().font_semibold().child("Transaction review"))
        .child(div().text_sm().child(format!(
            "{} inputs · {} match this signer · fee {fee}",
            review.input_count, review.matching_input_count
        )))
        .child(outputs)
}

const fn page_icon(page: Page) -> IconName {
    match page {
        Page::Wallet => IconName::LayoutDashboard,
        Page::Export => IconName::File,
        Page::Sign => IconName::SquareTerminal,
        Page::Bulk => IconName::GalleryVerticalEnd,
        Page::SeedQr => IconName::Frame,
    }
}

const fn page_description(page: Page) -> &'static str {
    match page {
        Page::Wallet => "Inspect accounts and manage the selected test wallet",
        Page::Export => "Create stable wallet imports for test devices and applications",
        Page::Sign => "Review and sign a Cove PSBT",
        Page::Bulk => "Plan and run recoverable test transactions",
        Page::SeedQr => "Import or export recovery data for test wallets",
    }
}

const fn address_types() -> [AddressType; 3] {
    [
        AddressType::Legacy,
        AddressType::WrappedSegwit,
        AddressType::NativeSegwit,
    ]
}

const fn address_type_label(address_type: AddressType) -> &'static str {
    match address_type {
        AddressType::Legacy => "BIP44 Legacy",
        AddressType::WrappedSegwit => "BIP49 Wrapped SegWit",
        AddressType::NativeSegwit => "BIP84 Native SegWit",
    }
}

fn account_path(address_type: AddressType, account_index: u32) -> String {
    let purpose = match address_type {
        AddressType::Legacy => 44,
        AddressType::WrappedSegwit => 49,
        AddressType::NativeSegwit => 84,
    };

    format!("m/{purpose}'/1'/{account_index}'")
}

const fn address_type_short_label(address_type: AddressType) -> &'static str {
    match address_type {
        AddressType::Legacy => "BIP44",
        AddressType::WrappedSegwit => "BIP49",
        AddressType::NativeSegwit => "BIP84",
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Styled as _, TestAppContext, div, point, px, size};

    use super::qr_preview_stage_surface;

    #[gpui::test]
    fn qr_stage_keeps_the_same_bounds_for_different_payload_densities(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(320.0), px(320.0)),
            |_, _| qr_preview_stage_surface(div().size(px(40.0)), 260.0),
        );
        let sparse_stage = cx
            .debug_bounds("qr-preview-stage")
            .expect("sparse QR stage is rendered");
        let sparse_image = cx
            .debug_bounds("qr-image-surface")
            .expect("sparse QR image is rendered");

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(320.0), px(320.0)),
            |_, _| qr_preview_stage_surface(div().size(px(500.0)), 260.0),
        );
        let dense_stage = cx
            .debug_bounds("qr-preview-stage")
            .expect("dense QR stage is rendered");
        let dense_image = cx
            .debug_bounds("qr-image-surface")
            .expect("dense QR image is rendered");

        assert_eq!(sparse_stage, dense_stage);
        assert_eq!(sparse_image, dense_image);
        assert_eq!(sparse_image.origin.x, sparse_stage.origin.x + px(1.0));
        assert_eq!(sparse_image.origin.y, sparse_stage.origin.y + px(1.0));
        assert_eq!(sparse_image.size.width, sparse_stage.size.width - px(2.0));
        assert_eq!(sparse_image.size.height, sparse_stage.size.height - px(2.0));
    }
}
