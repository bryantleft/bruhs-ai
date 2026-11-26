//! Modal dialog backdrop and card chrome (`role="dialog"` / `role="alertdialog"`).
//!
//! Desktop twin of `bruhs-web/src/ui/dialog.rs`: warm `longan-950/70` scrim,
//! `14px` (`lychee`) card radius, `longan-800` modal tone, and `ring_modal()` border
//! with no drop shadows.

use crate::native_blur::BlurScrimHandle;
use crate::theme::{self, ActiveTheme};
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, relative, App, Div, ElementId, Role, SharedString, Stateful};

/// Modal scrim opacity (`bg-longan-950/70`).
pub const DIALOG_OVERLAY_OPACITY: f32 = 0.70;
/// Modal card corner radius (`rounded-lychee`).
pub const DIALOG_RADIUS_PX: f32 = 14.0;
/// Top padding for top-anchored pickers (`5rem` = 80px).
pub const DIALOG_TOP_PADDING_PX: f32 = 80.0;
/// Vertical edge padding for centered modals (`2rem` = 32px).
pub const DIALOG_EDGE_GAP_PX: f32 = 32.0;
/// Maximum horizontal share of the window a modal may occupy.
pub const DIALOG_MAX_WIDTH_RATIO: f32 = 0.92;

/// Where a modal card sits inside its backdrop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogPlacement {
    /// Vertically centered with breathing room above and below.
    Centered,
    /// Anchored near the top (for pickers/palettes whose list height changes).
    Top,
}

/// Corner shape shared by the card and its native backdrop opening.
#[derive(Clone, Copy, Debug)]
pub enum DialogCorners {
    Standard,
    Rounded(f32),
    TopRounded(f32),
}

impl DialogCorners {
    /// Apply the corners and the matching blur to a custom dialog shell.
    pub fn apply(self, card: Stateful<Div>, blur: &BlurScrimHandle) -> Stateful<Div> {
        let (top, bottom) = match self {
            Self::Standard => (DIALOG_RADIUS_PX, DIALOG_RADIUS_PX),
            Self::Rounded(radius) => (radius, radius),
            Self::TopRounded(radius) => (radius, 0.0),
        };
        let (top, bottom) = (px(top), px(bottom));
        card.rounded_t(top)
            .rounded_b(bottom)
            .child(blur.backdrop(top, bottom))
    }
}

/// Full-window modal scrim that blocks clicks underneath.
pub fn dialog_backdrop(id: impl Into<ElementId>, placement: DialogPlacement) -> Stateful<Div> {
    let base = div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .occlude()
        .id(id)
        .bg(theme::longan_950().opacity(DIALOG_OVERLAY_OPACITY))
        .flex()
        .justify_center();
    match placement {
        DialogPlacement::Centered => base.items_center().py(px(DIALOG_EDGE_GAP_PX)),
        DialogPlacement::Top => base.items_start().pt(px(DIALOG_TOP_PADDING_PX)),
    }
}

/// Modal card shell with `role="dialog"` (or `AlertDialog`), modal tone, and `tab_group()`.
pub fn dialog_card(
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    role: Role,
    blur: &BlurScrimHandle,
    cx: &App,
) -> Stateful<Div> {
    dialog_card_with_corners(id, aria_label, role, DialogCorners::Standard, blur, cx)
}

/// The shared dialog shell with a custom corner shape, such as a bottom sheet.
pub fn dialog_card_with_corners(
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    role: Role,
    corners: DialogCorners,
    blur: &BlurScrimHandle,
    cx: &App,
) -> Stateful<Div> {
    let card = div()
        .id(id)
        .role(role)
        .aria_label(aria_label.into())
        .max_w(relative(DIALOG_MAX_WIDTH_RATIO))
        .border_1()
        .border_color(theme::ring_modal())
        .bg(theme::glass_surface(theme::longan_800()))
        .text_color(cx.theme().popover_foreground)
        .flex()
        .flex_col()
        .overflow_hidden()
        .tab_group();
    corners.apply(card, blur)
}

#[cfg(test)]
mod tests {
    use super::{dialog_backdrop, dialog_card, DialogPlacement};
    use crate::native_blur::BlurScrimHandle;
    use ::core::prelude::v1::test;
    use gpui::{
        div, Context, InteractiveElement, IntoElement, ParentElement, Render, Role, Styled,
        TestAppContext, Window,
    };

    #[derive(Default)]
    struct Sample(BlurScrimHandle);

    impl Render for Sample {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().relative().child(
                dialog_backdrop("sample-overlay", DialogPlacement::Centered).child(
                    dialog_card("sample-dialog", "Sample", Role::Dialog, &self.0, cx)
                        .debug_selector(|| "sample-dialog".to_owned())
                        .w(gpui::px(420.0))
                        .p_4()
                        .child("Hello"),
                ),
            )
        }
    }

    #[gpui::test]
    fn dialog_shell_renders_centered_card(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (sample, cx) = cx.add_window_view(|_, _| Sample::default());
        cx.run_until_parked();
        let bounds = cx.debug_bounds("sample-dialog").expect("sample-dialog");
        assert_eq!(bounds.size.width, gpui::px(420.0));
        assert!(bounds.size.height > gpui::px(0.0));
        assert_eq!(
            sample.read_with(cx, |sample, _| sample.0.clear_bounds()),
            Some(bounds)
        );
    }
}
