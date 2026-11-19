use super::{
    SignerLabApp, design,
    state::{QrAnimation, QrPlayback, QrPreviewSize},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    empty::{Empty, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle},
    group_box::{GroupBox, GroupBoxVariants as _},
    h_flex,
    input::{Textarea, TextareaState},
    v_flex,
};
use gpui_kit::{
    App, ClipboardItem, Context, Div, ElementId, Entity, FontFeatures, Hsla, Image,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _, px,
};
use std::sync::Arc;

/// Groups decimal digits so wallet amounts remain easy to read
pub(super) fn format_sats(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }

        result.push(digit);
    }
    result
}

/// Makes column sizing explicit in headers and rows
#[derive(Clone, Copy)]
pub(super) enum ColumnWidth {
    Fixed(Pixels),
    Flex,
}

/// Keeps column alignment consistent between headers and rows
#[derive(Clone, Copy)]
pub(super) enum Align {
    Left,
    Right,
}

/// Gives table headings one shared spacing and contrast policy
pub(super) fn table_header(
    columns: &[(&'static str, ColumnWidth, Align)],
    cx: &Context<SignerLabApp>,
) -> Div {
    h_flex()
        .h(px(28.0))
        .flex_shrink_0()
        .px_4()
        .bg(cx.theme().table_head)
        // the default table heading token has insufficient contrast in dark mode
        .text_color(cx.theme().muted_foreground)
        .text_size(px(11.0))
        .font_semibold()
        .children(
            columns
                .iter()
                .map(|(label, width, align)| table_cell(*width, *align).child(*label)),
        )
}

/// Keeps fixed columns from shrinking and numeric values aligned
pub(super) fn table_cell(width: ColumnWidth, align: Align) -> Div {
    let cell = match width {
        ColumnWidth::Fixed(width) => tabular("").w(width).flex_shrink_0(),
        ColumnWidth::Flex => div().flex_1().min_w_0(),
    };
    match align {
        Align::Left => cell,
        Align::Right => cell.text_right(),
    }
}

/// Uses equal digit widths for amounts and counts
pub(super) fn tabular(text: impl Into<SharedString>) -> Div {
    let text = text.into();
    div()
        .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
        .when(!text.is_empty(), |cell| cell.child(text))
}

/// Keeps amount columns aligned across values of different lengths
pub(super) fn amount_sats(value: u64) -> Div {
    tabular(format!("{} sats", format_sats(value)))
}

/// Abbreviates by characters rather than bytes so copied Unicode values stay valid
pub(super) fn abbreviate(value: &str, first: usize, last: usize) -> String {
    let chars: Vec<_> = value.chars().collect();
    if chars.len() <= first.saturating_add(last).saturating_add(1) {
        return value.to_owned();
    }
    chars[..first]
        .iter()
        .chain(std::iter::once(&'…'))
        .chain(chars[chars.len() - last..].iter())
        .collect()
}

/// Copy always uses the full value even when its display is shortened
#[derive(IntoElement)]
pub(super) struct MonoValue {
    id: ElementId,
    value: SharedString,
    truncated: bool,
}

impl MonoValue {
    pub(super) fn truncated(mut self) -> Self {
        self.truncated = true;
        self
    }
}

impl RenderOnce for MonoValue {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let displayed = if self.truncated {
            abbreviate(&self.value, 10, 8)
        } else {
            self.value.to_string()
        };

        let value = self.value;
        h_flex()
            .min_w_0()
            .gap_1()
            .items_start()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(cx.theme().mono_font_family.clone())
                    .whitespace_normal()
                    .child(displayed),
            )
            .child(
                Button::new(self.id)
                    .ghost()
                    .small()
                    .icon(IconName::Clipboard)
                    .tooltip("Copy")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
                    }),
            )
    }
}

/// Public wallet values share one copy behavior and one monospace treatment
pub(super) fn mono_value(
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    _: &Context<SignerLabApp>,
) -> MonoValue {
    MonoValue {
        id: id.into(),
        value: value.into(),
        truncated: false,
    }
}

/// Captions separate small sections without extra surfaces
pub(super) fn caption(text: impl Into<SharedString>, cx: &Context<SignerLabApp>) -> Div {
    div()
        .text_size(px(11.0))
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// Empty workflows keep their available action beside the missing state
pub(super) fn empty_state(icon: IconName, title: &'static str, action: Option<Button>) -> Empty {
    Empty::new()
        .header(
            EmptyHeader::new()
                .media(
                    EmptyMedia::new()
                        .with_variant(EmptyMediaVariant::Icon)
                        .child(Icon::new(icon)),
                )
                .title(EmptyTitle::new().child(title)),
        )
        .children(action)
}

/// The fixed stage prevents QR density from changing the surrounding layout
pub(super) fn qr_stage(image: Arc<Image>, side: u16, cx: &Context<SignerLabApp>) -> Div {
    qr_stage_surface(
        img(image)
            .size(px(f32::from(side)))
            .object_fit(gpui_kit::ObjectFit::Contain),
        side,
        cx.theme().border,
    )
}

fn qr_stage_surface(content: impl IntoElement, side: u16, border: gpui_kit::Hsla) -> Div {
    div()
        .debug_selector(|| "qr-preview-stage".into())
        .size(px(f32::from(side) + 34.0))
        .flex_shrink_0()
        .p(px(16.0))
        .border_1()
        .border_color(border)
        .rounded(px(8.0))
        .bg(design::QR_BACKGROUND)
        .child(
            div()
                .debug_selector(|| "qr-image-surface".into())
                .size(px(f32::from(side)))
                .overflow_hidden()
                .child(content),
        )
}

/// QR transport controls stay with the frame they control
pub(super) fn qr_output(
    playback: &QrPlayback,
    size: QrPreviewSize,
    cx: &Context<SignerLabApp>,
) -> Div {
    let stage = match playback.image(size.side()) {
        Ok(image) => qr_stage(image, size.side(), cx),
        Err(error) => div().size(px(f32::from(size.side()) + 34.0)).child(error),
    };

    let mut controls = h_flex()
        .gap_1()
        .items_center()
        .justify_center()
        .child(
            Button::new("qr-smaller")
                .ghost()
                .small()
                .icon(IconName::Minus)
                .tooltip("Smaller")
                .on_click(cx.listener(|app, _, _, cx| app.shrink_qr_preview(cx))),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} px", size.side())),
        )
        .child(
            Button::new("qr-larger")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .tooltip("Larger")
                .on_click(cx.listener(|app, _, _, cx| app.grow_qr_preview(cx))),
        );
    if playback.frame_count() > 1 {
        let playing = playback.animation() == Some(QrAnimation::Playing);
        controls = controls
            .child(
                Button::new("qr-play")
                    .ghost()
                    .small()
                    .icon(if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    })
                    .tooltip(if playing { "Pause" } else { "Play" })
                    .on_click(cx.listener(|app, _, _, cx| app.toggle_qr_animation(cx))),
            )
            .child(
                Button::new("qr-next")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronRight)
                    .tooltip("Next frame")
                    .on_click(cx.listener(|app, _, _, cx| app.next_qr_frame(cx))),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} / {}",
                        playback.frame_index() + 1,
                        playback.frame_count()
                    )),
            );
    }
    v_flex().gap_2().items_center().child(stage).child(controls)
}

/// Generated text remains selectable but cannot diverge from its saved payload
pub(super) fn payload_panel(
    title: &'static str,
    input: &Entity<TextareaState>,
    actions: impl IntoElement,
    cx: &Context<SignerLabApp>,
) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .min_h(px(120.0))
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(caption(title, cx))
                .child(actions),
        )
        .child(
            Textarea::new(input)
                .readonly(true)
                .font_family(cx.theme().mono_font_family.clone())
                .flex_1()
                .min_h(px(120.0)),
        )
}

#[cfg(test)]
mod tests {
    use super::{abbreviate, format_sats, qr_stage_surface};
    use gpui_kit::{Styled as _, TestAppContext, div, point, px, size};
    #[test]
    fn amounts_group_without_overflow() {
        assert_eq!(format_sats(0), "0");
        assert_eq!(format_sats(999), "999");
        assert_eq!(format_sats(1000), "1,000");
        assert_eq!(format_sats(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn abbreviation_keeps_short_and_unicode_values() {
        assert_eq!(abbreviate("abc", 10, 8), "abc");
        assert_eq!(
            abbreviate("1234567890abcdefghij", 10, 8),
            "1234567890…cdefghij"
        );

        assert_eq!(abbreviate("αβγδεζηθ", 2, 2), "αβ…ηθ");
        assert_eq!(abbreviate("long", 0, 0), "…");
    }

    #[gpui_kit::test]
    fn qr_stage_bounds_do_not_depend_on_density(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        for density in [40.0, 500.0] {
            cx.draw(
                point(px(0.0), px(0.0)),
                size(px(400.0), px(400.0)),
                |_, _| qr_stage_surface(div().size(px(density)), 260, gpui_kit::Hsla::black()),
            );

            assert_eq!(
                cx.debug_bounds("qr-preview-stage").expect("stage").size,
                size(px(294.0), px(294.0))
            );

            assert_eq!(
                cx.debug_bounds("qr-image-surface").expect("image").size,
                size(px(260.0), px(260.0))
            );
        }
    }
}

/// A typed choice with an optional network indicator
pub(super) struct Segment<T> {
    pub(super) value: T,
    pub(super) label: SharedString,
    pub(super) dot: Option<Hsla>,
}

/// Keeps each choice inside a visible shared control surface
#[derive(IntoElement)]
pub(super) struct Segmented {
    container: Stateful<Div>,
    segments: Vec<Stateful<Div>>,
}

impl Segmented {
    /// Gives each choice the same share of the available width
    pub(super) fn full_width(mut self) -> Self {
        self.container = self.container.w_full();
        self.segments = self
            .segments
            .into_iter()
            .map(gpui_kit::Styled::flex_1)
            .collect();
        self
    }
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.container.children(self.segments)
    }
}

/// Uses typed values so callers do not map click indices back into domain state
pub(super) fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<ElementId>,
    segments: impl IntoIterator<Item = Segment<T>>,
    selected: T,
    disabled: bool,
    on_select: impl Fn(&mut SignerLabApp, T, &mut Window, &mut Context<SignerLabApp>) + 'static,
    cx: &Context<SignerLabApp>,
) -> Segmented {
    let on_select = Arc::new(on_select);
    let theme = cx.theme();
    let segments = segments
        .into_iter()
        .enumerate()
        .map(|(index, segment)| {
            let value = segment.value;
            let active = value == selected;
            let on_select = Arc::clone(&on_select);
            h_flex()
                .id(index)
                .h(px(24.0))
                .px(px(10.0))
                .gap(px(4.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_size(px(12.0))
                .font_medium()
                .text_color(theme.muted_foreground)
                .when(active, |this| {
                    this.bg(theme.background)
                        .text_color(theme.foreground)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_xs()
                })
                .when(!disabled, |this| {
                    this.cursor_pointer()
                        .hover(|this| this.text_color(theme.foreground))
                        .on_click(cx.listener(move |app, _, window, cx| {
                            on_select(app, value, window, cx);
                        }))
                })
                .children(
                    segment
                        .dot
                        .map(|color| div().size(px(7.0)).rounded_full().bg(color)),
                )
                .child(segment.label)
        })
        .collect();
    Segmented {
        container: h_flex()
            .id(id)
            .p(px(2.0))
            .gap(px(2.0))
            .rounded(px(7.0))
            .bg(theme.muted)
            .border_1()
            .border_color(theme.border)
            .when(disabled, |this| this.opacity(0.5)),
        segments,
    }
}

/// Gives all outlined groups the same compact title and spacing
pub(super) fn group_box(title: &'static str) -> GroupBox {
    let title_style = div().text_size(px(12.0)).font_semibold().style().clone();
    GroupBox::new()
        .outline()
        .title(title)
        .title_style(title_style)
        .gap(px(6.0))
}
