//! Text editing without any UI: a string, a selection, motions and undo.
//!
//! Offsets are UTF-8 byte offsets that always sit on char boundaries.

use std::ops::Range;
use std::time::{Duration, Instant};

/// Typing within this long of the last edit undoes together.
const COALESCE: Duration = Duration::from_millis(1000);
const HISTORY_LIMIT: usize = 200;

/// A selection: `anchor` stays put while `head` (the caret) moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn reversed(&self) -> bool {
        self.head < self.anchor
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    text: String,
    selection: Selection,
}

/// The text being edited and its history.
#[derive(Clone, Debug, Default)]
pub struct EditCore {
    pub text: String,
    pub selection: Selection,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(Instant, EditKind)>,
    /// The last typed character was a space: the next word starts a new
    /// undo step.
    last_typed_space: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typing a character: consecutive ones undo together.
    Type,
    /// Anything else undoes on its own.
    Other,
}

impl EditCore {
    #[cfg(test)]
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            ..Self::default()
        }
    }

    /// Replace everything, forgetting history (the caller's `set_value`).
    pub fn reset(&mut self, text: &str, selection: Selection) {
        self.text = text.to_string();
        self.selection = self.clamp(selection);
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
    }

    pub fn clamp_offset(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub fn clamp(&self, selection: Selection) -> Selection {
        Selection {
            anchor: self.clamp_offset(selection.anchor),
            head: self.clamp_offset(selection.head),
        }
    }

    pub fn set_selection(&mut self, selection: Selection) {
        self.selection = self.clamp(selection);
        self.last_edit = None;
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.range()]
    }

    /// Replace `range` with `new`, leaving the caret after it. Returns the
    /// range the new text now covers, or `None` when nothing changed.
    pub fn replace(
        &mut self,
        range: Range<usize>,
        new: &str,
        kind: EditKind,
    ) -> Option<Range<usize>> {
        let range = self.clamp_offset(range.start)..self.clamp_offset(range.end);
        let range = range.start.min(range.end)..range.start.max(range.end);
        if self.text[range.clone()] == *new {
            return None;
        }
        let now = Instant::now();
        let space = new.contains(char::is_whitespace);
        // A word typed in one go undoes as one step; a space ends the step.
        let coalesce = kind == EditKind::Type
            && self.last_edit.is_some_and(|(at, last)| {
                last == EditKind::Type && now.duration_since(at) < COALESCE
            })
            && !space
            && !self.last_typed_space;
        self.last_typed_space = kind == EditKind::Type && space;
        if !coalesce {
            self.undo.push(Snapshot {
                text: self.text.clone(),
                selection: self.selection,
            });
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.text.replace_range(range.clone(), new);
        self.selection = Selection::caret(range.start + new.len());
        self.last_edit = Some((now, kind));
        Some(range.start..range.start + new.len())
    }

    /// [`Self::replace`] without an undo step: an IME composition updating
    /// text it already took a step for.
    pub fn replace_in_place(&mut self, range: Range<usize>, new: &str) -> Range<usize> {
        let range = self.clamp_offset(range.start)..self.clamp_offset(range.end);
        let range = range.start.min(range.end)..range.start.max(range.end);
        self.text.replace_range(range.clone(), new);
        self.selection = Selection::caret(range.start + new.len());
        self.last_edit = None;
        range.start..range.start + new.len()
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            text: std::mem::replace(&mut self.text, snapshot.text),
            selection: std::mem::replace(&mut self.selection, snapshot.selection),
        });
        self.last_edit = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            text: std::mem::replace(&mut self.text, snapshot.text),
            selection: std::mem::replace(&mut self.selection, snapshot.selection),
        });
        self.last_edit = None;
        true
    }

    pub fn prev_char(&self, offset: usize) -> usize {
        self.text[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(ix, _)| ix)
    }

    pub fn next_char(&self, offset: usize) -> usize {
        self.text[offset..]
            .chars()
            .next()
            .map_or(offset, |c| offset + c.len_utf8())
    }

    /// The start of the word before `offset` (skipping spaces first).
    pub fn prev_word(&self, offset: usize) -> usize {
        let before = &self.text[..offset];
        let mut chars = before.char_indices().rev().peekable();
        while chars.peek().is_some_and(|(_, c)| !is_word(*c)) {
            chars.next();
        }
        let mut start = chars.peek().map_or(0, |(ix, _)| *ix);
        for (ix, c) in chars {
            if !is_word(c) {
                break;
            }
            start = ix;
        }
        start
    }

    /// The end of the word after `offset` (skipping spaces first).
    pub fn next_word(&self, offset: usize) -> usize {
        let after = &self.text[offset..];
        let mut end = offset;
        let mut seen_word = false;
        for (ix, c) in after.char_indices() {
            if is_word(c) {
                seen_word = true;
            } else if seen_word {
                return offset + ix;
            }
            end = offset + ix + c.len_utf8();
        }
        end
    }

    /// The word around `offset`, or the run of other characters there.
    pub fn word_range(&self, offset: usize) -> Range<usize> {
        let class = |c: char| (is_word(c), c.is_whitespace());
        let at = self.text[offset..]
            .chars()
            .next()
            .or_else(|| self.text[..offset].chars().next_back());
        let Some(at) = at else {
            return offset..offset;
        };
        let target = class(at);
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| class(*c) == target)
            .last()
            .map_or(offset, |(ix, _)| ix);
        let end = self.text[offset..]
            .char_indices()
            .find(|(_, c)| class(*c) != target)
            .map_or(self.text.len(), |(ix, _)| offset + ix);
        start..end
    }

    /// The start of the logical line holding `offset`.
    pub fn line_start(&self, offset: usize) -> usize {
        self.text[..offset].rfind('\n').map_or(0, |ix| ix + 1)
    }

    /// The end of the logical line holding `offset` (before its newline).
    pub fn line_end(&self, offset: usize) -> usize {
        self.text[offset..]
            .find('\n')
            .map_or(self.text.len(), |ix| offset + ix)
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte offsets to UTF-16 offsets and back, for the platform's text input.
pub fn to_utf16(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].encode_utf16().count()
}

pub fn from_utf16(text: &str, offset16: usize) -> usize {
    let mut units = 0;
    for (ix, c) in text.char_indices() {
        if units >= offset16 {
            return ix;
        }
        units += c.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_coalesces_and_undo_redo_restore() {
        let mut core = EditCore::new("");
        for c in ["h", "e", "y"] {
            let at = core.selection.range();
            let _ = core.replace(at, c, EditKind::Type);
        }
        let at = core.selection.range();
        let _ = core.replace(at, " ", EditKind::Type);
        let at = core.selection.range();
        let _ = core.replace(at, "x", EditKind::Type);
        assert_eq!(core.text, "hey x");
        assert!(core.undo());
        assert_eq!(core.text, "hey ");
        assert!(core.undo());
        assert_eq!(core.text, "hey");
        assert!(core.undo());
        assert_eq!(core.text, "");
        assert!(core.redo());
        assert_eq!(core.text, "hey");
    }

    #[test]
    fn words_and_lines() {
        let core = EditCore::new("one two_x  three\nfour");
        assert_eq!(core.prev_word(8), 4);
        assert_eq!(core.next_word(3), 9);
        assert_eq!(core.word_range(5), 4..9);
        assert_eq!(core.line_start(18), 17);
        assert_eq!(core.line_end(2), 16);
    }

    #[test]
    fn offsets_stay_on_char_boundaries() {
        let mut core = EditCore::new("héllo");
        assert_eq!(core.clamp_offset(2), 1);
        assert_eq!(core.next_char(1), 3);
        assert_eq!(core.prev_char(3), 1);
        core.set_selection(Selection::caret(99));
        assert_eq!(core.selection.head, core.text.len());
        assert_eq!(to_utf16("a😀b", 5), 3);
        assert_eq!(from_utf16("a😀b", 3), 5);
    }
}
