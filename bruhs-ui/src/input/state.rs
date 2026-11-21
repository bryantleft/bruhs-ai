//! A text field's state: its text, selection, history and how it shows.

use std::marker::PhantomData;
use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    Pixels, Point, ScrollHandle, SharedString, Subscription, Task, TextLayout, UTF16Selection,
    Window,
};

use bruhs_models::num::ToFloatLossy as _;

use super::actions::*;
use super::core::{from_utf16, to_utf16, EditCore, EditKind, Selection};

const BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// Each masked character shows as a bullet.
pub(super) const MASK: char = '•';

/// What a field reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// The person changed the text (never [`EditorState::set_value`]).
    Change,
    PressEnter {
        secondary: bool,
        shift: bool,
    },
    Focus,
    Blur,
}

/// What a field holds, for platform autofill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputContentType {
    Password,
    NewPassword,
    OneTimeCode,
    Username,
    Email,
    Url,
    Name,
}

/// Single-line or multi-line.
pub trait FieldKind: 'static {
    const MULTI_LINE: bool;
}

pub struct SingleLine;
impl FieldKind for SingleLine {
    const MULTI_LINE: bool = false;
}

pub struct MultiLine;
impl FieldKind for MultiLine {
    const MULTI_LINE: bool = true;
}

/// A single-line field's state; draw it with [`super::Input`].
pub type InputState = EditorState<SingleLine>;
/// A multi-line field's state; draw it with [`super::Textarea`].
pub type TextareaState = EditorState<MultiLine>;

/// Where a frame drew the field's text, for the mouse and the platform.
#[derive(Clone)]
pub(super) struct Drawn {
    pub layout: TextLayout,
    /// The text's bounds as drawn (shifted by any horizontal scroll).
    pub text_bounds: Bounds<Pixels>,
    pub map: DisplayMap,
}

/// How the text shows: masked as bullets, with a ghost completion after
/// the caret.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DisplayMap {
    pub masked: bool,
    /// The value offset the ghost follows and its drawn length.
    pub ghost: Option<(usize, usize)>,
    pub value: String,
}

impl DisplayMap {
    fn base(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.value.len());
        while !self.value.is_char_boundary(offset) {
            offset -= 1;
        }
        if self.masked {
            self.value[..offset].chars().count() * MASK.len_utf8()
        } else {
            offset
        }
    }

    /// The drawn offset of a value offset. The caret sits before the ghost.
    pub fn to_display(&self, offset: usize) -> usize {
        let base = self.base(offset);
        match self.ghost {
            Some((at, len)) if offset > at => base + len,
            _ => base,
        }
    }

    /// The value offset of a drawn offset; inside the ghost it is the
    /// ghost's own position.
    pub fn to_value(&self, display: usize) -> usize {
        let display = match self.ghost {
            Some((at, len)) => {
                let at_display = self.base(at);
                if display <= at_display {
                    display
                } else if display < at_display + len {
                    return at;
                } else {
                    display - len
                }
            }
            None => display,
        };
        if self.masked {
            let chars = display / MASK.len_utf8();
            self.value
                .char_indices()
                .nth(chars)
                .map_or(self.value.len(), |(ix, _)| ix)
        } else {
            let mut ix = display.min(self.value.len());
            while !self.value.is_char_boundary(ix) {
                ix -= 1;
            }
            ix
        }
    }
}

/// The state of a text field.
pub struct EditorState<K: FieldKind> {
    pub(super) core: EditCore,
    focus_handle: FocusHandle,
    pub(super) placeholder: SharedString,
    pub(super) masked: bool,
    pub(super) rows: usize,
    pub(super) auto_grow: Option<(usize, usize)>,
    pub(super) submit_on_enter: bool,
    /// IME composition in progress.
    pub(super) marked: Option<Range<usize>>,
    pub(super) inline_completion: Option<SharedString>,
    pub(super) drawn: Option<Drawn>,
    /// Horizontal scroll of a single-line field.
    pub(super) scroll_x: Pixels,
    /// Vertical scroll of a multi-line field.
    pub(super) scroll: ScrollHandle,
    pub(super) selecting: bool,
    pub(super) caret_visible: bool,
    /// An edit or a caret move asks the next frame to bring the caret into
    /// view; a wheel scroll leaves it be.
    pub(super) reveal_pending: bool,
    /// Where a right click opened the edit menu.
    pub(super) context_menu: Option<Point<Pixels>>,
    /// Web `readonly`, as the component last drew it: every edit is
    /// refused here, whatever asks for it (a key, the edit menu, the
    /// platform's input handler, `insert`, `replace_all`, `clean`). Only
    /// `set_value`, the app's own value, still sets the text.
    pub(super) readonly: bool,
    blink: Option<Task<()>>,
    /// The caret's x on a vertical move, kept across lines.
    preferred_x: Option<Pixels>,
    _subscriptions: Vec<Subscription>,
    _kind: PhantomData<K>,
}

impl<K: FieldKind> EventEmitter<InputEvent> for EditorState<K> {}

impl<K: FieldKind> Focusable for EditorState<K> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<K: FieldKind> EditorState<K> {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        // A field is a tab stop, as web inputs are.
        let focus_handle = cx.focus_handle().tab_stop(true);
        super::register_field(&focus_handle, cx);
        let subscriptions = vec![
            cx.on_focus(&focus_handle, window, |this, _, cx| {
                this.start_blink(cx);
                cx.emit(InputEvent::Focus);
            }),
            cx.on_blur(&focus_handle, window, |this, _, cx| {
                this.blink = None;
                this.caret_visible = false;
                this.selecting = false;
                cx.emit(InputEvent::Blur);
                cx.notify();
            }),
        ];
        Self {
            core: EditCore::default(),
            focus_handle,
            placeholder: SharedString::default(),
            masked: false,
            rows: 1,
            auto_grow: None,
            submit_on_enter: false,
            marked: None,
            inline_completion: None,
            drawn: None,
            scroll_x: Pixels::ZERO,
            scroll: ScrollHandle::new(),
            selecting: false,
            caret_visible: false,
            reveal_pending: false,
            context_menu: None,
            readonly: false,
            blink: None,
            preferred_x: None,
            _subscriptions: subscriptions,
            _kind: PhantomData,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn default_value(mut self, value: impl Into<SharedString>) -> Self {
        let value: SharedString = value.into();
        let value = Self::clean_text(&value);
        let caret = if K::MULTI_LINE { 0 } else { value.len() };
        self.core.reset(&value, Selection::caret(caret));
        self
    }

    pub fn value(&self) -> SharedString {
        self.core.text.clone().into()
    }

    pub(crate) fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.core.text.is_empty()
    }

    /// Show `value`, forgetting history; reports no [`InputEvent::Change`].
    /// A single-line field puts the caret at the end, a multi-line one at
    /// the start, as `<input>` and `<textarea>` do.
    pub fn set_value(
        &mut self,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let value: SharedString = value.into();
        let value = Self::clean_text(&value);
        let caret = if K::MULTI_LINE { 0 } else { value.len() };
        self.core.reset(&value, Selection::caret(caret));
        self.marked = None;
        self.inline_completion = None;
        self.preferred_x = None;
        self.scroll_x = Pixels::ZERO;
        if K::MULTI_LINE {
            self.scroll.set_offset(Point::default());
        }
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.focus(&self.focus_handle, cx);
    }

    /// Clear the text, as the clear button does; reports a change. Does
    /// nothing in a read-only field.
    pub fn clean(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.readonly || self.core.text.is_empty() {
            return;
        }
        let len = self.core.text.len();
        if self.core.replace(0..len, "", EditKind::Other).is_some() {
            self.after_edit(window, cx);
        }
    }

    /// Insert `text` at the selection, as typing would: an undoable edit
    /// that reports a change and leaves the caret after it. Does nothing in
    /// a read-only field.
    pub fn insert(
        &mut self,
        text: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.readonly {
            return;
        }
        let text = Self::clean_text(text.as_ref());
        let range = self.core.selection.range();
        if self.core.replace(range, &text, EditKind::Other).is_some() {
            self.after_edit(window, cx);
        }
    }

    /// Replace the whole text as an edit (undoable, reported), unlike
    /// [`Self::set_value`]. Does nothing in a read-only field.
    pub fn replace_all(
        &mut self,
        text: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.readonly {
            return;
        }
        let text = Self::clean_text(text.as_ref());
        let len = self.core.text.len();
        if self.core.replace(0..len, &text, EditKind::Other).is_some() {
            self.after_edit(window, cx);
        }
    }

    /// Where the last frame drew the text in `range` (on its first line).
    pub fn range_to_bounds(&self, range: &Range<usize>) -> Option<Bounds<Pixels>> {
        let drawn = self.drawn.as_ref()?;
        let map = self.display_map();
        let start = map.to_display(range.start);
        let end = map.to_display(range.end);
        let from = drawn.layout.position_for_index(start)?;
        let to = drawn.layout.position_for_index(end).unwrap_or(from);
        let height = drawn.layout.line_height();
        Some(Bounds::from_corners(
            from,
            gpui::point(to.x.max(from.x), from.y + height),
        ))
    }

    /// The selection, start first, as byte offsets.
    pub fn selected_range(&self) -> Range<usize> {
        self.core.selection.range()
    }

    pub fn set_selected_range(&mut self, range: Range<usize>, cx: &mut Context<'_, Self>) {
        self.core.set_selection(Selection {
            anchor: range.start,
            head: range.end,
        });
        self.clear_inline_completion();
        self.preferred_x = None;
        self.reveal_caret(cx);
    }

    /// The caret, as a byte offset.
    pub fn cursor(&self) -> usize {
        self.core.selection.head
    }

    /// Ghost text drawn after the caret (an app's own suggestion). An edit
    /// or a caret move clears it.
    pub fn set_inline_completion_text(&mut self, text: Option<String>, cx: &mut Context<'_, Self>) {
        let text = text.filter(|text| !text.is_empty()).map(SharedString::from);
        if self.inline_completion != text {
            self.inline_completion = text;
            cx.notify();
        }
    }

    /// The ghost text showing now. A selection or an IME composition hides
    /// it, so it reads as none then.
    pub fn inline_completion_text(&self) -> Option<&str> {
        if self.marked.is_some() || !self.core.selection.is_empty() {
            return None;
        }
        self.inline_completion.as_deref()
    }

    fn clear_inline_completion(&mut self) {
        self.inline_completion = None;
    }

    /// A single-line field holds one line: pasted newlines become spaces.
    fn clean_text(text: &str) -> String {
        if K::MULTI_LINE {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.replace("\r\n", " ").replace(['\n', '\r'], " ")
        }
    }

    pub(super) fn display_map(&self) -> DisplayMap {
        let ghost = self
            .inline_completion
            .as_ref()
            .filter(|_| self.core.selection.is_empty() && self.marked.is_none())
            .map(|ghost| (self.core.selection.head, ghost.len()));
        DisplayMap {
            masked: self.masked,
            ghost,
            value: self.core.text.clone(),
        }
    }

    /// Report an edit the person made.
    fn after_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Any edit that is not the composition's own ends it: its range
        // would point into text that has changed.
        self.marked = None;
        self.clear_inline_completion();
        self.preferred_x = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.emit(InputEvent::Change);
        self.reveal_caret(cx);
    }

    /// Show the caret steadily (a blink restarts after an edit or move).
    fn reveal_caret(&mut self, cx: &mut Context<'_, Self>) {
        self.caret_visible = true;
        self.reveal_pending = true;
        if self.blink.is_some() {
            self.start_blink(cx);
        }
        cx.notify();
    }

    fn start_blink(&mut self, cx: &mut Context<'_, Self>) {
        self.caret_visible = true;
        self.blink = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(BLINK_INTERVAL).await;
            let alive = this.update(cx, |this, cx| {
                this.caret_visible = !this.caret_visible;
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        }));
        cx.notify();
    }

    fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.readonly {
            return;
        }
        let text = Self::clean_text(text);
        let kind = if text.chars().count() == 1 {
            EditKind::Type
        } else {
            EditKind::Other
        };
        // Committing a composition replaces its text in place: the step was
        // taken when the composition started.
        if let Some(marked) = self.marked.take() {
            self.core.replace_in_place(marked, &text);
            self.after_edit(window, cx);
            return;
        }
        let range = self.core.selection.range();
        if self.core.replace(range, &text, kind).is_some() {
            self.after_edit(window, cx);
        }
    }

    fn move_to(&mut self, offset: usize, select: bool, cx: &mut Context<'_, Self>) {
        let offset = self.core.clamp_offset(offset);
        let selection = if select {
            Selection {
                anchor: self.core.selection.anchor,
                head: offset,
            }
        } else {
            Selection::caret(offset)
        };
        self.core.set_selection(selection);
        self.clear_inline_completion();
        self.reveal_caret(cx);
    }

    fn delete_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if range.is_empty() || self.readonly {
            return;
        }
        if self.core.replace(range, "", EditKind::Other).is_some() {
            self.after_edit(window, cx);
        }
    }

    /// The range a deletion covers: the selection, or `extent` from the caret.
    fn deletion(&self, extent: impl FnOnce(&EditCore, usize) -> usize) -> Range<usize> {
        let selection = self.core.selection;
        if !selection.is_empty() {
            return selection.range();
        }
        let to = extent(&self.core, selection.head);
        selection.head.min(to)..selection.head.max(to)
    }

    pub(super) fn backspace(
        &mut self,
        _: &Backspace,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = self.deletion(|core, at| core.prev_char(at));
        self.delete_range(range, window, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<'_, Self>) {
        let range = self.deletion(|core, at| core.next_char(at));
        self.delete_range(range, window, cx);
    }

    pub(super) fn delete_to_beginning_of_line(
        &mut self,
        _: &DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = self.deletion(|core, at| core.line_start(at));
        self.delete_range(range, window, cx);
    }

    pub(super) fn delete_to_end_of_line(
        &mut self,
        _: &DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = self.deletion(|core, at| core.line_end(at));
        self.delete_range(range, window, cx);
    }

    pub(super) fn delete_to_previous_word_start(
        &mut self,
        _: &DeleteToPreviousWordStart,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = self.deletion(|core, at| core.prev_word(at));
        self.delete_range(range, window, cx);
    }

    pub(super) fn delete_to_next_word_end(
        &mut self,
        _: &DeleteToNextWordEnd,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = self.deletion(|core, at| core.next_word(at));
        self.delete_range(range, window, cx);
    }

    pub(super) fn left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<'_, Self>) {
        let selection = self.core.selection;
        let to = if selection.is_empty() {
            self.core.prev_char(selection.head)
        } else {
            selection.range().start
        };
        self.preferred_x = None;
        self.move_to(to, false, cx);
    }

    pub(super) fn right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<'_, Self>) {
        let selection = self.core.selection;
        // At the end with a ghost showing, → accepts nothing: the app decides
        // (its own Tab binding); the caret just stays.
        let to = if selection.is_empty() {
            self.core.next_char(selection.head)
        } else {
            selection.range().end
        };
        self.preferred_x = None;
        self.move_to(to, false, cx);
    }

    pub(super) fn select_left(
        &mut self,
        _: &SelectLeft,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = self.core.prev_char(self.core.selection.head);
        self.preferred_x = None;
        self.move_to(to, true, cx);
    }

    pub(super) fn select_right(
        &mut self,
        _: &SelectRight,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = self.core.next_char(self.core.selection.head);
        self.preferred_x = None;
        self.move_to(to, true, cx);
    }

    /// The caret offset one visual line up or down, keeping its x.
    fn vertical(&mut self, lines: i32) -> Option<usize> {
        let drawn = self.drawn.as_ref()?;
        let head = drawn.map.to_display(self.core.selection.head);
        let point = drawn.layout.position_for_index(head)?;
        let line_height = drawn.layout.line_height();
        let x = *self.preferred_x.get_or_insert(point.x);
        let y = point.y + line_height * lines.to_f32_lossy() + line_height / 2.0;
        let bounds = drawn.text_bounds;
        if y < bounds.top() {
            return Some(0);
        }
        if y > bounds.bottom() {
            return Some(self.core.text.len());
        }
        let ix = match drawn.layout.index_for_position(gpui::point(x, y)) {
            Ok(ix) | Err(ix) => ix,
        };
        Some(drawn.map.to_value(ix))
    }

    pub(super) fn up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<'_, Self>) {
        // A single-line field has no lines to move between: the key goes on
        // to what holds the field (a list it filters).
        if !K::MULTI_LINE {
            cx.propagate();
            return;
        }
        if let Some(to) = self.vertical(-1) {
            let keep = self.preferred_x;
            self.move_to(to, false, cx);
            self.preferred_x = keep;
        }
    }

    pub(super) fn down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<'_, Self>) {
        if !K::MULTI_LINE {
            cx.propagate();
            return;
        }
        if let Some(to) = self.vertical(1) {
            let keep = self.preferred_x;
            self.move_to(to, false, cx);
            self.preferred_x = keep;
        }
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<'_, Self>) {
        let to = if K::MULTI_LINE {
            self.vertical(-1)
        } else {
            Some(0)
        };
        if let Some(to) = to {
            let keep = self.preferred_x;
            self.move_to(to, true, cx);
            self.preferred_x = keep;
        }
    }

    pub(super) fn select_down(
        &mut self,
        _: &SelectDown,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = if K::MULTI_LINE {
            self.vertical(1)
        } else {
            Some(self.core.text.len())
        };
        if let Some(to) = to {
            let keep = self.preferred_x;
            self.move_to(to, true, cx);
            self.preferred_x = keep;
        }
    }

    pub(super) fn home(&mut self, _: &MoveHome, _: &mut Window, cx: &mut Context<'_, Self>) {
        let to = self.core.line_start(self.core.selection.head);
        self.move_to(to, false, cx);
    }

    pub(super) fn end(&mut self, _: &MoveEnd, _: &mut Window, cx: &mut Context<'_, Self>) {
        let to = self.core.line_end(self.core.selection.head);
        self.move_to(to, false, cx);
    }

    pub(super) fn move_to_start(
        &mut self,
        _: &MoveToStart,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.move_to(0, false, cx);
    }

    pub(super) fn move_to_end(
        &mut self,
        _: &MoveToEnd,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let end = self.core.text.len();
        self.move_to(end, false, cx);
    }

    pub(super) fn previous_word(
        &mut self,
        _: &MoveToPreviousWord,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = if self.masked {
            0
        } else {
            self.core.prev_word(self.core.selection.head)
        };
        self.move_to(to, false, cx);
    }

    pub(super) fn next_word(
        &mut self,
        _: &MoveToNextWord,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = if self.masked {
            self.core.text.len()
        } else {
            self.core.next_word(self.core.selection.head)
        };
        self.move_to(to, false, cx);
    }

    pub(super) fn select_to_start_of_line(
        &mut self,
        _: &SelectToStartOfLine,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = self.core.line_start(self.core.selection.head);
        self.move_to(to, true, cx);
    }

    pub(super) fn select_to_end_of_line(
        &mut self,
        _: &SelectToEndOfLine,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = self.core.line_end(self.core.selection.head);
        self.move_to(to, true, cx);
    }

    pub(super) fn select_to_start(
        &mut self,
        _: &SelectToStart,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.move_to(0, true, cx);
    }

    pub(super) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let end = self.core.text.len();
        self.move_to(end, true, cx);
    }

    pub(super) fn select_to_previous_word_start(
        &mut self,
        _: &SelectToPreviousWordStart,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = self.core.prev_word(self.core.selection.head);
        self.move_to(to, true, cx);
    }

    pub(super) fn select_to_next_word_end(
        &mut self,
        _: &SelectToNextWordEnd,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let to = self.core.next_word(self.core.selection.head);
        self.move_to(to, true, cx);
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<'_, Self>) {
        let len = self.core.text.len();
        self.core.set_selection(Selection {
            anchor: 0,
            head: len,
        });
        self.clear_inline_completion();
        self.reveal_caret(cx);
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<'_, Self>) {
        // Nothing selected here: the copy is for a text selection elsewhere
        // (a reply the person selected while this field kept focus).
        if self.core.selection.is_empty() {
            cx.propagate();
            return;
        }
        // A masked field never hands its secret to the clipboard.
        if self.masked {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.core.selected_text().to_string(),
        ));
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.masked || self.readonly || self.core.selection.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.core.selected_text().to_string(),
        ));
        let range = self.core.selection.range();
        self.delete_range(range, window, cx);
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.readonly {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = Self::clean_text(&text);
            let range = self.core.selection.range();
            if self.core.replace(range, &text, EditKind::Other).is_some() {
                self.after_edit(window, cx);
            }
        }
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.readonly && self.core.undo() {
            self.after_edit(window, cx);
        }
    }

    pub(super) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.readonly && self.core.redo() {
            self.after_edit(window, cx);
        }
    }

    pub(super) fn enter(
        &mut self,
        action: &Enter,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.clear_inline_completion();
        let newline = K::MULTI_LINE && !action.secondary && (!self.submit_on_enter || action.shift);
        if newline {
            self.type_text("\n", window, cx);
        } else {
            // A submit: whoever holds the field (a form, a chat) acts on it.
            cx.propagate();
        }
        cx.emit(InputEvent::PressEnter {
            secondary: action.secondary,
            shift: action.shift,
        });
    }

    pub(super) fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.inline_completion_text().is_some() {
            self.inline_completion = None;
            cx.notify();
            return;
        }
        if self.marked.take().is_some() {
            window.notify_text_input_changed(&self.focus_handle);
            cx.notify();
        }
        cx.propagate();
    }

    pub(super) fn character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<'_, Self>,
    ) {
        window.show_character_palette();
    }

    /// The caret a visible page up or down; one line goes on to the holder.
    fn page(&mut self, direction: i32, cx: &mut Context<'_, Self>) {
        if !K::MULTI_LINE {
            cx.propagate();
            return;
        }
        let lines = self.drawn.as_ref().map_or(1, |drawn| {
            let visible = self
                .scroll
                .bounds()
                .size
                .height
                .max(drawn.layout.line_height());
            let count = (visible / drawn.layout.line_height()).floor();
            // Whole lines, at least one; a page is never huge.
            (1..=200)
                .rev()
                .find(|n| n.to_f32_lossy() <= count)
                .unwrap_or(1)
        });
        if let Some(to) = self.vertical(direction * lines) {
            let keep = self.preferred_x;
            self.move_to(to, false, cx);
            self.preferred_x = keep;
        }
    }

    pub(super) fn page_up(&mut self, _: &MovePageUp, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.page(-1, cx);
    }

    pub(super) fn page_down(
        &mut self,
        _: &MovePageDown,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.page(1, cx);
    }

    /// Tab never edits a field: focus moves on, like a web field's.
    pub(super) fn tab(&mut self, _: &IndentInline, _: &mut Window, cx: &mut Context<'_, Self>) {
        cx.propagate();
    }

    pub(super) fn shift_tab(
        &mut self,
        _: &OutdentInline,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        cx.propagate();
    }

    /// The value offset under `point`, from the last frame's text.
    pub(super) fn offset_at(&self, point: Point<Pixels>) -> Option<usize> {
        let drawn = self.drawn.as_ref()?;
        let bounds = drawn.text_bounds;
        let clamped = gpui::point(
            point.x.max(bounds.left()).min(bounds.right()),
            point
                .y
                .min(bounds.bottom() - gpui::px(1.0))
                .max(bounds.top()),
        );
        let ix = match drawn.layout.index_for_position(clamped) {
            Ok(ix) | Err(ix) => ix,
        };
        Some(self.core.clamp_offset(drawn.map.to_value(ix)))
    }

    pub(super) fn mouse_down(
        &mut self,
        point: Point<Pixels>,
        click_count: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(at) = self.offset_at(point) else {
            return;
        };
        let selection = match click_count {
            // A masked field is one word: its spaces stay hidden.
            2 if self.masked => Selection {
                anchor: 0,
                head: self.core.text.len(),
            },
            2 => {
                let word = self.core.word_range(at);
                Selection {
                    anchor: word.start,
                    head: word.end,
                }
            }
            n if n >= 3 => Selection {
                anchor: self.core.line_start(at),
                head: self.core.line_end(at),
            },
            _ if shift => Selection {
                anchor: self.core.selection.anchor,
                head: at,
            },
            _ => Selection::caret(at),
        };
        self.core.set_selection(selection);
        self.selecting = true;
        self.preferred_x = None;
        self.clear_inline_completion();
        self.reveal_caret(cx);
    }

    pub(super) fn mouse_drag(&mut self, point: Point<Pixels>, cx: &mut Context<'_, Self>) {
        if !self.selecting {
            return;
        }
        if let Some(at) = self.offset_at(point) {
            if at != self.core.selection.head {
                let anchor = self.core.selection.anchor;
                self.core.set_selection(Selection { anchor, head: at });
                cx.notify();
            }
        }
    }

    pub(super) fn mouse_up(&mut self) {
        self.selecting = false;
    }

    /// A right click: the caret moves there unless it lands in the
    /// selection, and the edit menu opens.
    pub(super) fn context_click(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if let Some(at) = self.offset_at(point) {
            if !self.core.selection.range().contains(&at) {
                self.core.set_selection(Selection::caret(at));
            }
        }
        self.context_menu = Some(point);
        cx.notify();
    }

    pub(super) fn close_context_menu(&mut self, cx: &mut Context<'_, Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// A click in a text area's empty space below its text focuses it with
    /// the caret at the end, as a web textarea does.
    pub(super) fn click_below_text(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let below = self
            .drawn
            .as_ref()
            .is_some_and(|drawn| point.y > drawn.text_bounds.bottom());
        if !below {
            return;
        }
        window.focus(&self.focus_handle, cx);
        let end = self.core.text.len();
        self.move_to(end, false, cx);
    }

    /// Whether the edit menu can cut or copy: something is selected and the
    /// field is not masked.
    pub(super) fn can_copy(&self) -> bool {
        !self.core.selection.is_empty() && !self.masked
    }
}

impl InputState {
    /// Show bullets in place of the text (a password).
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    pub fn set_masked(&mut self, masked: bool, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.masked = masked;
        cx.notify();
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }
}

impl TextareaState {
    /// The field's height, in lines.
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows.max(1);
        self.auto_grow = None;
        self
    }

    /// Grow with the text from `min` to `max` lines, then scroll.
    pub fn auto_grow(mut self, min: usize, max: usize) -> Self {
        let min = min.max(1);
        self.auto_grow = Some((min, max.max(min)));
        self.rows = min;
        self
    }

    /// Enter submits ([`InputEvent::PressEnter`]); Shift-Enter starts a
    /// line.
    pub fn submit_on_enter(mut self, submit: bool) -> Self {
        self.submit_on_enter = submit;
        self
    }
}

impl<K: FieldKind> EntityInputHandler for EditorState<K> {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<String> {
        let text = &self.core.text;
        let range = from_utf16(text, range.start)..from_utf16(text, range.end);
        *adjusted = Some(to_utf16(text, range.start)..to_utf16(text, range.end));
        Some(text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<UTF16Selection> {
        let selection = self.core.selection;
        let range = selection.range();
        Some(UTF16Selection {
            range: to_utf16(&self.core.text, range.start)..to_utf16(&self.core.text, range.end),
            reversed: selection.reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<Range<usize>> {
        let marked = self.marked.clone()?;
        Some(to_utf16(&self.core.text, marked.start)..to_utf16(&self.core.text, marked.end))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.readonly {
            // Nothing is composed in a field that can't change.
            self.marked = None;
            return;
        }
        if let Some(range) = range {
            let doc = &self.core.text;
            let range = from_utf16(doc, range.start)..from_utf16(doc, range.end);
            self.marked = Some(range);
        }
        self.type_text(text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.readonly {
            return;
        }
        let doc = &self.core.text;
        let range = range
            .map(|range| from_utf16(doc, range.start)..from_utf16(doc, range.end))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.core.selection.range());
        let new_text = Self::clean_text(new_text);
        // A composition is one undo step: taken when it starts, then its
        // text is updated in place.
        let applied = if self.marked.is_some() {
            self.core.replace_in_place(range, &new_text)
        } else {
            let start = self.core.clamp_offset(range.start.min(range.end));
            self.core
                .replace(range, &new_text, EditKind::Other)
                .unwrap_or(start..start + new_text.len())
        };
        self.marked = (!new_text.is_empty()).then(|| applied.clone());
        if let Some(selected) = new_selected {
            let start = applied.start + from_utf16(&new_text, selected.start);
            let end = applied.start + from_utf16(&new_text, selected.end);
            self.core.set_selection(Selection {
                anchor: start,
                head: end,
            });
        }
        self.clear_inline_completion();
        self.preferred_x = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.emit(InputEvent::Change);
        self.reveal_caret(cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        let drawn = self.drawn.as_ref()?;
        let text = &self.core.text;
        let map = self.display_map();
        let start = map.to_display(from_utf16(text, range.start));
        let end = map.to_display(from_utf16(text, range.end));
        let line_height = drawn.layout.line_height();
        let from = drawn.layout.position_for_index(start)?;
        let to = drawn.layout.position_for_index(end).unwrap_or(from);
        Some(Bounds::from_corners(
            from,
            gpui::point(to.x.max(from.x), from.y + line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<usize> {
        let at = self.offset_at(point)?;
        Some(to_utf16(&self.core.text, at))
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = &self.core.text;
        let range = from_utf16(text, range.start)..from_utf16(text, range.end);
        self.set_selected_range(range, cx);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<usize> {
        Some(to_utf16(&self.core.text, self.core.text.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_masked_ghosted_map_round_trips() {
        let map = DisplayMap {
            masked: true,
            ghost: None,
            value: "héllo".into(),
        };
        assert_eq!(map.to_display(3), 2 * MASK.len_utf8());
        assert_eq!(map.to_value(2 * MASK.len_utf8()), 3);
        let ghost = DisplayMap {
            masked: false,
            ghost: Some((3, 4)),
            value: "abcdef".into(),
        };
        assert_eq!(ghost.to_display(3), 3);
        assert_eq!(ghost.to_display(4), 8);
        assert_eq!(ghost.to_value(5), 3);
        assert_eq!(ghost.to_value(8), 4);
    }
}
