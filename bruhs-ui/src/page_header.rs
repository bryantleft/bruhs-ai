//! Desktop twin of the web `PageHeader` (`bruhs-web/src/ui/page_header.rs`):
//! the page title, an optional one-line subtitle, and room for actions on the
//! right, on a bar above the page's scrolling content.

use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::*;

/// The header bar. Add the page's actions with `.child(…)`; they sit on the
/// right, and like the web's they wrap under the title when the page is too
/// narrow for both (give a group of them `flex_wrap` too).
pub fn page_header(title: impl Into<SharedString>, subtitle: Option<SharedString>) -> Div {
    let title: SharedString = title.into();
    div()
        .flex()
        .flex_wrap()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_3()
        .px_6()
        .py_3()
        .border_b_1()
        .border_color(theme::longan_800().opacity(0.5))
        .child(
            // Web `min-w-0` in a wrapping row: the title keeps its width
            // and the actions wrap; a long subtitle wraps within the row.
            // (`flex_1` here squeezed the title to a glyph per line when
            // the actions filled the row.)
            div()
                .debug_selector(|| "desktop-page-title".to_string())
                .min_w_0()
                .child(
                    // Web `text-h3`: Xenon 18px, weight 500, the page's `h1`.
                    div()
                        .id("page-title")
                        .role(Role::Heading)
                        .aria_level(1)
                        .aria_label(title.clone())
                        .font_family(theme::HEADING_FONT_FAMILY)
                        .text_size(px(18.0))
                        .font_weight(FontWeight::MEDIUM)
                        .line_height(relative(1.5))
                        .text_color(theme::rambutan_100())
                        .child(title),
                )
                .when_some(subtitle, |header, subtitle| {
                    // Web `mt-0.5 text-body-sm`: Neon 13px.
                    header.child(
                        div()
                            .mt(px(2.0))
                            .font_family(theme::CODE_FONT_FAMILY)
                            .text_size(px(13.0))
                            .line_height(relative(1.5))
                            .text_color(theme::rambutan_500())
                            .child(subtitle),
                    )
                }),
        )
}
