//! Display a keyboard shortcut keycap, e.g. `kbd("/")` or `kbd("⌘K")`.
//! Desktop twin of `bruhs-web/src/ui/kbd.rs`.

use crate::theme::{self, FontFace};
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, Div, SharedString};

const KBD_RADIUS_PX: f32 = 4.0;
const KBD_MIN_WIDTH_PX: f32 = 18.0;
const KBD_HEIGHT_PX: f32 = 18.0;
const KBD_TEXT_SIZE_PX: f32 = 11.0;

/// A monospace keycap pill for inline shortcut hints.
pub fn kbd(label: impl Into<SharedString>) -> Div {
    div()
        .min_w(px(KBD_MIN_WIDTH_PX))
        .h(px(KBD_HEIGHT_PX))
        .px(px(5.0))
        .rounded(px(KBD_RADIUS_PX))
        .bg(theme::longan_800().opacity(0.65))
        .border_1()
        .border_color(theme::ring_card())
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .font_family(FontFace::Code)
        .text_size(px(KBD_TEXT_SIZE_PX))
        .text_color(theme::rambutan_400())
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::{kbd, KBD_HEIGHT_PX, KBD_MIN_WIDTH_PX};
    use ::core::prelude::v1::test;
    use gpui::{
        div, Context, InteractiveElement, IntoElement, ParentElement, Render, Styled,
        TestAppContext, Window,
    };

    struct Sample;

    impl Render for Sample {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .items_start()
                .child(kbd("/").debug_selector(|| "kbd-slash".to_owned()))
        }
    }

    #[gpui::test]
    fn kbd_renders_at_keycap_height(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| Sample);
        cx.run_until_parked();
        let bounds = cx.debug_bounds("kbd-slash").expect("kbd-slash");
        assert!(bounds.size.width >= gpui::px(KBD_MIN_WIDTH_PX));
        assert_eq!(bounds.size.height, gpui::px(KBD_HEIGHT_PX));
    }
}
