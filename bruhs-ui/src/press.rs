//! What makes an element a button: it takes focus, shows the focus ring,
//! names itself to assistive tech and acts once per press of the mouse,
//! Enter or Space.
//!
//! gpui clicks a focused element on key **up**, after delivering the key
//! down, so an element that handles Enter/Space in `on_key_down` and also
//! listens to `on_click` acts twice for one keyboard press. [`on_press`]
//! acts on the key down and ignores the keyboard's click.

use std::rc::Rc;

use crate::theme;
use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Div, KeyDownEvent, Role, Stateful, Window};

/// Enter and Space press a focused button, like a native `<button>`.
pub fn is_activation_key(key: &str) -> bool {
    matches!(key, "enter" | "space")
}

/// A press handler: called once per mouse click, Enter or Space.
pub type PressHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// `el` acts on a mouse click, Enter or Space, once each. Stops propagation,
/// so a press never also reaches the row or card behind the button.
pub fn on_press(
    el: Stateful<Div>,
    handler: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let handler: PressHandler = Rc::new(handler);
    let key = Rc::clone(&handler);
    el.on_click(move |event: &ClickEvent, window, cx| {
        if !event.is_keyboard() {
            cx.stop_propagation();
            handler(window, cx);
        }
    })
    .on_key_down(move |event: &KeyDownEvent, window, cx| {
        if is_activation_key(event.keystroke.key.as_str()) {
            window.prevent_default();
            cx.stop_propagation();
            key(window, cx);
        }
    })
}

/// [`on_press`] for a handler on the view that renders the button.
pub fn on_press_view<V: 'static>(
    el: Stateful<Div>,
    cx: &Context<'_, V>,
    handler: impl Fn(&mut V, &mut Window, &mut Context<'_, V>) + 'static,
) -> Stateful<Div> {
    let view = cx.entity().downgrade();
    on_press(el, move |window, cx| {
        // A press on a view that is gone does nothing.
        let _ = view.update(cx, |this, cx| handler(this, window, cx));
    })
}

/// Whether a control takes presses, focus and hover styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interactivity {
    Enabled,
    Disabled,
}

impl Interactivity {
    pub fn disabled_if(disabled: bool) -> Self {
        if disabled {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }
}

/// The accessible, focusable shell of a custom control: `role`, a name, a
/// Tab stop and the persimmon focus ring on a transparent 1px border. Style
/// and children are the caller's.
pub fn focusable_control(
    el: Stateful<Div>,
    role: Role,
    label: impl Into<gpui::SharedString>,
    state: Interactivity,
) -> Stateful<Div> {
    let enabled = state == Interactivity::Enabled;
    el.role(role)
        .aria_label(label.into())
        .when(enabled, |el| el.tab_index(0).cursor_pointer())
        .when(!enabled, |el| el.cursor_not_allowed())
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
}

#[cfg(test)]
mod tests {
    use super::on_press_view;
    use ::core::prelude::v1::test;
    use gpui::{
        div, Context, FocusHandle, InteractiveElement, IntoElement, Keystroke, Modifiers,
        ParentElement, Render, StatefulInteractiveElement, Styled, TestAppContext, Window,
    };
    use std::{cell::Cell, rc::Rc};

    struct Naive {
        presses: Rc<Cell<u32>>,
        focus: FocusHandle,
    }

    impl Render for Naive {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            let click = Rc::clone(&self.presses);
            let key = Rc::clone(&self.presses);
            div().size_full().child(
                div()
                    .id("naive")
                    .track_focus(&self.focus)
                    .tab_index(0)
                    .size_10()
                    .on_click(move |_, _, _| click.set(click.get() + 1))
                    .on_key_down(move |event, window, _| {
                        if super::is_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            key.set(key.get() + 1);
                        }
                    }),
            )
        }
    }

    struct Pressed {
        presses: u32,
        focus: FocusHandle,
    }

    impl Render for Pressed {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().child(on_press_view(
                div()
                    .id("pressed")
                    .debug_selector(|| "pressed".into())
                    .track_focus(&self.focus)
                    .tab_index(0)
                    .size_10(),
                cx,
                |this, _, _| this.presses += 1,
            ))
        }
    }

    fn press_key(cx: &mut gpui::VisualTestContext, key: &str) {
        let keystroke = Keystroke {
            modifiers: Modifiers::default(),
            key: key.into(),
            key_char: None,
        };
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_click_handler_and_a_key_handler_act_twice_per_keypress(cx: &mut TestAppContext) {
        let presses = Rc::new(Cell::new(0));
        let shared = Rc::clone(&presses);
        let (view, cx) = cx.add_window_view(|_, cx| Naive {
            presses: shared,
            focus: cx.focus_handle(),
        });
        let focus = view.read_with(cx, |v, _| v.focus.clone());
        cx.update(|window, cx| focus.focus(window, cx));
        press_key(cx, "enter");
        assert_eq!(presses.get(), 2, "gpui clicks a focused element on key up");
    }

    #[gpui::test]
    fn on_press_acts_once_per_key_and_click(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| Pressed {
            presses: 0,
            focus: cx.focus_handle(),
        });
        let focus = view.read_with(cx, |v, _| v.focus.clone());
        cx.update(|window, cx| focus.focus(window, cx));
        press_key(cx, "enter");
        assert_eq!(view.read_with(cx, |v, _| v.presses), 1, "enter");
        press_key(cx, "space");
        assert_eq!(view.read_with(cx, |v, _| v.presses), 2, "space");
        press_key(cx, "a");
        assert_eq!(view.read_with(cx, |v, _| v.presses), 2, "other keys");
        let bounds = cx
            .debug_bounds("pressed")
            .map(|b| b.center())
            .unwrap_or_default();
        cx.simulate_click(bounds, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.presses), 3, "click");
    }
}
