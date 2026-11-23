//! Accessible tab lists and triggers (`role="tablist"` / `role="tab"`).
//!
//! Covers both pill tabs (used in import/move-in pickers) and segmented tabs
//! (used in compact destination switchers like Capture).

use crate::theme::{self, FontFace};
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, Div, ElementId, FontWeight, Role, SharedString, Stateful};

const TAB_RADIUS_PX: f32 = 8.0;
const SEGMENTED_TRIGGER_RADIUS_PX: f32 = 6.0;
const TAB_TEXT_SIZE_PX: f32 = 12.0;

/// Visual treatment for a tab list and its triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStyle {
    /// Scrollable row of pill tabs (`blueberry` active tint), for dialog source pickers.
    Pill,
    /// Recessed track (`longan-900/50`) with raised active tab (`longan-700`).
    Segmented,
}

/// A `role="tablist"` container.
pub fn tab_list(
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    style: TabStyle,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .role(Role::TabList)
        .aria_label(aria_label.into())
        .flex()
        .gap_1();
    match style {
        TabStyle::Pill => base.overflow_x_scroll(),
        TabStyle::Segmented => base
            .rounded(px(TAB_RADIUS_PX))
            .bg(theme::longan_900().opacity(0.5))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .p_1(),
    }
}

/// A `role="tab"` trigger. Attach activation with `on_press` / `on_press_view`.
pub fn tab_trigger(
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    selected: bool,
    style: TabStyle,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .role(Role::Tab)
        .aria_label(aria_label.into())
        .aria_selected(selected)
        .tab_index(0)
        .border_1()
        .cursor_pointer()
        .focus_visible(|el| el.border_color(theme::persimmon_400()));
    match style {
        TabStyle::Pill => base
            .flex_shrink_0()
            .whitespace_nowrap()
            .rounded(px(TAB_RADIUS_PX))
            .px(px(10.0))
            .py_1()
            .font_family(FontFace::Small)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(TAB_TEXT_SIZE_PX))
            .map(|el| {
                if selected {
                    el.bg(theme::blueberry_600().opacity(0.20))
                        .border_color(theme::blueberry_500().opacity(0.30))
                        .text_color(theme::blueberry_200())
                } else {
                    el.border_color(gpui::transparent_black())
                        .text_color(theme::rambutan_400())
                        .hover(|el| {
                            el.bg(theme::longan_800().opacity(0.50))
                                .text_color(theme::rambutan_200())
                        })
                }
            }),
        TabStyle::Segmented => base
            .border_color(gpui::transparent_black())
            .flex_1()
            .min_w_0()
            .rounded(px(SEGMENTED_TRIGGER_RADIUS_PX))
            .px_1()
            .py_2()
            .text_xs()
            .text_center()
            .text_color(if selected {
                theme::rambutan_100()
            } else {
                theme::rambutan_500()
            })
            .when(selected, |el| el.bg(theme::longan_700()))
            .hover(|el| {
                el.bg(theme::longan_800())
                    .text_color(theme::rambutan_200())
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::{tab_list, tab_trigger, TabStyle};
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
                .flex_col()
                .gap_2()
                .child(
                    tab_list("pills", "Sources", TabStyle::Pill)
                        .child(
                            tab_trigger("p1", "First", true, TabStyle::Pill)
                                .debug_selector(|| "tab-p1".to_owned())
                                .child("First"),
                        )
                        .child(
                            tab_trigger("p2", "Second", false, TabStyle::Pill)
                                .debug_selector(|| "tab-p2".to_owned())
                                .child("Second"),
                        ),
                )
                .child(
                    tab_list("segs", "Modes", TabStyle::Segmented).child(
                        tab_trigger("s1", "One", true, TabStyle::Segmented)
                            .debug_selector(|| "tab-s1".to_owned())
                            .child("One"),
                    ),
                )
        }
    }

    #[gpui::test]
    fn tabs_render_both_styles(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| Sample);
        cx.run_until_parked();
        for selector in ["tab-p1", "tab-p2", "tab-s1"] {
            let bounds = cx.debug_bounds(selector).expect(selector);
            assert!(bounds.size.width > gpui::px(0.0), "{selector}");
            assert!(bounds.size.height > gpui::px(0.0), "{selector}");
        }
    }
}
