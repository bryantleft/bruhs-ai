//! The web's `Switch`: a persimmon track when on, longan with a white/10
//! ring when off, and a rambutan thumb.
//!
//! The switch is drawn and named here; the caller attaches the press, e.g.
//! `switch(..).when(enabled, |el| on_press_view(el, cx, toggle))`.

use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, Div, Role, SharedString, Stateful, Toggled};

use crate::press::{focusable_control, Interactivity};
use crate::theme;

pub const SWITCH_WIDTH_PX: f32 = 44.0;
pub const SWITCH_HEIGHT_PX: f32 = 24.0;
pub const SWITCH_THUMB_PX: f32 = 20.0;
const SWITCH_THUMB_OFF_X_PX: f32 = 2.0;
const SWITCH_THUMB_ON_X_PX: f32 = 22.0;
const SWITCH_RING_OPACITY: f32 = 0.10;
const SWITCH_DISABLED_OPACITY: f32 = 0.5;

/// A `role="switch"` named `label`, on when `checked`. `id` doubles as its
/// debug selector.
pub fn switch(
    id: &'static str,
    label: impl Into<SharedString>,
    checked: bool,
    state: Interactivity,
) -> Stateful<Div> {
    // The focus border is reserved in both states; the thumb sits inside it.
    let border = 1.0;
    let thumb_x = if checked {
        SWITCH_THUMB_ON_X_PX
    } else {
        SWITCH_THUMB_OFF_X_PX
    } - border;
    let thumb_y = (SWITCH_HEIGHT_PX - SWITCH_THUMB_PX) / 2.0 - border;
    focusable_control(
        div().id(id).debug_selector(move || id.to_string()),
        Role::Switch,
        label,
        state,
    )
    .aria_toggled(if checked {
        Toggled::True
    } else {
        Toggled::False
    })
    .relative()
    .flex_shrink_0()
    .w(px(SWITCH_WIDTH_PX))
    .h(px(SWITCH_HEIGHT_PX))
    .rounded_full()
    .map(|el| {
        if checked {
            el.bg(theme::persimmon_500())
        } else {
            el.bg(theme::longan_700())
                .border_color(gpui::white().opacity(SWITCH_RING_OPACITY))
        }
    })
    .when(state == Interactivity::Disabled, |el| {
        el.opacity(SWITCH_DISABLED_OPACITY)
    })
    .child(
        div()
            .absolute()
            .left(px(thumb_x))
            .top(px(thumb_y))
            .size(px(SWITCH_THUMB_PX))
            .rounded_full()
            .bg(theme::rambutan_100()),
    )
}

#[cfg(test)]
mod tests {
    use super::{switch, SWITCH_HEIGHT_PX, SWITCH_WIDTH_PX};
    use crate::press::Interactivity;
    use ::core::prelude::v1::test;
    use gpui::{div, Context, IntoElement, ParentElement, Render, Styled, TestAppContext, Window};

    struct Two;

    impl Render for Two {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(switch("on", "On", true, Interactivity::Enabled))
                .child(switch("off", "Off", false, Interactivity::Disabled))
        }
    }

    #[gpui::test]
    fn a_switch_has_the_webs_size_on_and_off(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| Two);
        cx.run_until_parked();
        for id in ["on", "off"] {
            let bounds = cx.debug_bounds(id).expect(id);
            assert_eq!(bounds.size.width, gpui::px(SWITCH_WIDTH_PX), "{id}");
            assert_eq!(bounds.size.height, gpui::px(SWITCH_HEIGHT_PX), "{id}");
        }
    }
}
