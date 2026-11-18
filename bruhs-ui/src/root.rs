//! The window's root view: the app's surface, the "Root" key context, Tab
//! and Shift-Tab focus order (kept inside a [`FocusTrapElement`] while
//! focus is there), and the rem size from the theme.

use gpui::prelude::*;
use gpui::{
    actions, div, Action, AnyView, App, Context, FocusHandle, KeyBinding, Subscription, Window,
};

use crate::theme::ActiveTheme as _;

actions!(window_root, [Tab, TabPrev]);

/// The root's key context.
pub const CONTEXT: &str = "Root";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Tab, Some(CONTEXT)),
        KeyBinding::new("shift-tab", TabPrev, Some(CONTEXT)),
    ]);
}

/// Every window's root: wraps the app's view.
pub struct Root {
    view: AnyView,
    /// Tab with nothing focused: gpui then dispatches from above the root's
    /// key context, where its bindings match nothing.
    _unfocused_tab: Subscription,
}

impl Root {
    pub fn new(view: impl Into<AnyView>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        #[cfg(all(target_os = "macos", not(any(test, feature = "test-support"))))]
        macos::forward_accessibility_hit_tests(window);
        let window_handle = window.window_handle();
        let unfocused_tab = cx.observe_keystrokes(move |_, event, window, cx| {
            if event.action.is_some()
                || window.window_handle() != window_handle
                || window.focused(cx).is_some()
            {
                return;
            }
            let bound_to = |action: &dyn Action, cx: &App| {
                cx.key_bindings()
                    .borrow()
                    .bindings_for_action(action)
                    .any(|binding| {
                        binding.match_keystrokes(std::slice::from_ref(&event.keystroke))
                            == Some(false)
                    })
            };
            if bound_to(&Tab, cx) {
                window.focus_next(cx);
            } else if bound_to(&TabPrev, cx) {
                window.focus_prev(cx);
            }
        });
        Self {
            view: view.into(),
            _unfocused_tab: unfocused_tab,
        }
    }

    pub fn view(&self) -> &AnyView {
        &self.view
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        // Rem-based lengths follow the theme's (zoomed) font size.
        window.set_rem_size(theme.font_size);
        div()
            .id("root")
            .key_context(CONTEXT)
            .on_action(|_: &Tab, window, cx| window.focus_next(cx))
            .on_action(|_: &TabPrev, window, cx| window.focus_prev(cx))
            .relative()
            .size_full()
            .font_family(theme.font_family.clone())
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.view.clone())
    }
}

/// Keep Tab and Shift-Tab inside an element while focus is in it (a modal,
/// an expanded widget), as the web's focus traps do.
pub trait FocusTrapElement: InteractiveElement + Sized {
    /// `focus` becomes the element's own handle, and the one focus returns
    /// to when nothing inside can take it.
    fn focus_trap(self, _id: impl Into<gpui::ElementId>, focus: &FocusHandle) -> Self {
        let (next, prev) = (focus.clone(), focus.clone());
        self.track_focus(focus)
            .on_action(move |_: &Tab, window, cx| trap_advance(&next, window, cx, true))
            .on_action(move |_: &TabPrev, window, cx| trap_advance(&prev, window, cx, false))
    }
}

impl<T: InteractiveElement> FocusTrapElement for T {}

/// Move focus to the next (or previous) stop inside `container`, wrapping.
fn trap_advance(container: &FocusHandle, window: &mut Window, cx: &mut App, forward: bool) {
    let step = |window: &mut Window, cx: &mut App| {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    };
    step(window, cx);
    // gpui's order covers the whole window: step until focus is back inside,
    // and stop at the first stop seen twice (nothing inside can take it).
    let mut seen = Vec::new();
    while !container.contains_focused(window, cx) {
        let focused = window.focused(cx);
        if seen.contains(&focused) {
            window.focus(container, cx);
            break;
        }
        seen.push(focused);
        step(window, cx);
    }
}

#[cfg(all(target_os = "macos", not(any(test, feature = "test-support"))))]
mod macos {
    use objc2::ffi::class_addMethod;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSPoint;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// VoiceOver asks the window, not its view, what is under the pointer.
    /// gpui's view answers with its accessibility tree; the window only
    /// passes the question on.
    pub(super) fn forward_accessibility_hit_tests(window: &gpui::Window) {
        // `gpui::Window` has its own inherent `window_handle()`.
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        let view = appkit.ns_view.as_ptr().cast::<AnyObject>();
        // SAFETY: `view` is the live NSView gpui made for this window; asking
        // it for its window has no side effects.
        let ns_window: *mut AnyObject = unsafe { msg_send![view, window] };
        if ns_window.is_null() {
            return;
        }
        // SAFETY: `ns_window` is a live NSWindow, so it has a class.
        let class: *const AnyClass = unsafe { msg_send![ns_window, class] };
        if class.is_null() {
            return;
        }
        let imp: Imp = {
            let forward: extern "C-unwind" fn(*mut AnyObject, Sel, NSPoint) -> *mut AnyObject =
                hit_test;
            // SAFETY: an `Imp` is a type-erased method pointer; the runtime
            // calls it with the signature the type encoding below declares.
            unsafe { std::mem::transmute::<_, Imp>(forward) }
        };
        // `@@:{CGPoint=dd}`: returns an object, takes self, _cmd and a point.
        let types = c"@@:{CGPoint=dd}";
        // SAFETY: the class is live, the selector and encoding match
        // `hit_test`'s signature, and adding an existing method is a no-op.
        unsafe {
            class_addMethod(
                class.cast_mut(),
                sel!(accessibilityHitTest:),
                imp,
                types.as_ptr(),
            );
        }
    }

    extern "C-unwind" fn hit_test(this: *mut AnyObject, _: Sel, point: NSPoint) -> *mut AnyObject {
        // SAFETY: AppKit calls this on a live NSWindow.
        let view: *mut AnyObject = unsafe { msg_send![this, contentView] };
        if view.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `view` is the window's live content view.
        unsafe { msg_send![view, accessibilityHitTest: point] }
    }
}
