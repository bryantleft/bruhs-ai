//! The tooltip a hovered element shows: the web's `Tooltip` bubble
//! (`rounded-grape bg-longan-700 inset-ring-white/15 px-2 py-1
//! text-label-sm text-rambutan-100`), with no shadow.
//!
//! ```ignore
//! div().id("save").tooltip(|window, cx| Tooltip::new("Save").build(window, cx))
//! ```

use gpui::prelude::*;
use gpui::{div, AnyView, App, Context, FontWeight, Render, SharedString, Window};

use crate::theme;
use crate::zoom::px;

const RADIUS_PX: f32 = 8.0;
const X_PADDING_PX: f32 = 8.0;
const Y_PADDING_PX: f32 = 4.0;
const TEXT_SIZE_PX: f32 = 12.0;
/// Clear of the pointer, which gpui places the tooltip beside.
const OFFSET_PX: f32 = 12.0;
/// Maximum tooltip bubble width before wrapping to additional lines.
const MAX_WIDTH_PX: f32 = 320.0;
/// Floor width so ultra-narrow viewports still render legible wrapped text.
const MIN_WIDTH_PX: f32 = 120.0;

/// A text tooltip that stays single-line for short labels and wraps cleanly
/// within narrow windows or elevated zoom levels.
pub struct Tooltip {
    text: SharedString,
}

impl Tooltip {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }

    /// The view gpui's `.tooltip(..)` builder returns.
    pub fn build(self, _window: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| self).into()
    }

    /// The text it shows.
    pub fn text(&self) -> &SharedString {
        &self.text
    }

    fn max_bubble_width(viewport_width: gpui::Pixels) -> gpui::Pixels {
        let available = (viewport_width - px(OFFSET_PX * 2.0)).max(px(MIN_WIDTH_PX));
        available.min(px(MAX_WIDTH_PX))
    }
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        let max_width = Self::max_bubble_width(window.viewport_size().width);
        // The margin sits on an outer box so the bubble keeps its own size.
        div().p(px(OFFSET_PX)).child(
            div()
                .id("tooltip")
                .debug_selector(|| "tooltip-bubble".to_owned())
                .role(gpui::Role::Tooltip)
                .max_w(max_width)
                .rounded(px(RADIUS_PX))
                .bg(theme::longan_700())
                .border_1()
                .border_color(theme::ring_floating())
                .px(px(X_PADDING_PX))
                .py(px(Y_PADDING_PX))
                .font_family(theme::FontFace::Small)
                .text_size(px(TEXT_SIZE_PX))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_100())
                .whitespace_normal()
                .child(self.text.clone()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{size, TestAppContext};

    struct Blank;
    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn a_tooltip_builds_a_view_holding_its_text(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| Blank);
        let view = window
            .update(cx, |_, window, cx| Tooltip::new("Save").build(window, cx))
            .unwrap();
        let tooltip = view.downcast::<Tooltip>().unwrap();
        cx.read(|cx| assert_eq!(tooltip.read(cx).text(), "Save"));
    }

    #[gpui::test]
    fn long_tooltip_wraps_within_narrow_viewport_and_zoom(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let long_text = "Configure automatic background sync and retention policies for linked workspaces across every connected host";
        let (_, cx) = cx.add_window_view(move |_, _| Tooltip::new(long_text));
        cx.simulate_resize(size(gpui::px(240.0), gpui::px(200.0)));
        cx.run_until_parked();
        let narrow = cx.debug_bounds("tooltip-bubble").expect("tooltip-bubble");
        assert!(
            narrow.size.width <= gpui::px(216.0) + gpui::px(1.0),
            "narrow tooltip width {:?} exceeded viewport cap",
            narrow.size.width
        );
        assert!(
            narrow.size.height > gpui::px(32.0),
            "expected wrapped multi-line height, got {:?}",
            narrow.size.height
        );

        cx.update(|window, cx| {
            crate::zoom::set_zoom(2.0, cx);
            window.refresh();
        });
        cx.simulate_resize(size(gpui::px(640.0), gpui::px(480.0)));
        cx.run_until_parked();
        let zoomed = cx.debug_bounds("tooltip-bubble").expect("tooltip-bubble");
        assert!(
            zoomed.size.width <= gpui::px(640.0),
            "zoomed tooltip width {:?} overflowed 640px window",
            zoomed.size.width
        );
        cx.update(|window, cx| {
            crate::zoom::set_zoom(1.0, cx);
            window.refresh();
        });
    }
}
