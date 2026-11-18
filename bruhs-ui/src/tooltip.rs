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

/// A one-line text tooltip.
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
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        // The margin sits on an outer box so the bubble keeps its own size.
        div().p(px(OFFSET_PX)).child(
            div()
                .id("tooltip")
                .role(gpui::Role::Tooltip)
                .rounded(px(RADIUS_PX))
                .bg(theme::longan_700())
                .border_1()
                .border_color(theme::ring_floating())
                .px(px(X_PADDING_PX))
                .py(px(Y_PADDING_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_size(px(TEXT_SIZE_PX))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_100())
                .whitespace_nowrap()
                .child(self.text.clone()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

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
}
