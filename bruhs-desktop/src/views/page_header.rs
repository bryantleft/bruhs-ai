//! Desktop twin of the web `PageHeader` (`bruhs-web/src/ui/page_header.rs`):
//! the page title, an optional one-line subtitle, and room for actions on the
//! right, on a bar above the page's scrolling content.

use crate::theme;
use gpui::prelude::*;
use gpui::*;

/// The header bar. Add the page's actions with `.child(…)`; they sit on the
/// right.
pub fn page_header(title: impl Into<SharedString>, subtitle: Option<SharedString>) -> Div {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_3()
        .px_6()
        .py_3()
        .border_b_1()
        .border_color(theme::longan_800().opacity(0.5))
        .child(
            // flex_1: a bare min_w_0 column collapses to zero width in a
            // GPUI row and wraps its text one glyph per line.
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_family(theme::HEADING_FONT_FAMILY)
                        .text_size(px(16.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::rambutan_100())
                        .child(title.into()),
                )
                .when_some(subtitle, |header, subtitle| {
                    header.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_500())
                            .child(subtitle),
                    )
                }),
        )
}
