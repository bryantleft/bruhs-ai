//! Text fields: single-line [`Input`] and multi-line [`Textarea`], their
//! states, events and editing actions.
//!
//! A field's state ([`InputState`], [`TextareaState`]) holds the text, the
//! selection and the undo history; the component draws it. Typing, IME
//! composition and the platform's text services go through gpui's
//! [`gpui::EntityInputHandler`]. Edits the person makes report
//! [`InputEvent::Change`]; [`EditorState::set_value`] never does.

mod actions;
mod core;
mod element;
mod state;
mod view;

#[cfg(test)]
use gpui::Pixels;
use gpui::{App, FocusHandle, Global, WeakFocusHandle, Window};

pub use actions::{
    Backspace, Copy, Cut, Delete, DeleteToBeginningOfLine, DeleteToEndOfLine, DeleteToNextWordEnd,
    DeleteToPreviousWordStart, Enter, Escape, IndentInline, MoveDown, MoveEnd, MoveHome, MoveLeft,
    MoveRight, MoveToEnd, MoveToNextWord, MoveToPreviousWord, MoveToStart, MoveUp, OutdentInline,
    Paste, Redo, SelectAll, SelectDown, SelectLeft, SelectRight, SelectToEnd, SelectToEndOfLine,
    SelectToNextWordEnd, SelectToPreviousWordStart, SelectToStart, SelectToStartOfLine, SelectUp,
    Undo, CONTEXT,
};
pub use state::{
    EditorState, FieldKind, InputContentType, InputEvent, InputState, MultiLine, SingleLine,
    TextareaState,
};
pub use view::{Input, Sizable, Size, Textarea};

/// Every field's focus handle, so the app can ask whether one has focus.
#[derive(Default)]
struct Fields(Vec<WeakFocusHandle>);

impl Global for Fields {}

pub(crate) fn register_field(focus: &FocusHandle, cx: &mut App) {
    let fields = cx.default_global::<Fields>();
    fields.0.retain(|weak| weak.upgrade().is_some());
    fields.0.push(focus.downgrade());
}

/// Window helpers for text fields.
pub trait WindowExt {
    /// Whether a text field has focus (so plain keys type rather than act).
    fn has_focused_input(&mut self, cx: &App) -> bool;
}

impl WindowExt for Window {
    fn has_focused_input(&mut self, cx: &App) -> bool {
        cx.try_global::<Fields>().is_some_and(|fields| {
            fields
                .0
                .iter()
                .filter_map(WeakFocusHandle::upgrade)
                .any(|focus| focus.is_focused(self))
        })
    }
}

/// Bind the fields' editing keys.
pub fn init(cx: &mut App) {
    actions::bind_keys(cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::prelude::*;
    use gpui::{
        div, px, size, Context, Entity, EntityInputHandler as _, Focusable as _, TestAppContext,
        VisualTestContext,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Form {
        field: Entity<InputState>,
        readonly: bool,
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .w(px(120.))
                .child(Input::new(&self.field).readonly(self.readonly))
        }
    }

    fn form(
        cx: &mut TestAppContext,
    ) -> (
        Entity<InputState>,
        Rc<RefCell<Vec<InputEvent>>>,
        &mut VisualTestContext,
    ) {
        form_with(cx, false)
    }

    fn form_with(
        cx: &mut TestAppContext,
        readonly: bool,
    ) -> (
        Entity<InputState>,
        Rc<RefCell<Vec<InputEvent>>>,
        &mut VisualTestContext,
    ) {
        cx.update(crate::init);
        let events = Rc::new(RefCell::new(Vec::new()));
        let seen = events.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let field = cx.new(|cx| InputState::new(window, cx));
            cx.subscribe(&field, move |_, _, event: &InputEvent, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
            Form { field, readonly }
        });
        cx.simulate_resize(size(px(400.), px(200.)));
        let field = cx.read(|cx| view.read(cx).field.clone());
        cx.update(|window, cx| field.update(cx, |field, cx| field.focus(window, cx)));
        cx.run_until_parked();
        (field, events, cx)
    }

    #[gpui::test]
    fn a_composition_undoes_in_one_step(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "hi ", window, cx);
                for step in ["n", "ni", "nih", "niha", "nihao"] {
                    field.replace_and_mark_text_in_range(None, step, None, window, cx);
                }
                field.replace_text_in_range(None, "你好", window, cx);
                assert_eq!(field.value(), "hi 你好");
                field.undo(&Undo, window, cx);
                assert_eq!(field.value(), "hi ");
            })
        });
    }

    #[gpui::test]
    fn an_edit_mid_composition_drops_its_mark(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "日本", window, cx);
                field.replace_and_mark_text_in_range(None, "nihao", None, window, cx);
                field.undo(&Undo, window, cx);
                assert!(field.marked_text_range(window, cx).is_none());
            })
        });
        // Drawing after the undo must not cut a run inside a character.
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    #[gpui::test]
    fn set_value_is_quiet_and_typing_is_not(cx: &mut TestAppContext) {
        let (field, events, cx) = form(cx);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        assert!(!events.borrow().contains(&InputEvent::Change));
        cx.simulate_input("d");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abcd");
        assert!(events.borrow().contains(&InputEvent::Change));
    }

    #[gpui::test]
    fn a_long_value_stays_on_one_line(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.simulate_input("a long line of text that is much wider than the field");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let (lines, scroll) = cx.read(|cx| {
            let state = field.read(cx);
            let drawn = state.drawn.as_ref().expect("drawn");
            (drawn.layout.line_layouts().len(), state.scroll_x)
        });
        assert_eq!(lines, 1);
        assert!(scroll > Pixels::ZERO, "the field scrolls to the caret");
    }

    #[gpui::test]
    fn a_read_only_field_selects_but_never_edits(cx: &mut TestAppContext) {
        let (field, events, cx) = form_with(cx, true);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        cx.run_until_parked();
        cx.simulate_input("d");
        cx.simulate_keystrokes("cmd-a");
        assert_eq!(cx.read(|cx| field.read(cx).core.selection.range()), 0..3);
        cx.simulate_keystrokes("backspace cmd-x cmd-v cmd-z");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        assert!(!events.borrow().contains(&InputEvent::Change));
        assert!(cx.update(|window, cx| field.read(cx).focus_handle(cx).is_focused(window)));
    }

    /// Right click, then a menu item, as a person would.
    fn menu_item(cx: &mut VisualTestContext, field: &Entity<InputState>, item: &'static str) {
        let bounds =
            cx.update(|_, cx| field.read(cx).drawn.as_ref().map(|drawn| drawn.text_bounds));
        let at = bounds.expect("the field is drawn").center();
        cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.run_until_parked();
        let item = cx.debug_bounds(item).expect("the menu shows the item");
        cx.simulate_click(item.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    /// The edit menu can't change a read-only field either: Cut and Paste
    /// are drawn disabled, and the state refuses every edit, whatever asks.
    #[gpui::test]
    fn a_read_only_fields_menu_never_cuts_or_pastes(cx: &mut TestAppContext) {
        let (field, events, cx) = form_with(cx, true);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
        cx.run_until_parked();
        menu_item(cx, &field, "edit-paste");
        cx.simulate_keystrokes("cmd-a");
        menu_item(cx, &field, "edit-cut");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        // Nor can a caller reach the state's edits around the menu.
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.paste(&Paste, window, cx);
                field.cut(&Cut, window, cx);
                field.replace_text_in_range(None, "x", window, cx);
                field.replace_and_mark_text_in_range(None, "y", None, window, cx);
            })
        });
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        assert!(!events.borrow().contains(&InputEvent::Change));
    }

    /// The public edits honour read-only too, and a composition range the
    /// platform hands a read-only field leaves no mark behind.
    #[gpui::test]
    fn a_read_only_field_refuses_its_public_edits(cx: &mut TestAppContext) {
        let (field, events, cx) = form_with(cx, true);
        cx.update(|window, cx| field.update(cx, |field, cx| field.set_value("abc", window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.insert("x", window, cx);
                field.replace_all("y", window, cx);
                field.clean(window, cx);
                field.replace_text_in_range(Some(0..1), "z", window, cx);
                assert!(field.marked_text_range(window, cx).is_none());
            })
        });
        assert_eq!(cx.read(|cx| field.read(cx).value()), "abc");
        assert!(!events.borrow().contains(&InputEvent::Change));
    }

    /// The same menu edits an editable field, so the test above reaches it.
    #[gpui::test]
    fn an_editable_fields_menu_pastes(cx: &mut TestAppContext) {
        let (field, _, cx) = form(cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
        menu_item(cx, &field, "edit-paste");
        assert_eq!(cx.read(|cx| field.read(cx).value()), "pasted");
    }
}
