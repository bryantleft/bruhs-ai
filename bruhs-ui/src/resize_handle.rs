//! The web's panel divider: a 4px strip just inside the trailing panel's
//! leading edge, transparent until hovered or dragged, then persimmon at 40%
//! (`hover:bg-persimmon-400/40`).
//!
//! gpui-component owns the handle's hit area, cursor and drag; this only
//! paints inside it. Pass it to a panel group's `with_handle_appearance`.

use std::rc::Rc;

use crate::resizable::{ResizeHandleRenderer, ResizeHandleState};
use gpui::{div, Axis, IntoElement, Styled};

use crate::theme;

/// The strip's thickness across the divider, in window points like the rest
/// of the window chrome.
pub const HANDLE_STRIP_PX: f32 = 4.0;
const HANDLE_ACTIVE_OPACITY: f32 = 0.4;

/// The bruhs divider appearance for `h_resizable` / `v_resizable` groups.
pub fn handle_appearance() -> ResizeHandleRenderer {
    Rc::new(|handle, _window, _cx| {
        let color = if handle.state() == ResizeHandleState::Idle {
            gpui::transparent_black()
        } else {
            theme::persimmon_400().opacity(HANDLE_ACTIVE_OPACITY)
        };
        // Wider than the handle's one-point content box, so it must not
        // shrink; it overhangs into the trailing panel.
        let strip = div().flex_none().bg(color);
        Some(
            match handle.axis() {
                Axis::Horizontal => strip.w(gpui::px(HANDLE_STRIP_PX)).h_full(),
                _ => strip.h(gpui::px(HANDLE_STRIP_PX)).w_full(),
            }
            .into_any_element(),
        )
    })
}
