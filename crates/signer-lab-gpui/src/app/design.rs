use gpui_kit::component::Theme;
use gpui_kit::{Hsla, px, rgb};
use signer_lab_core::LabNetwork;

pub(super) const SIDEBAR_WIDTH: f32 = 232.0;
pub(super) const TOOLBAR_HEIGHT: f32 = 52.0;
pub(super) const STATUS_BAR_HEIGHT: f32 = 26.0;
pub(super) const INSPECTOR_WIDTH: f32 = 340.0;
pub(super) const CONTENT_PADDING: f32 = 24.0;

/// QR codes need a white surface in both appearance modes
pub(super) const QR_BACKGROUND: Hsla = Hsla::white();

/// Distinguishes the two test networks without changing semantic theme colors
pub(super) fn network_color(network: LabNetwork) -> Hsla {
    rgb(match network {
        LabNetwork::MutinyNet => 0x008b_5cf6,
        LabNetwork::Testnet4 => 0x0014_b8a6,
    })
    .into()
}

pub(crate) fn apply_brand(theme: &mut Theme) {
    theme.primary = rgb(0x00f7_931a).into();
    theme.primary_hover = rgb(0x00e8_850d).into();
    theme.primary_active = rgb(0x00d9_7700).into();
    theme.primary_foreground = Hsla::white();
    theme.ring = theme.primary;
    theme.button_primary = theme.primary;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.button_primary_foreground = theme.primary_foreground;
    theme.list_active = Hsla {
        a: 0.12,
        ..theme.primary
    };

    theme.table_active = theme.list_active;
    theme.list_active_border = theme.primary;
    theme.table_active_border = theme.primary;
    theme.sidebar.a = 0.8;
    theme.radius = px(6.0);
    theme.radius_lg = px(8.0);
    theme.font_size = px(13.0);
    theme.mono_font_size = px(12.0);
}

#[cfg(test)]
mod tests {
    use super::apply_brand;
    use gpui_kit::{
        TestAppContext, base,
        component::{Theme, ThemeMode},
    };

    #[gpui_kit::test]
    fn brand_reaches_render_tokens_in_both_modes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(mode, None, cx);
                let background = Theme::global(cx).background;
                Theme::update(cx, apply_brand);
                let theme = Theme::global(cx);
                assert_eq!(theme.background, background);
                assert_eq!(theme.tokens.button_primary.color, theme.primary);
                assert_eq!(base::Theme::global(cx).tokens.colors.primary, theme.primary);
                assert!((theme.sidebar.a - 0.8).abs() < f32::EPSILON);
            }
        });
    }
}
