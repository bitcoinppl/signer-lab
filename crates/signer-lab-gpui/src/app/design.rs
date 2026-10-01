use gpui::{Rgba, px, rgb, rgba};
use gpui_component::Theme;

pub(super) const SIDEBAR_WIDTH: f32 = 264.0;
pub(super) const INSPECTOR_WIDTH: f32 = 360.0;
pub(super) const WORKSPACE_HEADER_HEIGHT: f32 = 88.0;
pub(super) const SURFACE_RADIUS: f32 = 14.0;
pub(super) const SURFACE_INNER_RADIUS: f32 = SURFACE_RADIUS - 1.0;
pub(super) const CONTENT_RADIUS: f32 = 10.0;

const ACCENT_HEX: u32 = 0x00f7_931a;
const ACCENT_HOVER_HEX: u32 = 0x00e8_850d;
const ACCENT_ACTIVE_HEX: u32 = 0x00d9_7700;
const BORDER_HEX: u32 = 0xd5df_e7ff;
const CANVAS_HEX: u32 = 0xf9fb_fcff;
const INSPECTOR_HEX: u32 = 0xf2f6_f9ff;
const MUTED_HEX: u32 = 0x6672_7dff;
const SIDEBAR_HEX: u32 = 0xeff5_faff;
const SIDEBAR_BORDER_HEX: u32 = 0xd3df_e8ff;
const SIDEBAR_HOVER_HEX: u32 = 0xe5ed_f3ff;
const SIDEBAR_MUTED_HEX: u32 = 0x6b78_84ff;
const SIDEBAR_SELECTED_HEX: u32 = 0xffff_ffff;
const SIDEBAR_TEXT_HEX: u32 = 0x2b34_3cff;
const STATUS_GREEN_HEX: u32 = 0x22c5_5eff;
const TEXT_HEX: u32 = 0x2027_2eff;
const TINT_HEX: u32 = 0xfff3_e6ff;
const TOOLBAR_HEX: u32 = 0xf7fa_fcff;
const WINDOW_FIELD_HEX: u32 = 0xe7f0_f6ff;

pub(super) fn accent() -> Rgba {
    rgb(ACCENT_HEX)
}

pub(super) fn border() -> Rgba {
    rgba(BORDER_HEX)
}

pub(super) fn canvas() -> Rgba {
    rgba(CANVAS_HEX)
}

pub(super) fn inspector() -> Rgba {
    rgba(INSPECTOR_HEX)
}

pub(super) fn muted() -> Rgba {
    rgba(MUTED_HEX)
}

pub(super) fn sidebar() -> Rgba {
    rgba(SIDEBAR_HEX)
}

pub(super) fn sidebar_border() -> Rgba {
    rgba(SIDEBAR_BORDER_HEX)
}

pub(super) fn sidebar_hover() -> Rgba {
    rgba(SIDEBAR_HOVER_HEX)
}

pub(super) fn sidebar_muted() -> Rgba {
    rgba(SIDEBAR_MUTED_HEX)
}

pub(super) fn sidebar_selected() -> Rgba {
    rgba(SIDEBAR_SELECTED_HEX)
}

pub(super) fn sidebar_text() -> Rgba {
    rgba(SIDEBAR_TEXT_HEX)
}

pub(super) fn status_green() -> Rgba {
    rgba(STATUS_GREEN_HEX)
}

pub(super) fn text() -> Rgba {
    rgba(TEXT_HEX)
}

pub(super) fn tint() -> Rgba {
    rgba(TINT_HEX)
}

pub(super) fn toolbar() -> Rgba {
    rgba(TOOLBAR_HEX)
}

pub(super) fn window_field() -> Rgba {
    rgba(WINDOW_FIELD_HEX)
}

pub(crate) fn apply_theme(theme: &mut Theme) {
    theme.primary = rgb(ACCENT_HEX).into();
    theme.primary_hover = rgb(ACCENT_HOVER_HEX).into();
    theme.primary_active = rgb(ACCENT_ACTIVE_HEX).into();
    theme.primary_foreground = rgb(0x00ff_ffff).into();
    theme.background = canvas().into();
    theme.foreground = text().into();
    theme.muted_foreground = muted().into();
    theme.border = border().into();
    theme.input = rgba(0xb5bd_c666).into();
    theme.muted = rgba(0xe9ed_f0d9).into();
    theme.secondary = rgba(0xffff_fff0).into();
    theme.secondary_hover = rgb(0x00ff_ffff).into();
    theme.accent = tint().into();
    theme.radius = px(7.0);
    theme.radius_lg = px(9.0);
    theme.shadow = true;
}
