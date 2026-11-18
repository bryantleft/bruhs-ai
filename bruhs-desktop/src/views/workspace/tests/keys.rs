//! Keyboard helpers for acceptance tests, which check what keys do: gpui's
//! test platform drops accessibility updates, so a control is found by its
//! focus handle and pressed with a key down and key up, as the app gets it.

use super::*;

/// A key press as the app gets it: gpui clicks a focused button on key up.
pub(super) fn press(cx: &mut gpui::VisualTestContext, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

pub(super) fn focused(cx: &mut gpui::VisualTestContext) -> Option<gpui::FocusHandle> {
    cx.update(|window, cx| window.focused(cx))
}

/// Tabs forward until `target` has focus.
pub(super) fn tab_to(cx: &mut gpui::VisualTestContext, target: &gpui::FocusHandle, name: &str) {
    for _ in 0..500 {
        if focused(cx).as_ref() == Some(target) {
            return;
        }
        cx.simulate_keystrokes("tab");
    }
    panic!("Tab never reached {name}");
}

/// Every Tab stop in one full turn, in Tab order, from the top of the page.
pub(super) fn tab_stops(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
) -> Vec<gpui::FocusHandle> {
    focus_page(workspace, cx);
    cx.simulate_keystrokes("tab");
    let first = focused(cx).expect("a Tab stop");
    let mut stops = vec![first.clone()];
    for _ in 0..1000 {
        cx.simulate_keystrokes("tab");
        let next = focused(cx).expect("a Tab stop");
        if next == first {
            return stops;
        }
        stops.push(next);
    }
    panic!("Tab never came back around");
}

/// The Tab stops in `after` that `before` did not have, in Tab order: the
/// controls a change added.
pub(super) fn added_stops(
    before: &[gpui::FocusHandle],
    after: &[gpui::FocusHandle],
) -> Vec<gpui::FocusHandle> {
    after
        .iter()
        .filter(|stop| !before.contains(stop))
        .cloned()
        .collect()
}

/// A control's focus handle, found by pressing the pointer on it and
/// releasing away from it: focus moves there and nothing is clicked. The
/// test then reaches the control again with Tab.
pub(super) fn focus_handle_of(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> gpui::FocusHandle {
    let target = rendered_bounds(cx, selector).center();
    cx.simulate_mouse_down(target, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(
        gpui::point(px(640.0), px(4.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    focused(cx).unwrap_or_else(|| panic!("{selector} takes focus"))
}

/// Focus at the top of the page, where a navigation leaves it.
pub(super) fn focus_page(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) {
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.workspace_focus_handle.focus(window, cx);
    });
    cx.run_until_parked();
}

/// Whether focus is inside the open dialog.
pub(super) fn in_dialog(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> bool {
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .modal_start_focus_handle
            .contains_focused(window, cx)
    })
}
