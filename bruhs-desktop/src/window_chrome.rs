//! Native macOS window actions for the workspace's custom title bar.
use gpui::{actions, App, KeyBinding, Menu, MenuItem, Window};
use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

actions!(
    workspace_window,
    [CloseWindow, MinimizeWindow, ZoomWindow, ToggleFullscreen]
);

fn native_window(window: &Window) -> Option<objc2::rc::Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the borrowed GPUI window owns this AppKit view on the main thread.
    unsafe { handle.ns_view.cast::<NSView>().as_ref() }.window()
}

pub(crate) fn hide_buttons(window: &Window) {
    if let Some(native) = native_window(window) {
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = native
                .standardWindowButton(kind)
                .filter(|button| !button.isHidden())
            {
                button.setHidden(true);
            }
        }
    }
}

fn visible_window(cx: &mut App, action: impl FnOnce(&Window)) {
    if let Some(handle) = cx.active_window() {
        let _ = handle.update(cx, |_, window, _| action(window));
    }
}

pub(crate) fn window_menu(cx: &mut App) -> Menu {
    cx.on_action(|_: &CloseWindow, cx| {
        visible_window(cx, |window| {
            if let Some(native) = native_window(window) {
                native.performClose(None);
            }
        });
    });
    cx.on_action(|_: &MinimizeWindow, cx| visible_window(cx, Window::minimize_window));
    cx.on_action(|_: &ZoomWindow, cx| visible_window(cx, Window::zoom_window));
    cx.on_action(|_: &ToggleFullscreen, cx| visible_window(cx, Window::toggle_fullscreen));
    cx.bind_keys([
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullscreen, None),
    ]);
    Menu::new("Window").items([
        MenuItem::action("Minimize", MinimizeWindow),
        MenuItem::action("Zoom Window", ZoomWindow),
        MenuItem::action("Toggle Full Screen", ToggleFullscreen),
        MenuItem::separator(),
        MenuItem::action("Close Window", CloseWindow),
    ])
}
