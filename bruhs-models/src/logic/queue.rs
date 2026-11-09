//! Prompts typed while a turn is running. They wait here and send when the
//! thread is idle. Web stores the value as `bruhs.promptQueue`. Desktop stores
//! each thread's rows on that thread.

use serde::{Deserialize, Serialize};

use super::context_chip::{trim_with_chips, ContextChip};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedPrompt {
    pub id: String,
    pub thread_id: String,
    pub text: String,
    pub at_ms: f64,
    /// Inline chips whose labels sit in `text`. Empty for a text-only row
    /// so older `bruhs.promptQueue` JSON still loads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chips: Vec<ContextChip>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptQueue(Vec<QueuedPrompt>);

impl PromptQueue {
    pub fn from_items(items: Vec<QueuedPrompt>) -> Self {
        Self(items)
    }

    /// Queue `text` for `thread_id`. Blank text is ignored.
    pub fn push(&mut self, thread_id: &str, text: &str, at_ms: f64) -> Option<String> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        self.push_with_chips(thread_id, text, &[], at_ms)
    }

    /// Queue `text` and the chips whose labels it contains.
    pub fn push_with_chips(
        &mut self,
        thread_id: &str,
        text: &str,
        chips: &[ContextChip],
        at_ms: f64,
    ) -> Option<String> {
        let (text, chips) = trim_with_chips(text, chips);
        if text.is_empty() {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.0.push(QueuedPrompt {
            id: id.clone(),
            thread_id: thread_id.to_string(),
            text,
            at_ms,
            chips,
        });
        Some(id)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|prompt| prompt.id != id);
        before != self.0.len()
    }

    /// Replace the text of one queued message. Blank text is left unchanged.
    pub fn set_text(&mut self, id: &str, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let Some(prompt) = self.0.iter_mut().find(|prompt| prompt.id == id) else {
            return false;
        };
        prompt.text = text.to_string();
        // A text-only edit has no sidecar, so the previous ranges would
        // point at the wrong words.
        prompt.chips.clear();
        true
    }

    /// Replace one queued message's text and the chips inside it.
    pub fn set_body(&mut self, id: &str, text: &str, chips: Vec<ContextChip>) -> bool {
        let (text, chips) = trim_with_chips(text, &chips);
        if text.is_empty() {
            return false;
        }
        let Some(prompt) = self.0.iter_mut().find(|prompt| prompt.id == id) else {
            return false;
        };
        prompt.text = text;
        prompt.chips = chips;
        true
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.iter().any(|prompt| prompt.id == id)
    }

    pub fn text_of(&self, id: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|prompt| prompt.id == id)
            .map(|prompt| prompt.text.as_str())
    }

    /// Pop the oldest prompt queued for `thread_id`.
    pub fn next_for(&mut self, thread_id: &str) -> Option<QueuedPrompt> {
        let pos = self
            .0
            .iter()
            .position(|prompt| prompt.thread_id == thread_id)?;
        Some(self.0.remove(pos))
    }

    pub fn for_thread(&self, thread_id: &str) -> Vec<QueuedPrompt> {
        self.0
            .iter()
            .filter(|prompt| prompt.thread_id == thread_id)
            .cloned()
            .collect()
    }

    pub fn latest_for(&self, thread_id: &str) -> Option<QueuedPrompt> {
        self.0
            .iter()
            .rev()
            .find(|prompt| prompt.thread_id == thread_id)
            .cloned()
    }

    pub fn has_for(&self, thread_id: &str) -> bool {
        self.0.iter().any(|prompt| prompt.thread_id == thread_id)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Move `id` one step earlier or later among the same thread's messages.
    pub fn nudge(&mut self, id: &str, earlier: bool) -> bool {
        let Some(pair) = self.neighbor(id, earlier) else {
            return false;
        };
        self.0.swap(pair.0, pair.1);
        true
    }

    pub fn can_nudge(&self, id: &str, earlier: bool) -> bool {
        self.neighbor(id, earlier).is_some()
    }

    /// Place `id` immediately before `before_id` when both belong to one thread.
    pub fn move_before(&mut self, id: &str, before_id: &str) -> bool {
        if id == before_id {
            return false;
        }
        let Some(from) = self.0.iter().position(|prompt| prompt.id == id) else {
            return false;
        };
        let Some(target) = self.0.iter().position(|prompt| prompt.id == before_id) else {
            return false;
        };
        if self.0[from].thread_id != self.0[target].thread_id {
            return false;
        }
        let item = self.0.remove(from);
        let Some(insert_at) = self.0.iter().position(|prompt| prompt.id == before_id) else {
            self.0.insert(from, item);
            return false;
        };
        self.0.insert(insert_at, item);
        true
    }

    fn neighbor(&self, id: &str, earlier: bool) -> Option<(usize, usize)> {
        let idx = self.0.iter().position(|prompt| prompt.id == id)?;
        let thread = self.0[idx].thread_id.as_str();
        let other = if earlier {
            self.0[..idx]
                .iter()
                .rposition(|prompt| prompt.thread_id == thread)?
        } else {
            let offset = self.0[idx + 1..]
                .iter()
                .position(|prompt| prompt.thread_id == thread)?;
            idx + 1 + offset
        };
        Some((idx, other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_thread_check_does_not_match_another_threads_prompt() {
        let mut queue = PromptQueue::default();
        queue.push("one", "Pending", 1.0);
        assert!(queue.has_for("one"));
        assert!(!queue.has_for("two"));
    }

    #[test]
    fn push_ignores_blank() {
        let mut queue = PromptQueue::default();
        assert_eq!(queue.push("t1", "   ", 1.0), None);
        assert!(queue.is_empty());
        assert!(queue.push("t1", "  hello ", 1.0).is_some());
        assert_eq!(queue.for_thread("t1")[0].text, "hello");
    }

    #[test]
    fn next_for_pops_oldest_of_that_thread_only() {
        let mut queue = PromptQueue::default();
        queue.push("t1", "one", 1.0);
        queue.push("t2", "other", 2.0);
        queue.push("t1", "two", 3.0);
        assert_eq!(queue.next_for("t1").unwrap().text, "one");
        assert_eq!(queue.next_for("t1").unwrap().text, "two");
        assert_eq!(queue.next_for("t1"), None);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next_for("t2").unwrap().text, "other");
    }

    #[test]
    fn remove_by_id() {
        let mut queue = PromptQueue::default();
        let id = queue.push("t1", "one", 1.0).unwrap();
        queue.push("t1", "two", 2.0);
        assert!(queue.remove(&id));
        assert!(!queue.remove(&id));
        assert_eq!(queue.for_thread("t1").len(), 1);
    }

    #[test]
    fn round_trips_through_json() {
        let mut queue = PromptQueue::default();
        queue.push("t1", "persist me", 5.0);
        let json = serde_json::to_string(&queue).unwrap();
        let back: PromptQueue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, queue);
        assert!(serde_json::from_str::<PromptQueue>("[]")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn for_thread_preserves_order() {
        let mut queue = PromptQueue::default();
        queue.push("t1", "a", 1.0);
        queue.push("t1", "b", 2.0);
        queue.push("t1", "c", 3.0);
        let queued = queue.for_thread("t1");
        let texts: Vec<&str> = queued.iter().map(|prompt| prompt.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
    }

    #[test]
    fn nudge_and_edit_stay_inside_the_thread() {
        let mut queue = PromptQueue::default();
        let first = queue.push("t1", "a", 1.0).unwrap();
        queue.push("t2", "other", 2.0);
        let second = queue.push("t1", "b", 3.0).unwrap();
        let third = queue.push("t1", "c", 4.0).unwrap();
        assert!(!queue.can_nudge(&first, true));
        assert!(queue.nudge(&third, true));
        let texts: Vec<String> = queue
            .for_thread("t1")
            .into_iter()
            .map(|prompt| prompt.text)
            .collect();
        assert_eq!(texts, vec!["a", "c", "b"]);
        assert!(queue.move_before(&second, &first));
        let texts: Vec<String> = queue
            .for_thread("t1")
            .into_iter()
            .map(|prompt| prompt.text)
            .collect();
        assert_eq!(texts, vec!["b", "a", "c"]);
        assert_eq!(queue.for_thread("t2")[0].text, "other");
        assert!(queue.set_text(&first, "  aye "));
        assert_eq!(queue.text_of(&first), Some("aye"));
        assert!(!queue.set_text(&first, "  "));
        assert_eq!(queue.latest_for("t1").unwrap().text, "c");
    }
}
