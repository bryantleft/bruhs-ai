// Bruhs design system color palette as GPUI Hsla values.
// Approximates the OKLCH values from https://design.bruhs.dev/llms.txt

use gpui::{hsla, px, rgb, App, FontFeatures, Global, Hsla, Pixels, SharedString, Window};
use std::collections::BTreeMap;
use std::sync::Arc;
mod fonts;
mod palettes;
pub use fonts::{set_font_families, set_text_layout, show_composer_context, wrap_code, FontFace};
pub use palettes::{Palette, ThemeFamily};

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

// Shared size scale (docs/ui-standards.md), from shadcn's dense Sidebar and
// menu sizes. Web defines the same values as CSS variables in
// bruhs-web/tailwind.css. Call sites use these names, never raw numbers, so an
// interface density setting can change them in one place.
/// Default control: buttons, inputs in bars and menus, menu and list rows
/// (shadcn `h-8`).
pub const CONTROL_PX: f32 = 32.0;
/// Compact control: icon toggles in bars and headers (shadcn `size-7`).
pub const CONTROL_SM_PX: f32 = 28.0;
/// Actions inside rows and cards.
pub const CONTROL_XS_PX: f32 = 24.0;
/// A control row with the setting name above its current value.
pub const CONTROL_DETAIL_PX: f32 = 40.0;
/// Inside padding for standard text and select fields.
pub const FIELD_PADDING_X_PX: f32 = 10.0;
/// Every icon inside a control (shadcn `size-4`).
pub const ICON_PX: f32 = 16.0;
/// Icons that decorate 13 to 14px text that is not clickable.
pub const ICON_INLINE_PX: f32 = 14.0;
/// Icons that decorate 12px text that is not clickable.
pub const ICON_INLINE_SM_PX: f32 = 12.0;
/// Icons beside 12px text inside a chip or button, such as the composer's
/// model and branch chips (shadcn `xs` Button: `size-3`).
pub const ICON_XS_PX: f32 = 12.0;

pub const APP_UI_TEXT_SIZE_PX: f32 = 13.0;
pub const APP_BODY_TEXT_SIZE_PX: f32 = 13.0;
pub const APP_INPUT_TEXT_SIZE_PX: f32 = 14.0;
pub const APP_HEADER_HEIGHT_PX: f32 = 44.0;
pub const APP_CONTROL_HEIGHT_PX: f32 = CONTROL_PX;
pub const APP_COMPACT_CONTROL_HEIGHT_PX: f32 = CONTROL_SM_PX;

/// OpenType tabular numerals, like Tailwind `tabular-nums`: only the feature
/// changes, so the text keeps its Monaspace face (bruhs is all mono).
pub fn tabular_nums() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".to_string(), 1)]))
}

// =====================================================================
// longan — warm brown-black (canvas, panels, cards)
// =====================================================================

thread_local! {
    // Native windows draw on the UI thread, as with the shared page zoom.
    static PALETTE_MODE: std::cell::Cell<ThemeMode> = const { std::cell::Cell::new(ThemeMode::Dark) };
    static ACTIVE_PALETTE: std::cell::Cell<Palette> = const { std::cell::Cell::new(*ThemeFamily::Bruhs.palette(ThemeMode::Dark)) };
    static GLASS_OPACITY: std::cell::Cell<u8> = const { std::cell::Cell::new(100) };
    static CONTRAST: std::cell::Cell<u8> = const { std::cell::Cell::new(100) };
}

fn palette_color(dark: u32, light: u32) -> Hsla {
    let color = match PALETTE_MODE.get() {
        ThemeMode::Dark => dark,
        ThemeMode::Light => light,
    };
    toned_color(color)
}

fn toned_color(color: u32) -> Hsla {
    let mut color: Hsla = rgb(color).into();
    let contrast = CONTRAST.get();
    if contrast != 100 {
        let exponent = f32::from(contrast) / 100.;
        color.l = if color.l < 0.5 {
            0.5 * (2. * color.l).powf(exponent)
        } else {
            1. - 0.5 * (2. * (1. - color.l)).powf(exponent)
        };
    }
    color
}

fn surface_ring(alpha: f32) -> Hsla {
    hsla(
        0.,
        0.,
        if PALETTE_MODE.get() == ThemeMode::Dark {
            1.
        } else {
            0.
        },
        alpha,
    )
}

/// Apply a device contrast setting without changing its color scheme or zoom.
pub fn set_contrast(percent: u8, cx: &mut App) {
    if CONTRAST.replace(percent.clamp(50, 200)) != percent.clamp(50, 200) {
        apply_bruhs_theme(cx);
        cx.refresh_windows();
    }
}

/// Tint floating surfaces while keeping labels, controls and focus rings opaque.
pub fn glass_surface(surface: Hsla) -> Hsla {
    surface.opacity(f32::from(GLASS_OPACITY.get()) / 100.)
}

pub fn set_glass_opacity(percent: u8, cx: &mut App) {
    let percent = percent.clamp(40, 100);
    if GLASS_OPACITY.replace(percent) != percent {
        cx.refresh_windows();
    }
}

/// longan-950 — Canvas background
pub fn longan_950() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().surface[0])
}

/// longan-900 — Panel background
pub fn longan_900() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().surface[1])
}

/// longan-800 — Card background, borders
pub fn longan_800() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().surface[2])
}

/// longan-700 — Floating elevation, hover surfaces
pub fn longan_700() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().surface[3])
}

/// longan-600 — Strong hover surface
pub fn longan_600() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().surface[4])
}

// =====================================================================
// rambutan — warm gray (foreground text, surfaces)
// =====================================================================

/// rambutan-100 — Primary text on dark
pub fn rambutan_100() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().text[0])
}

/// rambutan-200 — Strong secondary text
pub fn rambutan_200() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().text[1])
}

/// rambutan-300 — Body text
pub fn rambutan_300() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().text[2])
}

/// rambutan-400 — Muted text. Darkest informative text that stays above
/// 4.5:1 on longan-950, longan-900, and longan-800.
pub fn rambutan_400() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().text[3])
}

/// rambutan-500 — Placeholder and subtle text. Also above 4.5:1 on those surfaces.
pub fn rambutan_500() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().text[4])
}

/// rambutan-600 — Separators and non-text tracks. Clears 4.5:1 on the
/// canvas only, so it is not for body text.
pub fn rambutan_600() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().text[5])
}

// =====================================================================
// persimmon — orange (primary actions, focus, attention)
// Reserved for primary buttons + focus rings ONLY
// =====================================================================

/// persimmon-100 — Timegrid event text
pub fn persimmon_100() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().accent[0])
}

/// persimmon-200 — Soft active control text
pub fn persimmon_200() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().accent[1])
}

/// persimmon-300 — Today's timegrid header
pub fn persimmon_300() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().accent[2])
}

/// persimmon-400 — Focus ring, soft accents
pub fn persimmon_400() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().accent[3])
}

/// persimmon-500 — Primary button background
pub fn persimmon_500() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().accent[4])
}

/// persimmon-600 — Active primary button background
pub fn persimmon_600() -> Hsla {
    toned_color(ACTIVE_PALETTE.get().accent[5])
}

// =====================================================================
// pandan — green (success, live indicators)
// =====================================================================

/// pandan-100 — Timegrid event text
pub fn pandan_100() -> Hsla {
    palette_color(0xd4f0cd, 0x143713)
}

/// pandan-200 — Today's "Day plan applied." (bruhs `oklch(91.9% .072 138.94)`)
pub fn pandan_200() -> Hsla {
    palette_color(0xccf1c3, 0x214c20)
}

/// pandan-300 — Auth command / subtle success foreground
pub fn pandan_300() -> Hsla {
    palette_color(0x7fcc6e, 0x245521)
}

/// pandan-400 — Success indicator
pub fn pandan_400() -> Hsla {
    palette_color(0x5ab44a, 0x286924)
}

/// pandan-500 — Strong success
pub fn pandan_500() -> Hsla {
    palette_color(0x3f9931, 0x287124)
}

// =====================================================================
// lychee — red (errors, destructive actions)
// =====================================================================

/// Foreground for saturated destructive controls in either color scheme.
pub fn lychee_50() -> Hsla {
    toned_color(0xfef2f2)
}

/// lychee-100 — Timegrid event text
pub fn lychee_100() -> Hsla {
    palette_color(0xfde2e1, 0x5c1b1d)
}

/// lychee-200 — Soft error badge text
pub fn lychee_200() -> Hsla {
    palette_color(0xfcc6c4, 0x742426)
}

/// lychee-300 — Diff removal / destructive hover foreground
pub fn lychee_300() -> Hsla {
    palette_color(0xf8a09c, 0x8f2930)
}

/// lychee-400 — Error indicator
pub fn lychee_400() -> Hsla {
    palette_color(0xf06b65, 0xa03138)
}

/// lychee-500 — Strong error
pub fn lychee_500() -> Hsla {
    palette_color(0xdf3f37, 0xa92e37)
}

/// lychee-600 — Destructive button surface
pub fn lychee_600() -> Hsla {
    palette_color(0xbf2a23, 0x86272c)
}

/// lychee-700 — Outlined danger button ring
pub fn lychee_700() -> Hsla {
    rgb(0x9a221c).into()
}

// =====================================================================
// durian — yellow (warnings, attention)
// =====================================================================

/// durian-100 — Strong warning text on dark
pub fn durian_100() -> Hsla {
    palette_color(0xfbf0c8, 0x4a2f04)
}

/// durian-200 — Muted warning text
pub fn durian_200() -> Hsla {
    palette_color(0xf6dd8c, 0x5c3c04)
}

/// durian-300 — Warning icon/text
pub fn durian_300() -> Hsla {
    palette_color(0xf0c450, 0x705009)
}

/// durian-400 — Warning indicator
pub fn durian_400() -> Hsla {
    palette_color(0xe9aa1e, 0x7f5707)
}

/// durian-500 — Warning surface
pub fn durian_500() -> Hsla {
    palette_color(0xcd8e0d, 0x89600a)
}

// =====================================================================
// blueberry — blue (informational, links)
// =====================================================================

/// blueberry-200 — Selected text on a blueberry tint
pub fn blueberry_200() -> Hsla {
    palette_color(0xbfddff, 0x20385d)
}

/// blueberry-300 — Links and booking times (bruhs `oklch(83.6% .098 257.89)`)
pub fn blueberry_300() -> Hsla {
    palette_color(0xa1cbff, 0x214d7c)
}

/// blueberry-400 — Informational
pub fn blueberry_400() -> Hsla {
    palette_color(0x5b9adf, 0x225b96)
}

/// blueberry-500 — Timegrid event surface
pub fn blueberry_500() -> Hsla {
    palette_color(0x3a7fcb, 0x255b9d)
}

/// blueberry-600 — Selection tint (used at low opacity)
pub fn blueberry_600() -> Hsla {
    palette_color(0x327de2, 0x254e7e)
}

// =====================================================================
// mangosteen / dragonfruit — secondary accents
// =====================================================================

/// mangosteen-400 — Purple accent
pub fn mangosteen_400() -> Hsla {
    palette_color(0xa87fcc, 0x724c92)
}

/// mangosteen-500 — Timegrid event surface
pub fn mangosteen_500() -> Hsla {
    palette_color(0x8a5fb5, 0x753d9d)
}

/// dragonfruit-400 — Pink accent
pub fn dragonfruit_400() -> Hsla {
    palette_color(0xe864a3, 0x993567)
}

/// dragonfruit-500 — Timegrid event surface
pub fn dragonfruit_500() -> Hsla {
    palette_color(0xd04388, 0xab2869)
}

// =====================================================================
// White overlays for inset rings (elevation borders)
// =====================================================================

/// white at 5% — Input/select surface
pub fn control_surface() -> Hsla {
    surface_ring(0.05)
}

/// white at 5% — Panel ring
pub fn ring_panel() -> Hsla {
    surface_ring(0.05)
}

/// white at 10% — Card ring
pub fn ring_card() -> Hsla {
    surface_ring(0.10)
}

/// white at 15% — Floating ring
pub fn ring_floating() -> Hsla {
    surface_ring(0.15)
}

/// white at 20% — Modal/dialog ring
pub fn ring_modal() -> Hsla {
    surface_ring(0.20)
}

// =====================================================================
// The app theme
// =====================================================================

/// Palette used by the native desktop surfaces and controls.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
}

/// Literal RGBA values for the selected appearance of an imported theme.
pub type ColorRoles = Arc<BTreeMap<String, [u8; 4]>>;

/// The few theme values the window and app read globally: the surface,
/// its text and the font. Everything else is a token above.
#[derive(Clone, Debug)]
pub struct Theme {
    color_roles: Option<ColorRoles>,
    pub mode: ThemeMode,
    pub font_family: SharedString,
    /// The rem size, which page zoom scales.
    pub font_size: Pixels,
    pub background: Hsla,
    pub foreground: Hsla,
    pub popover_foreground: Hsla,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            color_roles: None,
            mode: ThemeMode::Dark,
            font_family: FontFace::Interface.into(),
            font_size: px(16.0),
            background: longan_950(),
            foreground: rambutan_200(),
            popover_foreground: rambutan_100(),
        }
    }
}

impl Global for Theme {}

impl Theme {
    /// Use `mode`, creating the theme if there is none yet.
    pub fn change(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
        Self::use_family(ThemeFamily::Bruhs, mode, window, cx);
    }

    /// Apply the palette for the current light or dark appearance to every window.
    pub fn use_family(
        family: ThemeFamily,
        mode: ThemeMode,
        window: Option<&mut Window>,
        cx: &mut App,
    ) {
        Self::use_palette(*family.palette(mode), mode, window, cx);
    }

    pub fn use_palette(
        palette: Palette,
        mode: ThemeMode,
        window: Option<&mut Window>,
        cx: &mut App,
    ) {
        Self::use_color_roles(palette, None, mode, window, cx);
    }

    pub fn use_color_roles(
        palette: Palette,
        roles: Option<ColorRoles>,
        mode: ThemeMode,
        window: Option<&mut Window>,
        cx: &mut App,
    ) {
        PALETTE_MODE.set(mode);
        ACTIVE_PALETTE.set(palette);
        let theme = cx.default_global::<Theme>();
        theme.mode = mode;
        theme.color_roles = roles;
        apply_bruhs_theme(cx);
        cx.refresh_windows();
        if let Some(window) = window {
            window.refresh();
        }
    }

    pub fn color_role(&self, role: &str) -> Option<Hsla> {
        let &[r, g, b, a] = self.color_roles.as_ref()?.get(role)?;
        let rgb = u32::from_be_bytes([0, r, g, b]);
        Some(toned_color(rgb).opacity(f32::from(a) / 255.))
    }

    pub fn is_dark(&self) -> bool {
        self.mode == ThemeMode::Dark
    }

    pub fn global(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    pub fn global_mut(cx: &mut App) -> &mut Theme {
        cx.default_global::<Theme>()
    }
}

/// `cx.theme()`.
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        static FALLBACK: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
        self.try_global::<Theme>()
            .unwrap_or_else(|| FALLBACK.get_or_init(Theme::default))
    }
}

/// Install the bruhs theme: the window surface and text, and the scrollbar
/// look (always shown, in warm tones, like the web's).
pub fn apply_bruhs_theme(cx: &mut App) {
    let zoom = crate::zoom::zoom();
    let mode = cx
        .try_global::<Theme>()
        .map_or(ThemeMode::Dark, |theme| theme.mode);
    let color_roles = cx
        .try_global::<Theme>()
        .and_then(|theme| theme.color_roles.clone());
    cx.set_global(Theme {
        color_roles,
        mode,
        font_size: px(16.0) * zoom,
        ..Theme::default()
    });
    crate::scroll::set_scrollbar_style(
        crate::scroll::ScrollbarStyle {
            mode: crate::scroll::ScrollbarMode::Always,
            thumb: rambutan_500(),
            thumb_active: rambutan_400(),
        },
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scroll::ScrollableElement;
    use gpui::prelude::FluentBuilder;
    use gpui::{
        div, point, px, Context, InteractiveElement, IntoElement, Modifiers, MouseButton,
        ParentElement, Render, ScrollHandle, StatefulInteractiveElement, Styled, Window,
    };

    struct ScrollbarHarness {
        scroll: ScrollHandle,
        horizontal: bool,
    }

    impl Render for ScrollbarHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .id("scrollbar-fixture")
                .debug_selector(|| "scrollbar-fixture".to_string())
                .w(px(160.0))
                .h(px(120.0))
                .flex()
                .when(self.horizontal, |el| el.overflow_x_scroll())
                .when(!self.horizontal, |el| el.flex_col().overflow_y_scroll())
                .track_scroll(&self.scroll)
                .children((0..20).map(|index| {
                    div()
                        .when(self.horizontal, |el| el.w(px(40.0)))
                        .h(px(40.0))
                        .flex_shrink_0()
                        .when(index == 19, |row| {
                            row.debug_selector(|| "scrollbar-last-row".to_string())
                        })
                        .child(index.to_string())
                }))
                .when(self.horizontal, |el| el.horizontal_scrollbar(&self.scroll))
                .when(!self.horizontal, |el| el.vertical_scrollbar(&self.scroll))
        }
    }

    #[gpui::test]
    fn themed_scrollbar_drag_reveals_the_last_row(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            apply_bruhs_theme(cx);
        });
        let (view, cx) = cx.add_window_view(|_, _| ScrollbarHarness {
            scroll: ScrollHandle::new(),
            horizontal: false,
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("scrollbar-fixture").expect("scroll area");
        let start = point(bounds.right() - px(8.0), bounds.top() + px(8.0));
        let end = point(start.x, bounds.bottom() - px(8.0));
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        let last = cx.debug_bounds("scrollbar-last-row").expect("final row");
        assert!(last.bottom() <= bounds.bottom());
        assert!(last.top() >= bounds.top());
        view.read_with(cx, |view, _| {
            assert_eq!(view.scroll.offset().y, -view.scroll.max_offset().y);
        });
    }

    #[gpui::test]
    fn themed_scrollbar_drag_reveals_the_last_column(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            apply_bruhs_theme(cx);
        });
        let (view, cx) = cx.add_window_view(|_, _| ScrollbarHarness {
            scroll: ScrollHandle::new(),
            horizontal: true,
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("scrollbar-fixture").expect("scroll area");
        let start = point(bounds.left() + px(8.0), bounds.bottom() - px(8.0));
        let end = point(bounds.right() - px(8.0), start.y);
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        let last = cx.debug_bounds("scrollbar-last-row").expect("final column");
        assert!(last.right() <= bounds.right());
        assert!(last.left() >= bounds.left());
        view.read_with(cx, |view, _| {
            assert_eq!(view.scroll.offset().x, -view.scroll.max_offset().x);
        });
    }

    #[gpui::test]
    fn the_bruhs_theme_sets_the_surface_and_text(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            apply_bruhs_theme(cx);
            let theme = Theme::global(cx);
            assert_eq!(theme.background, longan_950());
            assert_eq!(theme.foreground, rambutan_200());
            assert_eq!(theme.font_family.as_ref(), UI_FONT_FAMILY);
        });
    }

    #[gpui::test]
    fn light_palette_and_contrast_update_surface_and_text(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            Theme::change(ThemeMode::Light, None, cx);
            let surface = longan_950();
            let text = rambutan_200();
            assert!(surface.l > text.l);
            assert_eq!(lychee_50(), rgb(0xfef2f2).into());
            set_contrast(150, cx);
            assert!(Theme::global(cx).background.l > surface.l);
            assert!(Theme::global(cx).foreground.l < text.l);
            set_contrast(100, cx);
            Theme::change(ThemeMode::Dark, None, cx);
            assert!(longan_950().l < rambutan_200().l);
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
    fn tabular_nums_should_match_web_tabular_nums_reference() {
        assert_eq!(
            tabular_nums().tag_value_list(),
            [("tnum".to_string(), 1)].as_slice()
        );
    }

    /// Web `tabular-nums` never swaps the family; nothing on desktop should
    /// fall back to the system face either.
    #[test]
    fn desktop_sources_should_not_name_the_system_font() {
        fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("source dir should be readable") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    scan(&path, hits);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && path.file_name().is_some_and(|name| name != "theme.rs")
                {
                    let text = std::fs::read_to_string(&path).expect("source should read");
                    if text.contains(".SystemUIFont") || text.contains("tabular_system_font") {
                        hits.push(path.display().to_string());
                    }
                }
            }
        }
        let mut hits = Vec::new();
        scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut hits,
        );
        assert!(hits.is_empty(), "system font in {hits:?}");
    }
}
