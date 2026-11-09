// Bruhs design system color palette as GPUI Hsla values.
// Approximates the OKLCH values from https://design.bruhs.dev/llms.txt

use gpui::{font, hsla, rgb, App, Font, FontFeatures, Hsla};
use gpui_component::{scroll::ScrollbarShow, Theme, ThemeTokens};
use std::sync::Arc;

/// bruhs label/UI face (web: `Monaspace Argon`).
pub const UI_FONT_FAMILY: &str = "Monaspace Argon";
/// Argon instanced 4% wider (`wdth` 104) for the web's `text-label-sm` /
/// `text-button-sm`, whose `letter-spacing: .025em` gpui cannot express:
/// the advance matches Chrome's tracked 12px text exactly.
pub const UI_SMALL_FONT_FAMILY: &str = "Monaspace Argon Tracked";
/// bruhs body/code face (web: `Monaspace Neon`).
pub const CODE_FONT_FAMILY: &str = "Monaspace Neon";
/// bruhs heading face (web: `text-h2` = `Monaspace Xenon`).
pub const HEADING_FONT_FAMILY: &str = "Monaspace Xenon";

/// System UI with OpenType tabular numerals, matching Tailwind `tabular-nums`.
pub fn tabular_system_font() -> Font {
    let mut tabular = font(".SystemUIFont");
    tabular.features = FontFeatures(Arc::new(vec![("tnum".to_string(), 1)]));
    tabular
}

// =====================================================================
// longan — warm brown-black (canvas, panels, cards)
// =====================================================================

/// longan-950 — Canvas background
pub fn longan_950() -> Hsla {
    rgb(0x141210).into()
}

/// longan-900 — Panel background
pub fn longan_900() -> Hsla {
    rgb(0x1d1916).into()
}

/// longan-800 — Card background, borders
pub fn longan_800() -> Hsla {
    rgb(0x2f2922).into()
}

/// longan-700 — Floating elevation, hover surfaces
pub fn longan_700() -> Hsla {
    rgb(0x443a2c).into()
}

/// longan-600 — Strong hover surface
pub fn longan_600() -> Hsla {
    rgb(0x594a36).into()
}

// =====================================================================
// rambutan — warm gray (foreground text, surfaces)
// =====================================================================

/// rambutan-100 — Primary text on dark
pub fn rambutan_100() -> Hsla {
    rgb(0xf5ede7).into()
}

/// rambutan-200 — Strong secondary text
pub fn rambutan_200() -> Hsla {
    rgb(0xede1d8).into()
}

/// rambutan-300 — Body text
pub fn rambutan_300() -> Hsla {
    rgb(0xdfcfc3).into()
}

/// rambutan-400 — Muted text. Darkest informative text that stays above
/// 4.5:1 on longan-950, longan-900, and longan-800.
pub fn rambutan_400() -> Hsla {
    rgb(0xc3afa1).into()
}

/// rambutan-500 — Placeholder and subtle text. Also above 4.5:1 on those surfaces.
pub fn rambutan_500() -> Hsla {
    rgb(0xab998f).into()
}

/// rambutan-600 — Separators and non-text tracks. Clears 4.5:1 on the
/// canvas only, so it is not for body text.
pub fn rambutan_600() -> Hsla {
    rgb(0x8b7c74).into()
}

// =====================================================================
// persimmon — orange (primary actions, focus, attention)
// Reserved for primary buttons + focus rings ONLY
// =====================================================================

/// persimmon-100 — Timegrid event text
pub fn persimmon_100() -> Hsla {
    rgb(0xfbe1cc).into()
}

/// persimmon-200 — Soft active control text
pub fn persimmon_200() -> Hsla {
    rgb(0xf7c194).into()
}

/// persimmon-300 — Today's timegrid header
pub fn persimmon_300() -> Hsla {
    rgb(0xf29e58).into()
}

/// persimmon-400 — Focus ring, soft accents
pub fn persimmon_400() -> Hsla {
    rgb(0xec7f2c).into()
}

/// persimmon-500 — Primary button background
pub fn persimmon_500() -> Hsla {
    rgb(0xdf6815).into()
}

/// persimmon-600 — Active primary button background
pub fn persimmon_600() -> Hsla {
    rgb(0xbd5310).into()
}

// =====================================================================
// pandan — green (success, live indicators)
// =====================================================================

/// pandan-100 — Timegrid event text
pub fn pandan_100() -> Hsla {
    rgb(0xd4f0cd).into()
}

/// pandan-300 — Auth command / subtle success foreground
pub fn pandan_300() -> Hsla {
    rgb(0x7fcc6e).into()
}

/// pandan-400 — Success indicator
pub fn pandan_400() -> Hsla {
    rgb(0x5ab44a).into()
}

/// pandan-500 — Strong success
pub fn pandan_500() -> Hsla {
    rgb(0x3f9931).into()
}

// =====================================================================
// lychee — red (errors, destructive actions)
// =====================================================================

/// lychee-50 — Destructive button text
pub fn lychee_50() -> Hsla {
    rgb(0xfef2f2).into()
}

/// lychee-100 — Timegrid event text
pub fn lychee_100() -> Hsla {
    rgb(0xfde2e1).into()
}

/// lychee-200 — Soft error badge text
pub fn lychee_200() -> Hsla {
    rgb(0xfcc6c4).into()
}

/// lychee-300 — Diff removal / destructive hover foreground
pub fn lychee_300() -> Hsla {
    rgb(0xf8a09c).into()
}

/// lychee-400 — Error indicator
pub fn lychee_400() -> Hsla {
    rgb(0xf06b65).into()
}

/// lychee-500 — Strong error
pub fn lychee_500() -> Hsla {
    rgb(0xdf3f37).into()
}

/// lychee-600 — Destructive button surface
pub fn lychee_600() -> Hsla {
    rgb(0xbf2a23).into()
}

// =====================================================================
// durian — yellow (warnings, attention)
// =====================================================================

/// durian-100 — Strong warning text on dark
pub fn durian_100() -> Hsla {
    rgb(0xfbf0c8).into()
}

/// durian-200 — Muted warning text
pub fn durian_200() -> Hsla {
    rgb(0xf6dd8c).into()
}

/// durian-300 — Warning icon/text
pub fn durian_300() -> Hsla {
    rgb(0xf0c450).into()
}

/// durian-400 — Warning indicator
pub fn durian_400() -> Hsla {
    rgb(0xe9aa1e).into()
}

/// durian-500 — Warning surface
pub fn durian_500() -> Hsla {
    rgb(0xcd8e0d).into()
}

// =====================================================================
// blueberry — blue (informational, links)
// =====================================================================

/// blueberry-200 — Selected text on a blueberry tint
pub fn blueberry_200() -> Hsla {
    rgb(0xbfddff).into()
}

/// blueberry-400 — Informational
pub fn blueberry_400() -> Hsla {
    rgb(0x5b9adf).into()
}

/// blueberry-500 — Timegrid event surface
pub fn blueberry_500() -> Hsla {
    rgb(0x3a7fcb).into()
}

/// blueberry-600 — Selection tint (used at low opacity)
pub fn blueberry_600() -> Hsla {
    rgb(0x327de2).into()
}

// =====================================================================
// mangosteen / dragonfruit — secondary accents
// =====================================================================

/// mangosteen-400 — Purple accent
pub fn mangosteen_400() -> Hsla {
    rgb(0xa87fcc).into()
}

/// mangosteen-500 — Timegrid event surface
pub fn mangosteen_500() -> Hsla {
    rgb(0x8a5fb5).into()
}

/// dragonfruit-400 — Pink accent
pub fn dragonfruit_400() -> Hsla {
    rgb(0xe864a3).into()
}

/// dragonfruit-500 — Timegrid event surface
pub fn dragonfruit_500() -> Hsla {
    rgb(0xd04388).into()
}

// =====================================================================
// White overlays for inset rings (elevation borders)
// =====================================================================

/// white at 5% — Input/select surface
pub fn control_surface() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.05)
}

/// white at 5% — Panel ring
pub fn ring_panel() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.05)
}

/// white at 10% — Card ring
pub fn ring_card() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.10)
}

/// white at 15% — Floating ring
pub fn ring_floating() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.15)
}

/// white at 20% — Modal/dialog ring
pub fn ring_modal() -> Hsla {
    hsla(0.0, 0.0, 1.0, 0.20)
}

// =====================================================================
// gpui-component Theme bridge
// =====================================================================

/// Overwrites the (already-initialized) gpui-component dark theme with bruhs
/// design tokens. Must be called *after* `gpui_component::init(cx)` and
/// `Theme::change(ThemeMode::Dark, ...)`.
pub fn apply_bruhs_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);

    // Surfaces
    // All-mono typography (bruhs non-negotiable): Argon for UI text, Neon
    // for code. The faces are embedded at startup (`assets::embedded_fonts`).
    theme.font_family = UI_FONT_FAMILY.into();
    theme.mono_font_family = CODE_FONT_FAMILY.into();
    theme.background = longan_950();
    theme.foreground = rambutan_200();
    theme.border = ring_card();
    theme.muted = longan_900();
    theme.muted_foreground = rambutan_400();
    theme.accent = longan_800();
    theme.accent_foreground = rambutan_100();
    theme.popover = longan_900();
    theme.popover_foreground = rambutan_100();

    // Primary (persimmon)
    theme.primary = persimmon_500();
    theme.primary_hover = persimmon_400();
    theme.primary_active = persimmon_600();
    theme.primary_foreground = longan_950();

    // Secondary
    theme.secondary = longan_800();
    theme.secondary_hover = longan_700();
    theme.secondary_active = longan_800();
    theme.secondary_foreground = rambutan_100();

    // Danger (lychee)
    theme.danger = lychee_600();
    theme.danger_hover = lychee_500();
    theme.danger_active = lychee_600();
    theme.danger_foreground = lychee_50();

    // Warning (durian)
    theme.warning = durian_400();
    theme.warning_hover = durian_400();
    theme.warning_active = durian_400();
    theme.warning_foreground = longan_950();

    // Success (pandan)
    theme.success = pandan_500();
    theme.success_hover = pandan_400();
    theme.success_active = pandan_500();
    theme.success_foreground = longan_950();

    // Info (blueberry)
    theme.info = blueberry_400();
    theme.info_hover = blueberry_400();
    theme.info_active = blueberry_400();
    theme.info_foreground = longan_950();

    // Focus ring + caret
    theme.ring = persimmon_400();
    theme.caret = persimmon_400();
    theme.selection = persimmon_500();

    // Sidebar
    theme.sidebar = longan_900();
    theme.sidebar_foreground = rambutan_300();
    theme.sidebar_border = ring_panel();
    theme.sidebar_accent = longan_800();
    theme.sidebar_accent_foreground = rambutan_100();
    theme.sidebar_primary = persimmon_500();
    theme.sidebar_primary_foreground = longan_950();

    // Title bar
    theme.title_bar = longan_900();
    theme.title_bar_border = ring_panel();

    // Inputs
    theme.input = ring_card();

    // Lists
    theme.list_hover = longan_800();
    theme.list_active = longan_800();
    theme.list_active_border = ring_card();
    theme.list_head = longan_900();
    theme.list_even = longan_900();

    // Tables
    theme.table = longan_900();
    theme.table_hover = longan_800();
    theme.table_active = longan_800();
    theme.table_active_border = ring_card();
    theme.table_head = longan_900();
    theme.table_head_foreground = rambutan_400();
    theme.table_row_border = ring_panel();
    theme.table_even = longan_900();

    // Tabs
    theme.tab = longan_900();
    theme.tab_active = longan_800();
    theme.tab_active_foreground = rambutan_100();
    theme.tab_foreground = rambutan_400();
    theme.tab_bar = longan_900();
    theme.tab_bar_segmented = longan_800();

    // Misc
    theme.skeleton = longan_800();
    theme.scrollbar_show = ScrollbarShow::Always;
    theme.scrollbar = hsla(0.0, 0.0, 0.0, 0.0);
    theme.scrollbar_thumb = rambutan_400().opacity(0.32);
    theme.scrollbar_thumb_hover = rambutan_400().opacity(0.55);
    theme.overlay = hsla(0.0, 0.0, 0.0, 0.5);
    theme.drop_target = persimmon_400();
    theme.drag_border = persimmon_400();

    // Base palette aliases (used by chart helpers etc.)
    theme.red = lychee_500();
    theme.red_light = lychee_400();
    theme.green = pandan_500();
    theme.green_light = pandan_400();
    theme.yellow = durian_400();
    theme.yellow_light = durian_400();
    theme.blue = blueberry_400();
    theme.blue_light = blueberry_400();

    // `Theme::tokens` is derived from the colors when the theme is built, so
    // it still holds gpui-component's defaults after the assignments above.
    // Widgets that paint from `tokens` (Checkbox's checked fill, for one) look
    // unthemed until it is re-derived.
    theme.tokens = ThemeTokens::from(&theme.colors);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Theme::tokens` is derived when the theme is built, so the bruhs
    /// colours have to be pushed into it after they are assigned — otherwise
    /// widgets that paint from tokens (gpui-component's `Checkbox` fills its
    /// box with `tokens.primary`) silently keep the library's defaults.
    #[gpui::test]
    fn theme_tokens_follow_the_bruhs_palette(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            apply_bruhs_theme(cx);
            let theme = gpui_component::Theme::global(cx);
            assert_eq!(theme.primary, persimmon_500(), "colors.primary");
            assert_eq!(
                theme.tokens.primary.color,
                persimmon_500(),
                "tokens.primary must track colors.primary"
            );
            assert_eq!(theme.tokens.primary_foreground.color, longan_950());
        });
    }

    #[test]
    fn modal_ring_should_match_web_dialog_reference() {
        assert_eq!(ring_modal(), hsla(0.0, 0.0, 1.0, 0.20));
    }

    #[test]
    fn control_surface_should_match_web_input_reference() {
        assert_eq!(control_surface(), hsla(0.0, 0.0, 1.0, 0.05));
    }

    #[test]
    fn strong_hover_surface_should_match_web_longan_600_reference() {
        assert_eq!(longan_600(), rgb(0x594a36).into());
    }

    #[test]
    fn base_palette_tokens_should_match_web_tailwind_reference() {
        assert_eq!(longan_950(), rgb(0x141210).into());
        assert_eq!(longan_900(), rgb(0x1d1916).into());
        assert_eq!(longan_800(), rgb(0x2f2922).into());
        assert_eq!(longan_700(), rgb(0x443a2c).into());
        assert_eq!(rambutan_100(), rgb(0xf5ede7).into());
        assert_eq!(rambutan_200(), rgb(0xede1d8).into());
        assert_eq!(rambutan_300(), rgb(0xdfcfc3).into());
        assert_eq!(rambutan_400(), rgb(0xc3afa1).into());
        assert_eq!(rambutan_500(), rgb(0xab998f).into());
        assert_eq!(rambutan_600(), rgb(0x8b7c74).into());
    }

    #[test]
    fn warning_tokens_should_match_web_durian_reference() {
        assert_eq!(durian_100(), rgb(0xfbf0c8).into());
        assert_eq!(durian_200(), rgb(0xf6dd8c).into());
        assert_eq!(durian_300(), rgb(0xf0c450).into());
        assert_eq!(durian_400(), rgb(0xe9aa1e).into());
        assert_eq!(durian_500(), rgb(0xcd8e0d).into());
    }

    #[test]
    fn destructive_tokens_should_match_web_button_reference() {
        assert_eq!(lychee_100(), rgb(0xfde2e1).into());
        assert_eq!(lychee_200(), rgb(0xfcc6c4).into());
        assert_eq!(lychee_300(), rgb(0xf8a09c).into());
        assert_eq!(lychee_400(), rgb(0xf06b65).into());
        assert_eq!(lychee_500(), rgb(0xdf3f37).into());
        assert_eq!(lychee_50(), rgb(0xfef2f2).into());
        assert_eq!(lychee_600(), rgb(0xbf2a23).into());
    }

    #[test]
    fn accent_tokens_should_match_web_tailwind_reference() {
        assert_eq!(pandan_100(), rgb(0xd4f0cd).into());
        assert_eq!(pandan_300(), rgb(0x7fcc6e).into());
        assert_eq!(pandan_400(), rgb(0x5ab44a).into());
        assert_eq!(pandan_500(), rgb(0x3f9931).into());
        assert_eq!(blueberry_400(), rgb(0x5b9adf).into());
        assert_eq!(blueberry_500(), rgb(0x3a7fcb).into());
        assert_eq!(mangosteen_400(), rgb(0xa87fcc).into());
        assert_eq!(mangosteen_500(), rgb(0x8a5fb5).into());
        assert_eq!(dragonfruit_400(), rgb(0xe864a3).into());
        assert_eq!(dragonfruit_500(), rgb(0xd04388).into());
    }

    #[test]
    fn primary_tokens_should_match_web_button_reference() {
        assert_eq!(persimmon_100(), rgb(0xfbe1cc).into());
        assert_eq!(persimmon_300(), rgb(0xf29e58).into());
        assert_eq!(persimmon_400(), rgb(0xec7f2c).into());
        assert_eq!(persimmon_500(), rgb(0xdf6815).into());
        assert_eq!(persimmon_600(), rgb(0xbd5310).into());
    }

    #[test]
    fn tabular_system_font_should_match_web_tabular_nums_reference() {
        let tabular = tabular_system_font();

        assert_eq!(tabular.family.to_string(), ".SystemUIFont");
        assert_eq!(
            tabular.features.tag_value_list(),
            [("tnum".to_string(), 1)].as_slice()
        );
    }
}
