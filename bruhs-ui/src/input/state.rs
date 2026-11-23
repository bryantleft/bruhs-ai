//! A text field's state: its text, selection, history and how it shows.

use std::marker::PhantomData;
use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, Pixels, Point, ScrollHandle, SharedString, Subscription, Task, UTF16Selection,
    Window,
};

use bruhs_models::{logic::composer_text::ComposerKey, num::ToFloatLossy as _};

use super::actions::*;
use super::code_fence::{self, Fence, Token};
use super::core::{from_utf16, to_utf16, EditCore, EditKind, Selection};
use super::markdown::{Mark, Projection};
use std::{cell::RefCell, rc::Rc};

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
    pub layout: super::text_layout::FieldLayout,
    /// The text's bounds as drawn (shifted by any horizontal scroll).
    pub text_bounds: Bounds<Pixels>,
    pub map: DisplayMap,
}

/// How the text shows: masked as bullets, with a ghost completion after
/// the caret.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DisplayMap {
    pub masked: bool,
    pub projection: Option<Rc<Projection>>,
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
            self.projection
                .as_ref()
                .map_or(offset, |p| p.to_display(offset))
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
            let mut ix = self
                .projection
                .as_ref()
                .map_or(display, |p| p.to_value(display))
                .min(self.value.len());
            while !self.value.is_char_boundary(ix) {
                ix -= 1;
            }
            ix
        }
    }
}

type CachedProjection = (String, Option<Range<usize>>, Rc<Projection>);

#[derive(Default)]
struct MarkdownEditor {
    projection: RefCell<Option<CachedProjection>>,
    pending: Vec<Mark>,
    syntax_source: Option<String>,
    colors: Vec<Token>,
    highlighting: Option<Task<()>>,
    picker: Option<LanguagePicker>,
    ime_inserted_newline: bool,
}

struct LanguagePicker {
    header: usize,
    query: gpui::Entity<InputState>,
    _subscription: Subscription,
}

/// The state of a text field.
pub struct EditorState<K: FieldKind> {
    pub(super) core: EditCore,
    markdown: Option<MarkdownEditor>,
    focus_handle: FocusHandle,
    pub(super) placeholder: SharedString,
    pub(super) masked: bool,
    pub(super) rows: usize,
    pub(super) soft_wrap: bool,
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
            markdown: None,
            focus_handle,
            placeholder: SharedString::default(),
            masked: false,
            rows: 1,
            soft_wrap: K::MULTI_LINE,
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

    /// Whether the active input method is composing marked text.
    pub fn is_composing(&self) -> bool {
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
        if let Some(editor) = &mut self.markdown {
            editor.pending.clear();
            editor.ime_inserted_newline = false;
        }
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
        if self.markdown_enabled() {
            self.commit_text(&text, EditKind::Other, window, cx);
            return;
        }
        let range = self.core.selection.range();
        if self.core.replace(range, &text, EditKind::Other).is_some() {
            self.after_edit(window, cx);
        }
    }

    fn apply_markdown_edit(
        &mut self,
        edit: super::markdown::MarkdownEdit,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let reversed = self.core.selection.reversed();
        let changed = self
            .core
            .replace(edit.range, &edit.text, EditKind::Other)
            .is_some();
        let (start, end) = (edit.selection.start, edit.selection.end);
        self.core.set_selection(if reversed {
            Selection {
                anchor: end,
                head: start,
            }
        } else {
            Selection {
                anchor: start,
                head: end,
            }
        });
        if changed {
            self.after_edit(window, cx);
        } else {
            self.reveal_caret(cx);
        }
    }

    /// Handle a structural editing key in rich Markdown before chat shortcuts.
    pub fn edit_markdown_key(
        &mut self,
        key: ComposerKey,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.try_markdown_key(key, false, window, cx)
    }

    fn try_markdown_key(
        &mut self,
        key: ComposerKey,
        enter_inserts_newline: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if key == ComposerKey::Backspace
            || !self.markdown_enabled()
            || self.readonly
            || self.marked.is_some()
        {
            return false;
        }
        let Some(projection) = self.display_map(window).projection else {
            return false;
        };
        let selection = self.core.selection.range();
        let source = &self.core.text;
        let edit = match key {
            ComposerKey::Enter | ComposerKey::ShiftEnter => {
                code_fence::newline(source, selection.clone(), &projection.fences).or_else(|| {
                    (key == ComposerKey::ShiftEnter || enter_inserts_newline)
                        .then(|| {
                            super::list::newline(source, selection, key == ComposerKey::ShiftEnter)
                        })
                        .flatten()
                })
            }
            ComposerKey::Tab | ComposerKey::ShiftTab => {
                let outdent = key == ComposerKey::ShiftTab;
                code_fence::indent(source, selection.clone(), &projection.fences, outdent)
                    .or_else(|| super::list::indent(source, selection, outdent))
            }
            ComposerKey::Backspace => None,
        };
        if let Some(edit) = edit {
            self.apply_markdown_edit(edit, window, cx);
            true
        } else {
            false
        }
    }

    fn at_empty_code_body(&self, range: &Range<usize>, window: &Window) -> bool {
        self.markdown_enabled()
            && range.is_empty()
            && self.display_map(window).projection.is_some_and(|p| {
                p.fences.iter().any(|f| {
                    f.body.is_empty() && !f.closing.is_empty() && f.body.start == range.start
                })
            })
    }

    /// Handle Enter after the composer has decided that this chord inserts a newline.
    pub fn insert_line_break(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !K::MULTI_LINE || self.readonly {
            return;
        }
        if self.try_markdown_key(ComposerKey::Enter, true, window, cx) {
            return;
        }
        self.type_text("\n", window, cx);
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
        let map = &drawn.map;
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
        if let Some(m) = &mut self.markdown {
            m.pending.clear();
        }
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

    pub(super) fn display_map(&self, window: &Window) -> DisplayMap {
        let ghost = self
            .inline_completion
            .as_ref()
            .filter(|_| self.core.selection.is_empty() && self.marked.is_none())
            .map(|ghost| (self.core.selection.head, ghost.len()));
        let projection = self
            .markdown
            .as_ref()
            .filter(|_| !self.masked)
            .map(|editor| {
                let selection = self
                    .focus_handle
                    .is_focused(window)
                    .then(|| self.core.selection.range());
                let mut cached = editor.projection.borrow_mut();
                if let Some((source, sel, projection)) = cached.as_ref() {
                    if source == &self.core.text && sel == &selection {
                        return projection.clone();
                    }
                }
                let projection = Rc::new(Projection::new(&self.core.text, selection.clone()));
                *cached = Some((self.core.text.clone(), selection, projection.clone()));
                projection
            });
        DisplayMap {
            projection,
            masked: self.masked,
            ghost,
            value: self.core.text.clone(),
        }
    }

    pub(super) fn prepare_fences(
        &mut self,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Fence> {
        if !self.markdown_enabled() {
            return Vec::new();
        }
        let Some(projection) = self.display_map(window).projection else {
            return Vec::new();
        };
        let fences = &projection.fences;
        let editor = self.markdown.as_mut().expect("rich editor");
        if fences.is_empty() {
            editor.syntax_source = None;
            editor.colors.clear();
            editor.highlighting = None;
            editor.picker = None;
            return Vec::new();
        }
        if editor.syntax_source.as_deref() != Some(&self.core.text) {
            editor.picker = None;
            editor.colors.clear();
            let source = self.core.text.clone();
            editor.syntax_source = Some(source.clone());
            let ranges = fences.clone();
            editor.highlighting = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let (source, colors) = cx
                    .background_spawn(async move {
                        let colors = code_fence::highlight(&source, &ranges);
                        (source, colors)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(editor) =
                        this.markdown.as_mut().filter(|_| this.core.text == source)
                    {
                        editor.colors = colors;
                        cx.notify();
                    }
                });
            }));
        }
        fences.clone()
    }

    pub(super) fn syntax_colors(&self) -> &[Token] {
        self.markdown
            .as_ref()
            .filter(|editor| editor.syntax_source.as_deref() == Some(&self.core.text))
            .map_or(&[], |editor| editor.colors.as_slice())
    }

    pub fn code_language_menu_open(&self) -> bool {
        self.markdown.as_ref().is_some_and(|e| e.picker.is_some())
    }

    pub(super) fn language_query(&self, header: usize) -> Option<&gpui::Entity<InputState>> {
        self.markdown
            .as_ref()?
            .picker
            .as_ref()
            .filter(|p| p.header == header)
            .map(|p| &p.query)
    }

    pub(super) fn toggle_language_menu(
        &mut self,
        header: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(editor) = &mut self.markdown else {
            return;
        };
        if self.readonly {
            return;
        }
        if !editor.picker.take().is_some_and(|p| p.header == header) {
            let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search languages"));
            let subscription = cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
            query.focus_handle(cx).focus(window, cx);
            editor.picker = Some(LanguagePicker {
                header,
                query,
                _subscription: subscription,
            });
        }
        cx.notify();
    }

    pub(super) fn choose_language(
        &mut self,
        header: usize,
        value: Option<&str>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.readonly || self.language_query(header).is_none() {
            return;
        }
        let Some(projection) = self.display_map(window).projection else {
            return;
        };
        let Some(fence) = projection.fences.iter().find(|f| f.header.start == header) else {
            return;
        };
        if let Some(markdown) = self.markdown.as_mut() {
            markdown.picker = None;
        }
        let value = value.unwrap_or(&self.core.text[fence.tag.clone()]);
        let (range, value) = fence.language_edit(&self.core.text, value);
        let value = value.to_owned();
        let selection = self.core.selection;
        if self
            .core
            .replace(range.clone(), &value, EditKind::Other)
            .is_some()
        {
            let reposition = |offset: usize| {
                if offset <= range.start {
                    offset
                } else if offset < range.end {
                    range.start + value.len()
                } else {
                    offset - range.len() + value.len()
                }
            };
            self.core.set_selection(Selection {
                anchor: reposition(selection.anchor),
                head: reposition(selection.head),
            });
            self.after_edit(window, cx);
        }
        // The dropdown restores its trigger focus when it handles selection.
        let focus = self.focus_handle.clone();
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        cx.notify();
    }

    /// Report an edit the person made.
    fn after_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(m) = &mut self.markdown {
            m.pending.clear();
            m.ime_inserted_newline = false;
        }
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

    pub fn markdown_enabled(&self) -> bool {
        self.markdown.is_some()
    }

    pub fn set_markdown_enabled(&mut self, enabled: bool, cx: &mut Context<'_, Self>) {
        if K::MULTI_LINE && enabled != self.markdown_enabled() {
            self.markdown = enabled.then(MarkdownEditor::default);
            self.drawn = None;
            cx.notify();
        }
    }

    pub(super) fn format_mark(
        &mut self,
        mark: Mark,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.readonly || self.marked.is_some() || !self.markdown_enabled() {
            return;
        }
        let selected = self.core.selection.range();
        let Some(projection) = self.display_map(window).projection else {
            return;
        };
        if projection
            .fences
            .iter()
            .any(|f| f.covers(&self.core.text, &selected))
        {
            return;
        }
        let existing = projection
            .marks
            .iter()
            .filter(|m| {
                m.kind == mark
                    && ((m.inner.start <= selected.start && selected.end <= m.inner.end)
                        || m.outer == selected)
            })
            .min_by_key(|m| m.outer.len());
        if selected.is_empty() {
            if let Some(m) = existing
                .filter(|m| selected.start == m.inner.end || selected.start == m.inner.start)
            {
                let outside = if selected.start == m.inner.end {
                    m.outer.end
                } else {
                    m.outer.start
                };
                self.move_to(outside, false, cx);
                return;
            }
            let Some(markdown) = self.markdown.as_mut() else {
                return;
            };
            let pending = &mut markdown.pending;
            if let Some(i) = pending.iter().position(|m| *m == mark) {
                pending.remove(i);
            } else {
                pending.push(mark);
            }
            return;
        }
        let reversed = self.core.selection.reversed();
        let (replace, text, after) = if let Some(m) = existing {
            let from = selected.start.max(m.inner.start);
            let to = selected.end.min(m.inner.end);
            let delimiter = &self.core.text[m.outer.start..m.inner.start];
            let mut result = String::new();
            if from > m.inner.start {
                result.push_str(&self.core.text[m.outer.start..from]);
                result.push_str(delimiter);
            }
            let start = m.outer.start + result.len();
            result.push_str(&self.core.text[from..to]);
            let end = m.outer.start + result.len();
            if to < m.inner.end {
                result.push_str(delimiter);
                result.push_str(&self.core.text[to..m.outer.end]);
            }
            (m.outer.clone(), result, start..end)
        } else {
            let delimiter = mark.delimiter();
            let start = selected.start + delimiter.len();
            let text = format!(
                "{delimiter}{}{delimiter}",
                &self.core.text[selected.clone()]
            );
            (selected.clone(), text, start..start + selected.len())
        };
        self.core.replace(replace, &text, EditKind::Other);
        let (anchor, head) = if reversed {
            (after.end, after.start)
        } else {
            (after.start, after.end)
        };
        self.core.set_selection(Selection { anchor, head });
        self.after_edit(window, cx);
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
        self.commit_text(&text, kind, window, cx);
    }

    // A composition in a previously empty block inserted the separator before its
    // closing fence. Cancelling that composition removes the separator too.
    fn cancel_fence_padding(&mut self, range: &mut Range<usize>) {
        if let Some(editor) = &mut self.markdown {
            if std::mem::take(&mut editor.ime_inserted_newline)
                && self.core.text.as_bytes().get(range.end) == Some(&b'\n')
            {
                range.end += 1;
            }
        }
    }

    fn commit_text(
        &mut self,
        text: &str,
        kind: EditKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let marked = self.marked.take();
        let composing = marked.is_some();
        let mut range = marked.unwrap_or_else(|| self.core.selection.range());
        if composing && text.is_empty() {
            self.cancel_fence_padding(&mut range);
        }
        let mut formatted = None;
        if !text.is_empty() && self.at_empty_code_body(&range, window) {
            formatted = Some((format!("{text}\n"), range.start + text.len()));
        } else if !text.is_empty() && self.has_pending_format() {
            let marks = self
                .markdown
                .as_mut()
                .map_or_else(Vec::new, |m| std::mem::take(&mut m.pending));
            let projection = self.display_map(window).projection.expect("rich input");
            let delimiters: Vec<_> = marks
                .iter()
                .map(|mark| {
                    projection
                        .marks
                        .iter()
                        .find(|m| m.kind == *mark && m.inner.contains(&range.start))
                        .map_or(mark.delimiter(), |m| {
                            &self.core.text[m.outer.start..m.inner.start]
                        })
                })
                .collect();
            let open = delimiters.concat();
            let close: String = delimiters.into_iter().rev().collect();
            let end = range.start + open.len() + text.len();
            formatted = Some((format!("{open}{text}{close}"), end));
        }

        let replacement = formatted.as_ref().map_or(text, |(value, _)| value.as_str());
        if composing {
            // Starting the IME composition already created its undo step.
            self.core.replace_in_place(range, replacement);
        } else {
            let kind = if formatted.is_some() {
                EditKind::Other
            } else {
                kind
            };
            if self.core.replace(range, replacement, kind).is_none() {
                return;
            }
        }
        if let Some((_, caret)) = formatted {
            self.core.set_selection(Selection::caret(caret));
        }
        self.after_edit(window, cx);
    }

    fn has_pending_format(&self) -> bool {
        self.markdown
            .as_ref()
            .is_some_and(|m| !m.pending.is_empty())
    }

    fn move_to(&mut self, offset: usize, select: bool, cx: &mut Context<'_, Self>) {
        if let Some(m) = &mut self.markdown {
            m.pending.clear();
        }
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
            self.commit_text(&text, EditKind::Other, window, cx);
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
            let key = if action.shift {
                ComposerKey::ShiftEnter
            } else {
                ComposerKey::Enter
            };
            if !self.try_markdown_key(key, true, window, cx) {
                self.type_text("\n", window, cx);
            }
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

    pub(super) fn tab(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.focus_handle.is_focused(window)
            || !self.edit_markdown_key(ComposerKey::Tab, window, cx)
        {
            cx.propagate();
        }
    }
    pub(super) fn shift_tab(
        &mut self,
        _: &OutdentInline,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.focus_handle.is_focused(window)
            || !self.edit_markdown_key(ComposerKey::ShiftTab, window, cx)
        {
            cx.propagate();
        }
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
        if let Some(editor) = &mut self.markdown {
            editor.pending.clear();
        }
        if click_count == 1 && !shift && !self.readonly {
            let task = self
                .drawn
                .as_ref()
                .filter(|d| d.map.value == self.core.text)
                .and_then(|d| d.map.projection.as_ref())
                .and_then(|p| {
                    p.tasks
                        .get(p.tasks.partition_point(|r| r.end < at))
                        .filter(|r| r.start <= at && at <= r.end)
                        .cloned()
                });
            if let Some(task) =
                task.filter(|r| self.range_to_bounds(r).is_some_and(|b| b.contains(&point)))
            {
                let selection = self.core.selection;
                let next = if self.core.text[task.clone()].eq_ignore_ascii_case("[x]") {
                    "[ ]"
                } else {
                    "[x]"
                };
                self.core.replace(task, next, EditKind::Other);
                self.core.set_selection(selection);
                self.after_edit(window, cx);
                return;
            }
        }
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
    /// Wrap long lines to the field width. Disable for horizontally scrollable source text.
    pub fn soft_wrap(mut self, enabled: bool) -> Self {
        self.soft_wrap = enabled;
        self
    }

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

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.has_pending_format() {
            if let Some(range) = self.marked.clone() {
                let text = self.core.text[range].to_owned();
                self.commit_text(&text, EditKind::Other, window, cx);
                return;
            }
        }
        if let Some(editor) = &mut self.markdown {
            editor.ime_inserted_newline = false;
        }
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
        // Some platform text services deliver Return as text directly.
        if range.is_none()
            && text == "\n"
            && self.try_markdown_key(ComposerKey::Enter, true, window, cx)
        {
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
        let mut range = range
            .map(|range| from_utf16(doc, range.start)..from_utf16(doc, range.end))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.core.selection.range());
        let new_text = Self::clean_text(new_text);
        if self.marked.is_some() && new_text.is_empty() {
            self.cancel_fence_padding(&mut range);
        }
        // A composition is one undo step: taken when it starts, then its
        // text is updated in place.
        let applied = if self.marked.is_none()
            && !new_text.is_empty()
            && self.at_empty_code_body(&range, window)
        {
            self.markdown
                .as_mut()
                .expect("empty rich code block")
                .ime_inserted_newline = true;
            let start = range.start;
            self.core
                .replace(range, &format!("{new_text}\n"), EditKind::Other);
            self.core
                .set_selection(Selection::caret(start + new_text.len()));
            start..start + new_text.len()
        } else if self.marked.is_some() {
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
        let map = &drawn.map;
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
            projection: None,
            masked: true,
            ghost: None,
            value: "héllo".into(),
        };
        assert_eq!(map.to_display(3), 2 * MASK.len_utf8());
        assert_eq!(map.to_value(2 * MASK.len_utf8()), 3);
        let ghost = DisplayMap {
            projection: None,
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
