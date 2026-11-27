//! Semantic status badges and unread count pills.
//!
//! Uses the shared Bruhs fruit tokens (`longan`, `blueberry`, `pandan`,
//! `durian`, `lychee`, `persimmon`) for compact labels and numeric counters.

use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, relative, svg, App, Div, Hsla, SharedString, Window};

/// Semantic color tone for a [`Badge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
    Info,
    Success,
    Warning,
    Error,
    Accent,
}

impl BadgeTone {
    fn colors(self) -> (Hsla, Hsla) {
        match self {
            Self::Neutral => (theme::longan_800(), theme::rambutan_300()),
            Self::Info => (theme::blueberry_500().opacity(0.15), theme::blueberry_200()),
            Self::Success => (theme::pandan_500().opacity(0.15), theme::pandan_400()),
            Self::Warning => (theme::durian_500().opacity(0.15), theme::durian_300()),
            Self::Error => (theme::lychee_500().opacity(0.15), theme::lychee_300()),
            Self::Accent => (theme::persimmon_500().opacity(0.15), theme::persimmon_300()),
        }
    }
}

/// Filled semantic badge pill as a composable [`Div`].
pub fn badge(label: impl Into<SharedString>, tone: BadgeTone) -> Div {
    let (bg, fg) = tone.colors();
    div()
        .flex_shrink_0()
        .px(px(6.0))
        .rounded(px(4.0))
        .bg(bg)
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(fg)
        .child(label.into())
}

/// Compact semantic status badge, optionally prefixed by a 12 px icon.
#[derive(IntoElement)]
pub struct Badge {
    label: SharedString,
    tone: BadgeTone,
    icon: Option<&'static str>,
    inline: bool,
}

impl Badge {
    pub fn new(label: impl Into<SharedString>, tone: BadgeTone) -> Self {
        Self {
            label: label.into(),
            tone,
            icon: None,
            inline: false,
        }
    }

    pub fn icon(mut self, path: &'static str) -> Self {
        self.icon = Some(path);
        self
    }

    /// Render as borderless inline status text + icon (e.g., "Up to date").
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (bg, fg) = self.tone.colors();
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .text_size(px(12.0))
            .line_height(relative(1.5))
            .text_color(fg)
            .when(!self.inline, |el| el.px(px(6.0)).rounded(px(4.0)).bg(bg))
            .when_some(self.icon, |el, path| {
                el.child(
                    svg()
                        .path(path)
                        .size(px(theme::ICON_INLINE_SM_PX))
                        .flex_shrink_0()
                        .text_color(fg),
                )
            })
            .child(self.label)
    }
}

/// Format an unread count for [`count_badge`], capping values over 99 at `"99+"`.
pub fn format_count(count: usize) -> SharedString {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string().into()
    }
}

/// Circular/pill unread counter badge (`blueberry-500` fill, capped at `99+`).
pub fn count_badge(count: usize) -> Div {
    div()
        .flex_shrink_0()
        .min_w(px(18.0))
        .px(px(5.0))
        .rounded(px(9.0))
        .bg(theme::blueberry_500())
        .text_color(theme::rambutan_100())
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .flex()
        .justify_center()
        .child(format_count(count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons;
    use gpui::{Context, TestAppContext};

    #[test]
    fn count_badge_caps_over_ninety_nine() {
        assert_eq!(format_count(0).as_ref(), "0");
        assert_eq!(format_count(99).as_ref(), "99");
        assert_eq!(format_count(100).as_ref(), "99+");
    }

    struct BadgeHarness;

    impl gpui::Render for BadgeHarness {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut Context<'_, Self>,
        ) -> impl IntoElement {
            div()
                .flex()
                .gap_2()
                .child(Badge::new("Synced", BadgeTone::Info))
                .child(
                    Badge::new("Up to date", BadgeTone::Success)
                        .icon(icons::CHECK)
                        .inline(),
                )
                .child(count_badge(128))
        }
    }

    #[gpui::test]
    fn badges_render_in_light_and_dark_themes(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| BadgeHarness);
        cx.run_until_parked();
    }
}
