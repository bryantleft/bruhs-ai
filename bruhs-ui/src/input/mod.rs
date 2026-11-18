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
