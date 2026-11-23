//! The web's `Checkbox`: a 24px `role="checkbox"` box, persimmon with a
//! check when checked, the input tone with a ring otherwise, and an optional
//! label beside it that toggles it too.
//!
//! ```ignore
//! Checkbox::new("due")
//!     .label("Due tomorrow")
//!     .checked(due)
//!     .on_click(|checked, _window, _cx| println!("now {checked}"))
//! ```

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, svg, App, ClickEvent, ElementId, FontWeight, SharedString, Toggled, Window};

use crate::press::{focusable_control, on_press, Interactivity};
use crate::zoom::px;
use crate::{icons, theme};

const BOX_PX: f32 = 24.0;
/// `rounded-seed`.
const BOX_RADIUS_PX: f32 = 4.0;
const CHECK_PX: f32 = 14.0;
const LABEL_GAP_PX: f32 = 8.0;
const LABEL_TEXT_PX: f32 = 13.0;
const UNCHECKED_BG_OPACITY: f32 = 0.05;
const HOVER_BG_OPACITY: f32 = 0.10;
const RING_OPACITY: f32 = 0.15;
const DISABLED_OPACITY: f32 = 0.5;

/// Called with the state a press asks for (the opposite of the current one).
type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// A checkbox the caller keeps the state of: it draws `checked` and reports
/// a press through [`Checkbox::on_click`].
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    debug_selector: Option<SharedString>,
    label: Option<SharedString>,
    aria_label: Option<SharedString>,
    checked: bool,
    disabled: bool,
    on_click: Option<ChangeHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            debug_selector: None,
            label: None,
            aria_label: None,
            checked: false,
            disabled: false,
            on_click: None,
        }
    }

    /// Optional debug selector attached to the focusable checkbox control.
    pub fn debug_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.debug_selector = Some(selector.into());
        self
    }

    /// Text beside the box, which also names it for assistive tech.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The name assistive tech reads, for a box whose label is drawn
    /// elsewhere.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `handler` gets the new state when a click, Enter or Space toggles it.
    pub fn on_click(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let checked = self.checked;
        let state = Interactivity::disabled_if(self.disabled);
        let next = !checked;
        let handler = self.on_click.filter(|_| !self.disabled);
        let label = self
            .aria_label
            .clone()
            .or_else(|| self.label.clone())
            .unwrap_or_default();

        let mut check = focusable_control(
            div()
                .id(self.id.clone())
                .when_some(self.debug_selector, |el, selector| {
                    el.debug_selector(move || selector.to_string())
                }),
            gpui::Role::CheckBox,
            label,
            state,
        )
        .aria_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        })
        .flex_none()
        .size(px(BOX_PX))
        .rounded(px(BOX_RADIUS_PX))
        .flex()
        .items_center()
        .justify_center()
        .map(|el| {
            if checked {
                el.bg(theme::persimmon_500())
            } else {
                // The ring is the control's border until it takes focus.
                el.bg(gpui::white().opacity(UNCHECKED_BG_OPACITY))
                    .border_color(gpui::white().opacity(RING_OPACITY))
                    .when(state == Interactivity::Enabled, |el| {
                        el.hover(|el| el.bg(gpui::white().opacity(HOVER_BG_OPACITY)))
                    })
            }
        })
        .when(checked, |el| {
            el.child(
                svg()
                    .path(icons::CHECK)
                    .size(px(CHECK_PX))
                    .text_color(theme::longan_950()),
            )
        });
        if let Some(handler) = handler.clone() {
            check = on_press(check, move |window, cx| handler(&next, window, cx));
        }

        let Some(text) = self.label else {
            return check
                .when(self.disabled, |el| el.opacity(DISABLED_OPACITY))
                .into_any_element();
        };
        // Like a `<label>`, the text toggles the box; it takes no focus of its
        // own, so Tab stops on the box only.
        div()
            .id(ElementId::NamedChild(self.id.into(), "label".into()))
            .flex()
            .items_center()
            .gap(px(LABEL_GAP_PX))
            .when(self.disabled, |el| el.opacity(DISABLED_OPACITY))
            .child(check)
            .child(
                div()
                    .text_size(px(LABEL_TEXT_PX))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(theme::rambutan_200())
                    .child(text),
            )
            .when_some(handler, |el, handler| {
                el.cursor_pointer()
                    .on_click(move |event: &ClickEvent, window, cx| {
                        if !event.is_keyboard() {
                            handler(&next, window, cx);
                        }
                    })
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        actions, Context, KeyBinding, Modifiers, MouseButton, TestAppContext, VisualTestContext,
    };
    use std::cell::RefCell;

    actions!(checkbox_test, [FocusNext]);

    struct Form {
        checked: bool,
        changes: Rc<RefCell<Vec<bool>>>,
        disabled: bool,
        focus: gpui::FocusHandle,
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let changes = self.changes.clone();
            let view = cx.entity().downgrade();
            div()
                .size_full()
                .track_focus(&self.focus)
                .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
                .child(
                    Checkbox::new("due")
                        .label("Due tomorrow")
                        .checked(self.checked)
                        .disabled(self.disabled)
                        .on_click(move |checked, _, cx| {
                            changes.borrow_mut().push(*checked);
                            let checked = *checked;
                            let _ = view.update(cx, |form, cx| {
                                form.checked = checked;
                                cx.notify();
                            });
                        }),
                )
        }
    }

    fn form(
        cx: &mut TestAppContext,
        disabled: bool,
    ) -> (Rc<RefCell<Vec<bool>>>, &mut VisualTestContext) {
        let changes = Rc::new(RefCell::new(Vec::new()));
        let shared = changes.clone();
        cx.update(|cx| cx.bind_keys([KeyBinding::new("tab", FocusNext, None)]));
        let (form, cx) = cx.add_window_view(move |_, cx| Form {
            checked: false,
            changes: shared,
            disabled,
            focus: cx.focus_handle(),
        });
        // gpui dispatches no keys until something in the window has focus.
        cx.update(|window, cx| {
            let focus = form.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        (changes, cx)
    }

    #[gpui::test]
    fn a_click_on_the_box_or_its_label_toggles_it(cx: &mut TestAppContext) {
        let (changes, cx) = form(cx, false);
        cx.simulate_click(gpui::point(px(12.0), px(12.0)), Modifiers::none());
        cx.simulate_click(gpui::point(px(60.0), px(12.0)), Modifiers::none());
        assert_eq!(*changes.borrow(), vec![true, false]);
    }

    #[gpui::test]
    fn space_toggles_the_focused_box_once(cx: &mut TestAppContext) {
        let (changes, cx) = form(cx, false);
        cx.simulate_keystrokes("tab space");
        assert_eq!(*changes.borrow(), vec![true]);
    }

    #[gpui::test]
    fn a_disabled_checkbox_ignores_presses(cx: &mut TestAppContext) {
        let (changes, cx) = form(cx, true);
        cx.simulate_mouse_down(
            gpui::point(px(12.0), px(12.0)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_up(
            gpui::point(px(12.0), px(12.0)),
            MouseButton::Left,
            Modifiers::none(),
        );
        assert!(changes.borrow().is_empty());
    }
}
