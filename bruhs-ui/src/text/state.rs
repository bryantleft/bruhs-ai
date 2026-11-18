//! A text view's source, its parsed document and its selection.

use gpui::{actions, App, ClipboardItem, Context, Global, KeyBinding, WeakEntity};

use super::document::{parse_html, parse_markdown, Document};

actions!(text_view, [CopySelection]);

/// What the source text is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFormat {
    Markdown,
    Html,
}

/// A position in a document: a segment and a byte offset in its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    pub segment: usize,
    pub offset: usize,
}

/// The source and parsed form of one text view, kept across frames.
pub struct TextViewState {
    format: TextFormat,
    source: String,
    document: Document,
    /// Where a selection was started and where it now ends.
    pub(super) selection: Option<(TextPosition, TextPosition)>,
    pub(super) selecting: bool,
}

impl TextViewState {
    pub fn markdown(source: &str, _cx: &mut Context<'_, Self>) -> Self {
        Self::new(TextFormat::Markdown, source)
    }

    pub fn html(source: &str, _cx: &mut Context<'_, Self>) -> Self {
        Self::new(TextFormat::Html, source)
    }

    fn new(format: TextFormat, source: &str) -> Self {
        Self {
            format,
            source: source.to_string(),
            document: parse(format, source),
            selection: None,
            selecting: false,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Show `source`. The whole text is parsed again: a streamed document
    /// always matches one parsed fresh, and parsing is cheap next to laying
    /// the text out.
    pub fn set_text(&mut self, source: &str, cx: &mut Context<'_, Self>) {
        if self.replace(source) {
            cx.notify();
        }
    }

    /// Append `more` to the source, as a stream does.
    pub fn push_str(&mut self, more: &str, cx: &mut Context<'_, Self>) {
        if more.is_empty() {
            return;
        }
        let source = format!("{}{more}", self.source);
        self.set_text(&source, cx);
    }

    /// [`Self::set_text`] without notifying, for a view syncing its own state
    /// while it renders. Returns whether anything changed.
    pub(super) fn replace(&mut self, source: &str) -> bool {
        if source == self.source {
            return false;
        }
        let appended = source.starts_with(self.source.as_str());
        self.source = source.to_string();
        self.document = parse(self.format, source);
        // A selection survives text appended after it; a replacement may
        // have moved everything, so it goes.
        if !appended {
            self.selection = None;
        }
        self.clamp_selection();
        true
    }

    fn clamp_selection(&mut self) {
        let segments = &self.document.segments;
        let valid = |pos: &TextPosition| {
            segments
                .get(pos.segment)
                .is_some_and(|text| pos.offset <= text.len() && text.is_char_boundary(pos.offset))
        };
        if let Some((anchor, head)) = &self.selection {
            if !valid(anchor) || !valid(head) {
                self.selection = None;
            }
        }
    }

    /// The selection, start first, when it covers any text.
    pub fn selected_range(&self) -> Option<(TextPosition, TextPosition)> {
        let (anchor, head) = self.selection?;
        let (start, end) = if anchor <= head {
            (anchor, head)
        } else {
            (head, anchor)
        };
        (start != end).then_some((start, end))
    }

    /// The selected text; blocks are separated by newlines.
    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.selected_range() else {
            return String::new();
        };
        let segments = &self.document.segments;
        let mut out = String::new();
        for segment in start.segment..=end.segment {
            let Some(text) = segments.get(segment) else {
                break;
            };
            let from = if segment == start.segment {
                start.offset
            } else {
                0
            };
            let to = if segment == end.segment {
                end.offset
            } else {
                text.len()
            };
            if segment != start.segment {
                out.push('\n');
            }
            // Drawn padding is not text.
            let pads = self
                .document
                .pads
                .get(segment)
                .map_or(&[][..], Vec::as_slice);
            let mut at = from;
            for pad in pads.iter().filter(|pad| pad.end > from && pad.start < to) {
                out.push_str(text.get(at..pad.start.max(at)).unwrap_or_default());
                at = pad.end.min(to);
            }
            out.push_str(text.get(at..to).unwrap_or_default());
        }
        out
    }

    pub fn clear_selection(&mut self, cx: &mut Context<'_, Self>) {
        if self.selection.take().is_some() {
            cx.notify();
        }
    }

    /// Whether `other` parsed to the same document.
    #[cfg(any(test, feature = "test-support"))]
    pub fn parsed_content_eq(&self, other: &Self) -> bool {
        self.document == other.document
    }
}

fn parse(format: TextFormat, source: &str) -> Document {
    match format {
        TextFormat::Markdown => parse_markdown(source),
        TextFormat::Html => parse_html(source),
    }
}

/// The one text view in the app whose selection is live: selecting in
/// another clears it, as a browser keeps one selection per page.
#[derive(Default)]
struct ActiveSelection(Option<WeakEntity<TextViewState>>);

impl Global for ActiveSelection {}

pub(super) fn make_active(state: &gpui::Entity<TextViewState>, cx: &mut App) {
    let previous = cx
        .try_global::<ActiveSelection>()
        .and_then(|active| active.0.clone());
    if let Some(previous) = previous.and_then(|weak| weak.upgrade()) {
        if previous != *state {
            previous.update(cx, |previous, cx| previous.clear_selection(cx));
        }
    }
    cx.set_global(ActiveSelection(Some(state.downgrade())));
}

/// The app's text selection.
pub struct TextSelection;

impl TextSelection {
    /// The text selected in any text view of the app, or an empty string.
    pub fn selected_text(_window: &mut gpui::Window, cx: &mut App) -> String {
        Self::current(cx)
    }

    fn current(cx: &App) -> String {
        cx.try_global::<ActiveSelection>()
            .and_then(|active| active.0.clone())
            .and_then(|weak| weak.upgrade())
            .map(|state| state.read(cx).selected_text())
            .unwrap_or_default()
    }
}

/// Bind ⌘C / Ctrl-C to copy the selected text. Fields that edit text bind
/// their own copy deeper in the key context, so theirs wins while one has
/// focus.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", CopySelection, Some("Root")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", CopySelection, Some("Root")),
    ]);
    cx.on_action(|_: &CopySelection, cx: &mut App| {
        let text = TextSelection::current(cx);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(source: &str) -> TextViewState {
        TextViewState::new(TextFormat::Markdown, source)
    }

    #[test]
    fn selected_text_spans_segments_with_newlines() {
        let mut state = state("first para\n\nsecond para");
        state.selection = Some((
            TextPosition {
                segment: 0,
                offset: 6,
            },
            TextPosition {
                segment: 1,
                offset: 6,
            },
        ));
        assert_eq!(state.selected_text(), "para\nsecond");
    }

    #[test]
    fn a_code_chip_copies_without_its_padding() {
        let mut state = state("run `cargo` now");
        let len = state.document.segments[0].len();
        state.selection = Some((
            TextPosition {
                segment: 0,
                offset: 0,
            },
            TextPosition {
                segment: 0,
                offset: len,
            },
        ));
        assert_eq!(state.selected_text(), "run cargo now");
    }

    #[test]
    fn appending_keeps_the_selection_and_replacing_drops_it() {
        let mut state = state("one");
        let sel = (
            TextPosition {
                segment: 0,
                offset: 0,
            },
            TextPosition {
                segment: 0,
                offset: 3,
            },
        );
        state.selection = Some(sel);
        state.replace("one two");
        assert_eq!(state.selection, Some(sel));
        state.replace("different");
        assert_eq!(state.selection, None);
    }

    #[test]
    fn a_streamed_document_matches_a_fresh_parse() {
        let text = "- a\n- b\n\n```rust\nfn x() {}\n```\n\n[l][r]\n\n[r]: /x\n";
        let mut streamed = state("");
        for (ix, _) in text.char_indices() {
            streamed.replace(&text[..ix]);
        }
        streamed.replace(text);
        assert_eq!(streamed.document, state(text).document);
    }
}
