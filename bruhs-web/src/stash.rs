//! Prompt stash (⌘S): park the composer draft to come back to
//! later. Newest first, capped, persisted as JSON under `bruhs.promptStash`.

use bruhs_models::logic::context_chip::{trim_with_chips, ContextChip};
use serde::{Deserialize, Serialize};

pub const STASH_MAX: usize = 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StashEntry {
    pub id: String,
    pub text: String,
    pub at_ms: f64,
    /// Chips whose labels sit in `text`. Absent in older stash JSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chips: Vec<ContextChip>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptStash {
    #[serde(default)]
    pub entries: Vec<StashEntry>,
}

impl PromptStash {
    /// Stash `text` (newest first); blank text is ignored. Over the cap the
    /// oldest entries fall off. Production passes chips through
    /// [`Self::push_with_chips`]; tests keep this text-only path.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "text-only stash entry; production passes chips, tests cover this path"
        )
    )]
    pub fn push(&mut self, text: &str, at_ms: f64) -> Option<String> {
        self.push_with_chips(text, &[], at_ms)
    }

    /// Stash `text` and the chips inside it. Blank text is ignored.
    pub fn push_with_chips(
        &mut self,
        text: &str,
        chips: &[ContextChip],
        at_ms: f64,
    ) -> Option<String> {
        let (text, chips) = trim_with_chips(text, chips);
        if text.is_empty() {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.entries.insert(
            0,
            StashEntry {
                id: id.clone(),
                text,
                at_ms,
                chips,
            },
        );
        self.entries.truncate(STASH_MAX);
        Some(id)
    }

    /// Remove and return the entry `id`.
    pub fn take(&mut self, id: &str) -> Option<StashEntry> {
        let pos = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(pos))
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.take(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "clippy::len_without_is_empty; only tests ask")
    )]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Put a stashed prompt back into the draft: replaces an empty draft,
/// otherwise appends after a blank line. Production joins chips with
/// `join_drafts`; tests keep this text-only path.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "text-only restore; production joins chips, tests cover this path"
    )
)]
pub fn restore_into_draft(draft: &str, text: &str) -> String {
    if draft.trim().is_empty() {
        text.to_string()
    } else {
        format!("{}\n\n{}", draft.trim_end(), text)
    }
}

/// First line of a stashed prompt, cut to `max` chars for menu rows.
pub fn stash_preview(text: &str, max: usize) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(max).collect();
    if first.chars().count() > max || text.lines().count() > 1 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_caps_at_twenty_dropping_oldest() {
        let mut s = PromptStash::default();
        for i in 0..25 {
            s.push(&format!("prompt {i}"), f64::from(i));
        }
        assert_eq!(s.len(), STASH_MAX);
        assert_eq!(s.entries[0].text, "prompt 24");
        assert_eq!(s.entries[STASH_MAX - 1].text, "prompt 5");
    }

    #[test]
    fn push_rejects_blank() {
        let mut s = PromptStash::default();
        assert_eq!(s.push("  \n ", 1.0), None);
        assert!(s.is_empty());
    }

    #[test]
    fn take_removes_and_returns() {
        let mut s = PromptStash::default();
        let id = s.push("keep me", 1.0).unwrap();
        s.push("other", 2.0);
        let e = s.take(&id).unwrap();
        assert_eq!(e.text, "keep me");
        assert_eq!(s.len(), 1);
        assert!(s.take(&id).is_none());
        assert!(!s.remove(&id));
    }

    #[test]
    fn restore_appends_after_blank_line() {
        assert_eq!(restore_into_draft("", "hello"), "hello");
        assert_eq!(restore_into_draft("   ", "hello"), "hello");
        assert_eq!(restore_into_draft("first\n", "second"), "first\n\nsecond");
    }

    #[test]
    fn preview_is_first_line_capped() {
        assert_eq!(stash_preview("one line", 80), "one line");
        assert_eq!(stash_preview("first\nsecond", 80), "first…");
        assert_eq!(stash_preview("abcdefghij", 4), "abcd…");
    }

    #[test]
    fn round_trips_through_json() {
        let mut s = PromptStash::default();
        s.push("x", 3.0);
        let json = serde_json::to_string(&s).unwrap();
        let back: PromptStash = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let legacy: PromptStash = serde_json::from_str("{}").unwrap();
        assert!(legacy.is_empty());
    }
}
