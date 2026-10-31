use super::{
    SignerLabApp, components, design,
    state::{Activity, Outcome, Page},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    sidebar::{Sidebar, SidebarHeader, SidebarMenu, SidebarMenuItem},
    spinner::Spinner,
    status_bar::StatusBar,
    toolbar::Toolbar,
    v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use signer_lab_core::LabNetwork;

impl SignerLabApp {
    pub(super) fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let menu = SidebarMenu::new().children(Page::ALL.map(|page| {
            SidebarMenuItem::new(page.label())
                .icon(page_icon(page))
                .active(self.page == page)
                .on_click(cx.listener(move |app, _, window, cx| app.select_page(page, window, cx)))
        }));
        v_flex()
            .debug_selector(|| "app-sidebar".into())
            .w(px(design::SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                TitleBar::new()
                    .h(px(design::TOOLBAR_HEIGHT))
                    .border_b_0()
                    .bg(cx.theme().transparent),
            )
            .child(div().px_2().child(self.wallet_switcher(cx)))
            .child(
                Sidebar::new("navigation")
                    .bg(cx.theme().transparent)
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .border_r_0()
                    .child(menu),
            )
            .child(
                div().p_3().child(
                    components::segmented(
                        "network",
                        [LabNetwork::MutinyNet, LabNetwork::Testnet4]
                            .into_iter()
                            .map(|network| components::Segment {
                                value: network,
                                label: (network.to_string()).into(),
                                dot: Some(design::network_color(network)),
                            }),
                        self.network,
                        self.busy(),
                        |app, value, window, cx| {
                            app.set_network(value, window, cx);
                        },
                        cx,
                    )
                    .full_width(),
                ),
            )
    }

    fn wallet_switcher(&self, cx: &Context<Self>) -> gpui_kit::AnyElement {
        let Some(signer) = self.selected() else {
            return div()
                .id("new-wallet-switcher")
                .on_click(cx.listener(|app, _, window, cx| app.start_new_signer(window, cx)))
                .child(
                    SidebarHeader::new()
                        .h(px(44.0))
                        .rounded(px(8.0))
                        .items_center()
                        .justify_start()
                        .child(
                            div()
                                .size(px(28.0))
                                .rounded(px(6.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Icon::new(IconName::Plus)),
                        )
                        .child("New wallet"),
                )
                .into_any_element();
        };

        let letter = signer
            .name
            .to_string()
            .chars()
            .next()
            .unwrap_or('?')
            .to_string();
        let wallets = self.signers.clone();

        let selected = self.selected_signer;
        let busy = self.busy();
        let entity = cx.entity().downgrade();
        SidebarHeader::new()
            .h(px(44.0))
            .rounded(px(8.0))
            .items_center()
            .child(
                div()
                    .size(px(28.0))
                    .flex_shrink_0()
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().primary)
                    .bg(cx.theme().primary.opacity(0.15))
                    .font_semibold()
                    .child(letter),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_semibold()
                            .child(signer.name.to_string()),
                    )
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(11.0))
                            .text_color(cx.theme().muted_foreground)
                            .child(signer.fingerprint.to_string()),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(px(14.0))
                    .text_color(cx.theme().muted_foreground),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for signer in &wallets {
                    let id = signer.id;
                    let entity = entity.clone();
                    menu = menu.item(
                        PopupMenuItem::new(signer.name.to_string())
                            .checked(selected == Some(id))
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let _ =
                                    entity.update(cx, |app, cx| app.select_signer(id, window, cx));
                            }),
                    );
                }

                let entity = entity.clone();
                menu.separator()
                    .item(PopupMenuItem::new("New wallet…").disabled(busy).on_click(
                        move |_, window, cx| {
                            let _ = entity.update(cx, |app, cx| app.start_new_signer(window, cx));
                        },
                    ))
            })
            .into_any_element()
    }

    pub(super) fn toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let actions: Option<gpui_kit::AnyElement> = match self.page {
            Page::Wallet => Some(self.wallet_toolbar_actions(cx).into_any_element()),
            Page::SeedQr | Page::Export | Page::Bulk => None,
            Page::Sign => Some(self.sign_toolbar_actions(cx).into_any_element()),
            Page::KeyTeleport => Some(self.teleport_toolbar_actions(cx).into_any_element()),
        };
        // the style override removes TitleBar's macOS inset because traffic lights belong to the sidebar
        div()
            .debug_selector(|| "app-toolbar".into())
            .h(px(design::TOOLBAR_HEIGHT))
            .flex_shrink_0()
            .child(
                TitleBar::new()
                    .h(px(design::TOOLBAR_HEIGHT))
                    .pl_0()
                    .px_4()
                    .bg(cx.theme().background)
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(design::TOOLBAR_HEIGHT - 1.0))
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_semibold()
                                    .child(self.page.label()),
                            )
                            .children(actions.map(|actions| {
                                Toolbar::new("page-actions").small().content(actions)
                            })),
                    ),
            )
    }

    pub(super) fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let (indicator, text, color) = match &self.activity {
            Activity::Working(label) => (
                Spinner::new().small().into_any_element(),
                label.clone(),
                cx.theme().muted_foreground,
            ),
            Activity::Syncing { .. } => (
                Spinner::new().small().into_any_element(),
                "Syncing wallet".into(),
                cx.theme().muted_foreground,
            ),
            Activity::Idle => match &self.outcome {
                Outcome::Error(label) => (
                    Icon::new(IconName::CircleAlert)
                        .size(px(12.0))
                        .into_any_element(),
                    label.clone(),
                    cx.theme().danger,
                ),
                Outcome::Info(label) => (
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(cx.theme().success)
                        .into_any_element(),
                    label.clone(),
                    cx.theme().muted_foreground,
                ),
            },
        };

        let mut left = h_flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .text_color(color)
            .child(indicator)
            .child(div().truncate().child(text));
        if matches!(self.activity, Activity::Syncing { .. }) {
            left = left.child(
                Button::new("stop-sync-status")
                    .ghost()
                    .small()
                    .label("Stop")
                    .on_click(cx.listener(|app, _, _, cx| app.stop_sync(cx))),
            );
        }
        StatusBar::new()
            .h(px(design::STATUS_BAR_HEIGHT))
            .flex_shrink_0()
            .text_size(px(12.0))
            .left(left)
            .right(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(6.0))
                            .rounded_full()
                            .bg(design::network_color(self.network)),
                    )
                    .child(self.network.to_string()),
            )
    }
}

const fn page_icon(page: Page) -> IconName {
    match page {
        Page::Wallet => IconName::Wallet,
        Page::SeedQr => IconName::QrCode,
        Page::Export => IconName::FileOutput,
        Page::Sign => IconName::PenLine,
        Page::Bulk => IconName::Layers,
        Page::KeyTeleport => IconName::ArrowLeftRight,
    }
}
