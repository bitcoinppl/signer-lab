mod create;
mod dialogs;

use super::components::{Align, ColumnWidth};
use super::{
    SignerLabApp, components, design, error_message, input,
    state::{Activity, AddressTypeLabels, Page, ResultScope, address_types},
    text_area,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants},
    description_list::DescriptionList,
    h_flex,
    input::{Input, InputEvent, InputState, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
    spinner::Spinner,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use signer_lab_core::{
    AccountPublicData, AddressType, NetworkWallet, SyncControl, WalletAccountSnapshot, sync_account,
};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct WalletView {
    pub(super) accounts: Vec<AccountPublicData>,
    pub(super) snapshots: Vec<WalletAccountSnapshot>,
}

impl WalletView {
    pub(super) fn from_wallet(wallet: &NetworkWallet) -> Result<Self, String> {
        Ok(Self {
            accounts: wallet.public_accounts().map_err(error_message)?,
            snapshots: address_types()
                .into_iter()
                .map(|kind| wallet.account(kind).snapshot())
                .collect(),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WalletPane {
    Selected,
    Create,
}

#[derive(Clone, Copy)]
pub(super) enum RenameState {
    Idle,
    Editing(signer_lab_core::SignerId),
}

pub(super) struct WalletPage {
    pub(super) pane: WalletPane,
    pub(super) selected_account: AddressType,
    pub(super) rename: RenameState,
    pub(super) rename_name: Entity<InputState>,
    pub(super) view: Option<WalletView>,
    pub(super) import_mnemonic: Entity<TextareaState>,
}

impl WalletPage {
    pub(super) fn new(creating: bool, window: &mut Window, cx: &mut Context<SignerLabApp>) -> Self {
        let rename_name = input(window, cx, "Wallet name");
        cx.subscribe_in(&rename_name, window, |app, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. })
                && matches!(app.wallet.rename, RenameState::Editing(_))
            {
                app.rename_signer(window, cx);
            }
        })
        .detach();

        Self {
            pane: if creating {
                WalletPane::Create
            } else {
                WalletPane::Selected
            },
            selected_account: AddressType::NativeSegwit,
            rename: RenameState::Idle,
            rename_name,
            view: None,
            import_mnemonic: text_area(window, cx, "Recovery words or SeedQR digits"),
        }
    }

    pub(super) fn reset(&mut self, creating: bool) {
        self.pane = if creating {
            WalletPane::Create
        } else {
            WalletPane::Selected
        };

        self.selected_account = AddressType::NativeSegwit;
        self.rename = RenameState::Idle;
        self.view = None;
    }
}

impl SignerLabApp {
    pub(super) fn load_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            self.wallet.view = None;
            return;
        };

        let repository = Arc::clone(&self.repository);
        let network = self.network;
        self.run(
            "Loading wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let mut repository = repository.lock().await;
                repository
                    .reconcile_bulk_reservations(&signer, network)
                    .await
                    .map_err(error_message)?;
                let wallet = repository
                    .open_wallet(&signer, network)
                    .map_err(error_message)?;
                let operation = repository
                    .current_bulk_operation(signer.id, network)
                    .await
                    .map_err(error_message)?;
                Ok((WalletView::from_wallet(&wallet)?, operation))
            },
            |app, (wallet, operation), _, cx| {
                app.wallet.view = Some(wallet);
                app.accept_current_bulk_operation(operation);
                cx.notify();
            },
        );
    }

    pub(super) fn refresh_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        let network = self.network;
        self.run(
            "Loading wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let repository = repository.lock().await;
                WalletView::from_wallet(
                    &repository
                        .open_wallet(&signer, network)
                        .map_err(error_message)?,
                )
            },
            |app, wallet, _, cx| {
                app.wallet.view = Some(wallet);
                cx.notify();
            },
        );
    }

    pub(super) fn start_rename_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };

        self.wallet.rename_name.update(cx, |input, cx| {
            input.set_value(signer.name.to_string(), window, cx);
            input.focus(window, cx);
        });

        self.wallet.rename = RenameState::Editing(signer.id);
        cx.notify();
    }

    fn rename_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let RenameState::Editing(id) = self.wallet.rename else {
            return;
        };

        let Some(signer) = self.signers.iter().find(|signer| signer.id == id).cloned() else {
            return;
        };

        let name = self.wallet.rename_name.read(cx).value().to_string();
        let repository = Arc::clone(&self.repository);
        self.run(
            "Renaming wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                repository
                    .lock()
                    .await
                    .rename(&signer, name)
                    .await
                    .map_err(error_message)
            },
            |app, renamed, _, cx| {
                if let Some(signer) = app
                    .signers
                    .iter_mut()
                    .find(|signer| signer.id == renamed.id)
                {
                    *signer = renamed;
                }

                app.wallet.rename = RenameState::Idle;
                app.set_status("Wallet renamed", cx);
            },
        );
    }

    pub(super) fn reveal_secret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        self.run(
            "Loading recovery words",
            ResultScope::Page(self.page),
            window,
            cx,
            async move {
                repository
                    .lock()
                    .await
                    .reveal_secret(signer.id)
                    .map(|secret| secret.expose_phrase().to_owned())
                    .map_err(error_message)
            },
            |_, words, window, cx| {
                Self::show_recovery_words(words, window, cx);
            },
        );
    }

    fn delete_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            return;
        };

        if self.bulk.workflow.operation_id().is_some() {
            return;
        }

        let id = signer.id;
        let repository = Arc::clone(&self.repository);
        self.run(
            "Deleting wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                repository
                    .lock()
                    .await
                    .delete(id)
                    .await
                    .map_err(error_message)
            },
            move |app, (), window, cx| {
                app.signers.retain(|signer| signer.id != id);
                let selected = app.signers.first().map(|signer| signer.id);
                app.set_wallet_scope(selected, app.network, window, cx);
                app.set_status("Wallet deleted", cx);
                app.load_wallet(window, cx);
            },
        );
    }

    pub(super) fn sync_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        let network = self.network;
        let control = SyncControl::new();
        let task_control = control.clone();
        self.run(
            "Syncing wallet",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let repository = repository.lock().await;
                let mut wallet = repository
                    .open_wallet(&signer, network)
                    .map_err(error_message)?;
                let result = async {
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
                            return Err("Wallet sync stopped".to_owned());
                        }
                    }
                    Ok(())
                }
                .await;
                // partial scans persist account updates even when the next account stops or fails
                drop(wallet);
                let wallet = repository
                    .open_wallet(&signer, network)
                    .map_err(error_message)?;
                Ok((result, WalletView::from_wallet(&wallet)?))
            },
            |app, (result, wallet), window, cx| {
                app.wallet.view = Some(wallet);
                match result {
                    Ok(()) => app.set_status("Wallet sync is complete", cx),
                    Err(error) => app.set_error(error, window, cx),
                }
            },
        );

        self.activity = Activity::Syncing { control };
    }

    pub(super) fn stop_sync(&mut self, cx: &mut Context<Self>) {
        if let Activity::Syncing { control } = &self.activity {
            control.cancel();
            self.set_status("Stopping wallet sync", cx);
        }
    }

    pub(super) fn wallet_toolbar_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.selected().is_none() || self.wallet.pane == WalletPane::Create {
            return h_flex();
        }

        let syncing = matches!(self.activity, Activity::Syncing { .. });
        let entity = cx.entity().downgrade();
        let busy = self.busy();
        let has_operation = self.bulk.workflow.operation_id().is_some();
        h_flex()
            .gap_2()
            .items_center()
            .child(
                (if syncing {
                    Button::new("sync").icon(Spinner::new())
                } else {
                    Button::new("sync").icon(IconName::RefreshCw)
                })
                .outline()
                .small()
                .label(if syncing { "Stop" } else { "Sync" })
                .disabled(self.busy() && !syncing)
                .on_click(cx.listener(move |app, _, window, cx| {
                    if syncing {
                        app.stop_sync(cx);
                    } else {
                        app.sync_wallet(window, cx);
                    }
                })),
            )
            .child(
                Button::new("rename")
                    .ghost()
                    .small()
                    .icon(IconName::Pencil)
                    .tooltip("Rename")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|app, _, window, cx| app.start_rename_signer(window, cx)),
                    ),
            )
            .child(
                Button::new("wallet-more")
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .tooltip("More")
                    .dropdown_menu(move |menu, _, _| {
                        let reveal = entity.clone();
                        let seed = entity.clone();
                        let delete = entity.clone();
                        menu.item(
                            PopupMenuItem::new("Show recovery words…")
                                .disabled(busy)
                                .on_click(move |_, window, cx| {
                                    let _ =
                                        reveal.update(cx, |app, cx| app.reveal_secret(window, cx));
                                }),
                        )
                        .item(PopupMenuItem::new("Show SeedQR").disabled(busy).on_click(
                            move |_, window, cx| {
                                let _ = seed.update(cx, |app, cx| {
                                    app.select_page(Page::SeedQr, window, cx);
                                    app.export_seed_qr(window, cx);
                                });
                            },
                        ))
                        .separator()
                        .item(
                            PopupMenuItem::element(|_, cx| {
                                div().text_color(cx.theme().danger).child("Delete wallet…")
                            })
                            .disabled(busy || has_operation)
                            .on_click(move |_, window, cx| {
                                let _ = delete.update(cx, |app, cx| {
                                    app.confirm_delete_signer(window, cx);
                                });
                            }),
                        )
                    }),
            )
    }

    pub(super) fn wallet_page(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.wallet.pane == WalletPane::Create || self.selected().is_none() {
            return self.create_wallet_view(cx).into_any_element();
        }

        let (total, utxos, reserved) = self.wallet.view.as_ref().map_or((0, 0, 0), |view| {
            view.snapshots
                .iter()
                .fold((0, 0, 0), |(balance, utxos, reserved), snapshot| {
                    (
                        balance + snapshot.balance.total().to_sat(),
                        utxos + snapshot.utxo_count,
                        reserved + snapshot.locked_utxo_count,
                    )
                })
        });

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px(px(design::CONTENT_PADDING))
                    .pt(px(20.0))
                    .pb(px(16.0))
                    .items_start()
                    .justify_between()
                    .child(self.wallet_header(cx))
                    .child(
                        v_flex()
                            .items_end()
                            .gap(px(2.0))
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .child(
                                        components::tabular(components::format_sats(total))
                                            .text_size(px(26.0))
                                            .font_semibold(),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(" sats"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{utxos} UTXOs · {reserved} reserved")),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_resizable("wallet-panels")
                            .child(
                                resizable_panel()
                                    .size_range(px(480.0)..px(f32::MAX))
                                    .child(self.accounts_view(cx)),
                            )
                            .child(
                                resizable_panel()
                                    .size(px(design::INSPECTOR_WIDTH))
                                    .size_range(px(280.0)..px(480.0))
                                    .child(self.account_inspector(cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn wallet_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let signer = self.selected().expect("selected wallet");
        let mut header = v_flex().gap(px(6.0));
        if matches!(self.wallet.rename, RenameState::Editing(_)) {
            header = header.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .on_key_down(cx.listener(|app, event: &gpui_kit::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            app.wallet.rename = RenameState::Idle;
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .child(
                        Input::new(&self.wallet.rename_name)
                            .small()
                            .w(px(280.0))
                            .disabled(self.busy()),
                    )
                    .child(
                        Button::new("rename-save")
                            .primary()
                            .small()
                            .label("Save")
                            .disabled(self.busy())
                            .on_click(
                                cx.listener(|app, _, window, cx| app.rename_signer(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("rename-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.wallet.rename = RenameState::Idle;
                                cx.notify();
                            })),
                    ),
            );
        } else {
            header = header.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_semibold()
                            .child(signer.name.to_string()),
                    )
                    .child(
                        Tag::custom(
                            design::network_color(self.network),
                            design::network_color(self.network),
                            design::network_color(self.network),
                        )
                        .outline()
                        .child(self.network.to_string()),
                    ),
            );
        }
        header = header.child(
            h_flex()
                .gap(px(6.0))
                .items_center()
                .text_size(px(12.0))
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("Fingerprint"),
                )
                .child(div().child(components::mono_value(
                    "fingerprint",
                    signer.fingerprint.to_string(),
                    cx,
                ))),
        );
        header
    }

    fn accounts_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(view) = &self.wallet.view else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(self.no_wallet_data(cx))
                .into_any_element();
        };

        let header = components::table_header(
            &[
                ("Type", ColumnWidth::Fixed(px(200.0)), Align::Left),
                ("Path", ColumnWidth::Fixed(px(140.0)), Align::Left),
                ("Receive address", ColumnWidth::Flex, Align::Left),
                ("Balance", ColumnWidth::Fixed(px(120.0)), Align::Right),
                ("UTXOs", ColumnWidth::Fixed(px(72.0)), Align::Right),
            ],
            cx,
        )
        .debug_selector(|| "wallet-table-header".into());

        let rows = view
            .snapshots
            .iter()
            .map(|snapshot| self.account_row(view, snapshot, cx));

        div()
            .id("wallet-accounts-table")
            .size_full()
            .overflow_x_scrollbar()
            .child(
                v_flex()
                    .size_full()
                    .min_w(px(700.0))
                    .justify_start()
                    .gap_0()
                    .child(header)
                    .children(rows),
            )
            .into_any_element()
    }

    fn account_row(
        &self,
        view: &WalletView,
        snapshot: &WalletAccountSnapshot,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let kind = snapshot.address_type;
        let path = view
            .accounts
            .iter()
            .find(|account| account.address_type == kind)
            .map_or_else(String::new, |account| account.account_path.to_string());
        let selected = self.wallet.selected_account == kind;
        h_flex()
            .id(("account", kind as usize))
            .debug_selector(move || format!("wallet-account-{}", kind as usize))
            .h(px(44.0))
            .flex_shrink_0()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().table_row_border)
            .bg(if selected {
                cx.theme().table_active
            } else {
                cx.theme().background
            })
            .relative()
            .when(selected, |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.0))
                        .bg(cx.theme().table_active_border),
                )
            })
            .hover(|style| style.bg(cx.theme().table_hover))
            .on_click(cx.listener(move |app, _, _, cx| {
                app.wallet.selected_account = kind;
                cx.notify();
            }))
            .child(
                h_flex()
                    .w(px(200.0))
                    .flex_shrink_0()
                    .gap_2()
                    .child(div().font_medium().child(kind.full_label()))
                    .child(Tag::secondary().small().child(kind.script_label())),
            )
            .child(
                components::table_cell(ColumnWidth::Fixed(px(140.0)), Align::Left)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(12.0))
                    .text_color(cx.theme().muted_foreground)
                    .child(path),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(12.0))
                    .child(components::abbreviate(&snapshot.receive_address, 10, 8)),
            )
            .child(
                components::table_cell(ColumnWidth::Fixed(px(120.0)), Align::Right)
                    .child(components::format_sats(snapshot.balance.total().to_sat())),
            )
            .child(
                components::table_cell(ColumnWidth::Fixed(px(72.0)), Align::Right)
                    .child(snapshot.utxo_count.to_string()),
            )
    }

    fn no_wallet_data(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.busy() {
            return div().child(Spinner::new()).into_any_element();
        }

        components::empty_state(
            IconName::Wallet,
            "Sync to load accounts",
            Some(
                Button::new("load-sync")
                    .outline()
                    .label("Sync")
                    .disabled(self.busy())
                    .on_click(cx.listener(|app, _, window, cx| app.sync_wallet(window, cx))),
            ),
        )
        .into_any_element()
    }

    fn account_inspector(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut inspector = v_flex()
            .id("account-inspector")
            .size_full()
            .justify_start()
            .bg(cx.theme().background)
            .p_4()
            .gap_4()
            .border_l_1()
            .border_color(cx.theme().border)
            .overflow_y_scrollbar();
        let Some(view) = &self.wallet.view else {
            return inspector;
        };

        let Some(account) = view
            .accounts
            .iter()
            .find(|account| account.address_type == self.wallet.selected_account)
        else {
            return inspector;
        };

        let Some(snapshot) = view
            .snapshots
            .iter()
            .find(|snapshot| snapshot.address_type == account.address_type)
        else {
            return inspector;
        };
        inspector = inspector
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_semibold()
                            .child(account.address_type.full_label()),
                    )
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(12.0))
                            .text_color(cx.theme().muted_foreground)
                            .child(account.account_path.to_string()),
                    ),
            )
            .child(Self::account_details(account, snapshot, cx))
            .child(
                h_flex().child(
                    Button::new("account-export")
                        .outline()
                        .label("Export…")
                        .disabled(self.busy())
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.export.address_type = app.wallet.selected_account;
                            app.select_page(Page::Export, window, cx);
                        })),
                ),
            );
        inspector
    }

    fn account_details(
        account: &AccountPublicData,
        snapshot: &WalletAccountSnapshot,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        DescriptionList::vertical()
            .columns(1)
            .bordered(false)
            .item(
                "Account XPUB",
                components::mono_value("account-xpub", account.account_xpub.to_string(), cx)
                    .into_any_element(),
                1,
            )
            .item(
                "Receive address",
                components::mono_value("account-address", snapshot.receive_address.clone(), cx)
                    .into_any_element(),
                1,
            )
            .item(
                "Master fingerprint",
                components::mono_value(
                    "account-fingerprint",
                    account.master_fingerprint.to_string(),
                    cx,
                )
                .into_any_element(),
                1,
            )
            .item(
                "Balance",
                components::amount_sats(snapshot.balance.total().to_sat()).into_any_element(),
                1,
            )
            .item(
                "UTXOs",
                format!(
                    "{} available · {} reserved",
                    snapshot.utxo_count, snapshot.locked_utxo_count
                ),
                1,
            )
    }
}
