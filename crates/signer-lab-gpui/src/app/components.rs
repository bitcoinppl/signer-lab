use gpui::{
    Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use gpui_component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    h_flex, v_flex,
};

use super::{SignerLabApp, design};

#[derive(Clone, Copy)]
pub(super) enum ActionRole {
    Primary,
    Secondary,
    Destructive,
}

#[derive(Clone, Copy)]
pub(super) enum SegmentedControl {
    Network,
    QrTransport,
    BulkMode,
    DestinationMode,
    DestinationAddressType,
}

impl SegmentedControl {
    const fn name(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::QrTransport => "qr-transport",
            Self::BulkMode => "bulk-mode",
            Self::DestinationMode => "destination-mode",
            Self::DestinationAddressType => "destination-address-type",
        }
    }
}

impl From<SegmentedControl> for ElementId {
    fn from(control: SegmentedControl) -> Self {
        control.name().into()
    }
}

#[derive(Clone, Copy)]
pub(super) enum SegmentId {
    NetworkMutinyNet,
    NetworkTestnet4,
    QrTransportBbqr,
    QrTransportUr,
    BulkModeFanOut,
    BulkModeBatch,
    BulkModeConsolidate,
    DestinationModeFixedAddress,
    DestinationModeAccountXpub,
    DestinationModeDescriptor,
    DestinationModeSavedSigner,
    DestinationAddressLegacy,
    DestinationAddressWrappedSegwit,
    DestinationAddressNativeSegwit,
}

impl From<SegmentId> for ElementId {
    fn from(segment: SegmentId) -> Self {
        let (control, index) = match segment {
            SegmentId::NetworkMutinyNet => (SegmentedControl::Network, 0),
            SegmentId::NetworkTestnet4 => (SegmentedControl::Network, 1),
            SegmentId::QrTransportBbqr => (SegmentedControl::QrTransport, 0),
            SegmentId::QrTransportUr => (SegmentedControl::QrTransport, 1),
            SegmentId::BulkModeFanOut => (SegmentedControl::BulkMode, 0),
            SegmentId::BulkModeBatch => (SegmentedControl::BulkMode, 1),
            SegmentId::BulkModeConsolidate => (SegmentedControl::BulkMode, 2),
            SegmentId::DestinationModeFixedAddress => (SegmentedControl::DestinationMode, 0),
            SegmentId::DestinationModeAccountXpub => (SegmentedControl::DestinationMode, 1),
            SegmentId::DestinationModeDescriptor => (SegmentedControl::DestinationMode, 2),
            SegmentId::DestinationModeSavedSigner => (SegmentedControl::DestinationMode, 3),
            SegmentId::DestinationAddressLegacy => (SegmentedControl::DestinationAddressType, 0),
            SegmentId::DestinationAddressWrappedSegwit => {
                (SegmentedControl::DestinationAddressType, 1)
            }
            SegmentId::DestinationAddressNativeSegwit => {
                (SegmentedControl::DestinationAddressType, 2)
            }
        };

        Self::named_usize(control.name(), index)
    }
}

pub(super) fn segmented_group(control: SegmentedControl) -> ButtonGroup {
    ButtonGroup::new(control).compact().outline()
}

pub(super) fn action_button(
    role: ActionRole,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Button {
    let label = label.into();
    let button = Button::new(id)
        .small()
        .h(px(32.0))
        .px_3()
        .rounded(px(7.0))
        .border_1()
        .shadow_xs()
        .child(button_label(label));

    match role {
        ActionRole::Primary => button.primary(),
        ActionRole::Secondary => button.outline(),
        ActionRole::Destructive => button.danger(),
    }
}

pub(super) fn segmented_button(
    id: SegmentId,
    label: impl Into<SharedString>,
    selected: bool,
) -> Button {
    Button::new(id)
        .selected(selected)
        .child(button_label(label.into()))
}

fn button_label(label: SharedString) -> gpui::Div {
    h_flex()
        .h_full()
        .items_center()
        .justify_center()
        .pb(px(2.0))
        .line_height(px(14.0))
        .child(label)
}

pub(super) fn page_heading(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> gpui::Div {
    v_flex()
        .gap_1()
        .child(div().text_2xl().font_semibold().child(title.into()))
        .child(
            div()
                .text_sm()
                .text_color(design::muted())
                .child(description.into()),
        )
}

pub(super) fn app_shell(sidebar: impl IntoElement, workspace: impl IntoElement) -> gpui::Div {
    h_flex()
        .size_full()
        .min_h_0()
        .gap_3()
        .p_3()
        .bg(design::window_field())
        .child(sidebar)
        .child(workspace)
}

pub(super) fn workspace_surface(
    header: impl IntoElement,
    content: impl IntoElement,
    status: impl Into<SharedString>,
) -> gpui::Div {
    v_flex()
        .debug_selector(|| "workspace-surface".to_owned())
        .flex_1()
        .h_full()
        .min_w_0()
        .rounded(px(design::SURFACE_RADIUS))
        .p(px(1.0))
        .bg(design::border())
        .shadow_sm()
        .child(
            v_flex()
                .debug_selector(|| "workspace-inner-surface".to_owned())
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .rounded(px(design::SURFACE_INNER_RADIUS))
                .bg(design::canvas())
                .child(header)
                .child(div().flex_1().min_h_0().overflow_hidden().child(content))
                .child(workspace_status(status)),
        )
}

fn workspace_status(status: impl Into<SharedString>) -> gpui::Div {
    h_flex()
        .px_5()
        .py_3()
        .border_t_1()
        .border_color(design::border())
        .bg(design::toolbar())
        .text_color(design::muted())
        .text_xs()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .size(px(8.0))
                        .rounded_full()
                        .bg(design::status_green()),
                )
                .child(status.into()),
        )
}

pub(super) fn section_label(label: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_xs()
        .font_semibold()
        .text_color(design::muted())
        .child(label.into())
}

pub(super) fn field_label(label: impl Into<SharedString>) -> gpui::Div {
    section_label(label)
}

pub(super) fn metric(label: &'static str, value: impl Into<SharedString>) -> gpui::Div {
    v_flex()
        .flex_1()
        .gap_1()
        .child(section_label(label.to_uppercase()))
        .child(div().text_lg().font_semibold().child(value.into()))
}

pub(super) fn mono_text(value: impl Into<SharedString>, cx: &Context<SignerLabApp>) -> gpui::Div {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .child(value.into())
}

pub(super) fn inspector_field(
    label: &'static str,
    value: String,
    cx: &Context<SignerLabApp>,
) -> gpui::Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(design::border())
        .child(section_label(label.to_uppercase()))
        .child(
            mono_text(value, cx)
                .w_full()
                .min_w_0()
                .text_sm()
                .overflow_hidden()
                .whitespace_nowrap(),
        )
}

pub(super) fn activity_row(
    label: &'static str,
    complete: bool,
    cx: &Context<SignerLabApp>,
) -> gpui::Div {
    h_flex()
        .gap_3()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(design::border())
        .child(
            div()
                .text_color(if complete {
                    design::status_green().into()
                } else {
                    cx.theme().muted_foreground
                })
                .child(if complete { "●" } else { "○" }),
        )
        .child(div().flex_1().text_sm().child(label))
        .child(
            div()
                .text_xs()
                .text_color(design::muted())
                .child(if complete { "Complete" } else { "Pending" }),
        )
}

pub(super) fn timeline_row(
    label: &'static str,
    complete: bool,
    cx: &Context<SignerLabApp>,
) -> gpui::Div {
    h_flex()
        .gap_2()
        .py_1()
        .child(
            div()
                .text_color(if complete {
                    design::accent().into()
                } else {
                    cx.theme().muted_foreground
                })
                .child(if complete { "●" } else { "○" }),
        )
        .child(div().text_sm().child(label))
}

pub(super) fn form_section(label: &'static str, content: impl IntoElement) -> gpui::Div {
    v_flex()
        .gap_3()
        .child(div().text_lg().font_semibold().child(label))
        .child(content)
}

pub(super) fn form_field(label: &'static str, content: impl IntoElement) -> gpui::Div {
    v_flex().gap_2().child(field_label(label)).child(content)
}

pub(super) fn form_fields() -> gpui::Div {
    v_flex().gap_4()
}

pub(super) fn form_action_row() -> gpui::Div {
    h_flex().gap_2().items_center().flex_wrap()
}

pub(super) fn form_action_stack() -> gpui::Div {
    v_flex().gap_3().pt_3()
}

pub(super) fn workflow_step(number: &'static str, label: &'static str) -> gpui::Div {
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .size(px(22.0))
                .rounded_full()
                .border_1()
                .border_color(design::muted())
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_semibold()
                .child(number),
        )
        .child(div().font_semibold().child(label))
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, div, point, px, size};

    use super::workspace_surface;

    #[gpui::test]
    fn workspace_content_is_inset_from_every_outer_edge(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(800.0), px(600.0)),
            |_, _| workspace_surface(div(), div(), "Ready"),
        );

        let outer = cx
            .debug_bounds("workspace-surface")
            .expect("workspace surface is rendered");
        let inner = cx
            .debug_bounds("workspace-inner-surface")
            .expect("workspace inner surface is rendered");

        assert_eq!(inner.origin.x, outer.origin.x + px(1.0));
        assert_eq!(inner.origin.y, outer.origin.y + px(1.0));
        assert_eq!(inner.size.width, outer.size.width - px(2.0));
        assert_eq!(inner.size.height, outer.size.height - px(2.0));
    }
}
