//! Shared card, floating popover, and menu row primitives.
//!
//! Uses tone-and-ring elevation (no drop shadows) matching `bruhs-web/src/ui/card.rs`
//! and `docs/ui-standards.md`.

use crate::button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, svg, Div, ElementId, FontWeight, SharedString, Stateful};

/// Radius of a panel or card (`rounded-lychee`).
pub const CARD_RADIUS_PX: f32 = 14.0;
/// Radius of a floating popover or details card.
pub const POPOVER_RADIUS_PX: f32 = 10.0;
/// Standard menu/action row height inside popovers and cards.
pub const MENU_ROW_HEIGHT_PX: f32 = 28.0;
const MENU_ROW_ICON_PX: f32 = 14.0;
const MENU_ROW_TEXT_PX: f32 = 12.5;
const POPOVER_HEADER_HEIGHT_PX: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardVariant {
    /// Solid muted surface with subtle ring (default).
    Solid,
    /// Slightly lifted surface sitting above the page background.
    Elevated,
    /// Quiet surface with a low-contrast ring for lightweight grouping.
    Subtle,
}

/// A bordered card container matching web's `Card`.
pub fn card(variant: CardVariant) -> Div {
    let (bg, border) = match variant {
        CardVariant::Solid => (theme::longan_800(), gpui::white().opacity(0.10)),
        CardVariant::Elevated => (theme::longan_700(), gpui::white().opacity(0.15)),
        CardVariant::Subtle => (theme::longan_900(), gpui::white().opacity(0.05)),
    };
    div()
        .rounded(px(CARD_RADIUS_PX))
        .p_4()
        .bg(bg)
        .border_1()
        .border_color(border)
}

/// A floating popover or card surface with tone-and-ring elevation (no drop shadow).
pub fn popover_card(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .rounded(px(POPOVER_RADIUS_PX))
        .border_1()
        .border_color(theme::ring_floating())
        .bg(theme::longan_900())
        .overflow_hidden()
        .flex()
        .flex_col()
}

/// Crisp header strip with a bottom hairline for floating cards and popovers.
pub fn popover_header(title: impl Into<SharedString>) -> Div {
    div()
        .relative()
        .h(px(POPOVER_HEADER_HEIGHT_PX))
        .pl_3()
        .pr_2()
        .border_b_1()
        .border_color(theme::longan_800().opacity(0.8))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_100())
                .child(title.into()),
        )
}

/// A full-width left-aligned action row with a leading 14px icon for menus and popovers.
pub fn menu_row(
    id: impl Into<SharedString>,
    icon_path: &'static str,
    label: impl Into<SharedString>,
    state: ButtonState,
) -> Stateful<Div> {
    let id = id.into();
    let selector = id.to_string();
    let label = label.into();
    web_button(id, WebButtonVariant::Ghost, WebButtonSize::Small, state)
        .debug_selector(move || selector)
        .aria_label(label.clone())
        .h(px(MENU_ROW_HEIGHT_PX))
        .w_full()
        .min_w_0()
        .px_2()
        .rounded(px(6.0))
        .justify_start()
        .gap_2()
        .text_size(px(MENU_ROW_TEXT_PX))
        .text_color(theme::rambutan_200())
        .child(
            svg()
                .path(icon_path)
                .size(px(MENU_ROW_ICON_PX))
                .flex_shrink_0()
                .text_color(theme::rambutan_400()),
        )
        .child(div().min_w_0().truncate().child(label))
}

#[cfg(test)]
mod tests {
    use super::{card, menu_row, popover_card, popover_header, CardVariant, MENU_ROW_HEIGHT_PX};
    use crate::button::ButtonState;
    use crate::icons;
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
                    card(CardVariant::Solid)
                        .debug_selector(|| "solid-card".to_owned())
                        .child("Card body"),
                )
                .child(
                    popover_card("sample-popover")
                        .debug_selector(|| "sample-popover".to_owned())
                        .w(gpui::px(240.0))
                        .child(popover_header("Details"))
                        .child(div().p_1p5().child(menu_row(
                            "sample-row",
                            icons::CHECK,
                            "Action",
                            ButtonState::Enabled,
                        ))),
                )
        }
    }

    #[gpui::test]
    fn card_popover_and_menu_row_render_with_consistent_geometry(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| Sample);
        cx.run_until_parked();
        assert!(cx.debug_bounds("solid-card").is_some());
        let popover = cx.debug_bounds("sample-popover").expect("sample-popover");
        assert_eq!(popover.size.width, gpui::px(240.0));
        let row = cx.debug_bounds("sample-row").expect("sample-row");
        assert_eq!(row.size.height, gpui::px(MENU_ROW_HEIGHT_PX));
    }
}
