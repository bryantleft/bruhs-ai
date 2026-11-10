//! The turns that are running, one per thread at most.
//!
//! A running turn keeps its thread's settled messages (up to and including
//! the prompt) and, apart from them, the reply it is writing. The reply has
//! its own signal, so a token re-renders only that reply: not the thread's
//! transcript, and not another thread's reply streaming at the same time.
//! When the turn ends the reply joins the settled messages and the turn is
//! gone.
//!
//! Several threads may run at once, up to [`MAX_RUNNING_TURNS`].

use std::collections::HashMap;

use leptos::prelude::*;

use super::AgentMessage;

/// Most turns that run at once. Each holds one connection to the server for
/// as long as it streams, and a browser allows a host six in all.
pub const MAX_RUNNING_TURNS: usize = 3;

/// The reply a turn is writing. `None` once it has joined the transcript.
pub type LiveReply = ArcRwSignal<Option<AgentMessage>>;

struct RunningTurn {
    /// Settled messages, up to and including this turn's prompt.
    messages: Vec<AgentMessage>,
    reply: LiveReply,
    /// Stops the turn's request. `None` when the browser gave no controller.
    abort: Option<web_sys::AbortController>,
}

/// Every running turn, by thread id.
#[derive(Clone, Copy)]
pub struct Turns {
    /// Threads with a running turn, oldest first. Changes when a turn starts
    /// or ends, never per token.
    running: RwSignal<Vec<String>>,
    /// Threads whose next turn is being prepared (their saved messages are
    /// loading). Not running yet, but not free either.
    starting: RwSignal<Vec<String>>,
    /// `AbortController` is neither `Send` nor `Sync`.
    turns: StoredValue<HashMap<String, RunningTurn>, LocalStorage>,
}

impl Turns {
    pub fn new() -> Self {
        Self {
            running: RwSignal::new(Vec::new()),
            starting: RwSignal::new(Vec::new()),
            turns: StoredValue::new_local(HashMap::new()),
        }
    }

    /// Start `thread_id`'s turn: `messages` end with its prompt, and `reply`
    /// is the message the answer streams into. Returns the reply's signal.
    pub fn begin(
        self,
        thread_id: &str,
        messages: Vec<AgentMessage>,
        reply: AgentMessage,
        abort: Option<web_sys::AbortController>,
    ) -> LiveReply {
        let live = ArcRwSignal::new(Some(reply));
        self.turns.update_value(|turns| {
            turns.insert(
                thread_id.to_string(),
                RunningTurn {
                    messages,
                    reply: live.clone(),
                    abort,
                },
            );
        });
        self.starting
            .update(|starting| starting.retain(|id| id != thread_id));
        self.running.update(|running| {
            running.retain(|id| id != thread_id);
            running.push(thread_id.to_string());
        });
        live
    }

    /// End `thread_id`'s turn: its reply joins the settled messages, which
    /// are returned. `None` when no turn was running there.
    pub fn end(self, thread_id: &str) -> Option<Vec<AgentMessage>> {
        let turn = self
            .turns
            .try_update_value(|turns| turns.remove(thread_id))
            .flatten()?;
        let mut messages = turn.messages;
        messages.extend(turn.reply.try_update(Option::take).flatten());
        self.running
            .update(|running| running.retain(|id| id != thread_id));
        Some(messages)
    }

    /// `thread_id`'s next turn is being prepared. `false` when it already
    /// is, or a turn is running there.
    pub fn reserve(self, thread_id: &str) -> bool {
        if self.is_busy_untracked(thread_id) {
            return false;
        }
        self.starting
            .update(|starting| starting.push(thread_id.to_string()));
        true
    }

    /// The prepared turn did not start after all.
    pub fn release(self, thread_id: &str) {
        self.starting
            .update(|starting| starting.retain(|id| id != thread_id));
    }

    /// Whether a turn is running in `thread_id`. Tracked.
    pub fn is_running(self, thread_id: &str) -> bool {
        self.running
            .with(|running| running.iter().any(|id| id == thread_id))
    }

    pub fn is_running_untracked(self, thread_id: &str) -> bool {
        self.running
            .with_untracked(|running| running.iter().any(|id| id == thread_id))
    }

    /// Running, or about to be.
    pub fn is_busy_untracked(self, thread_id: &str) -> bool {
        self.is_running_untracked(thread_id)
            || self
                .starting
                .with_untracked(|starting| starting.iter().any(|id| id == thread_id))
    }

    /// Threads with a running turn, oldest first. Tracked.
    pub fn running(self) -> Vec<String> {
        self.running.get()
    }

    pub fn running_untracked(self) -> Vec<String> {
        self.running.get_untracked()
    }

    /// Whether any turn is running. Tracked.
    pub fn any_running(self) -> bool {
        self.running.with(|running| !running.is_empty())
    }

    pub fn any_running_untracked(self) -> bool {
        self.running.with_untracked(|running| !running.is_empty())
    }

    /// Track turns starting and ending, and turns being prepared.
    pub fn track(self) {
        self.running.track();
        self.starting.track();
    }

    /// Whether one more turn may start now.
    pub fn has_room_untracked(self) -> bool {
        let running = self.running.with_untracked(Vec::len);
        let starting = self.starting.with_untracked(Vec::len);
        running + starting < MAX_RUNNING_TURNS
    }

    /// Whether the turn on screen is running: tracks `active` and the turns
    /// that start and end, never a token.
    pub fn here(self, active: ReadSignal<Option<String>>) -> Memo<bool> {
        Memo::new(move |_| active.with(|id| id.as_deref().is_some_and(|id| self.is_running(id))))
    }

    /// The settled messages of `thread_id`'s running turn (not its reply).
    pub fn messages(self, thread_id: &str) -> Option<Vec<AgentMessage>> {
        self.turns
            .try_with_value(|turns| turns.get(thread_id).map(|turn| turn.messages.clone()))
            .flatten()
    }

    /// The settled messages plus the reply as it stands: what to save.
    pub fn snapshot(self, thread_id: &str) -> Option<Vec<AgentMessage>> {
        self.turns
            .try_with_value(|turns| {
                turns.get(thread_id).map(|turn| {
                    let mut messages = turn.messages.clone();
                    messages.extend(turn.reply.get_untracked());
                    messages
                })
            })
            .flatten()
    }

    /// Change the settled messages of `thread_id`'s running turn.
    pub fn update_messages(self, thread_id: &str, change: impl FnOnce(&mut Vec<AgentMessage>)) {
        self.turns.update_value(|turns| {
            if let Some(turn) = turns.get_mut(thread_id) {
                change(&mut turn.messages);
            }
        });
    }

    /// The reply `thread_id`'s turn is writing. Call [`Turns::is_running`]
    /// first where the caller must react to the turn starting or ending.
    pub fn reply(self, thread_id: &str) -> Option<LiveReply> {
        self.turns
            .try_with_value(|turns| turns.get(thread_id).map(|turn| turn.reply.clone()))
            .flatten()
    }

    /// Stop `thread_id`'s request. `false` when there was nothing to stop.
    pub fn abort(self, thread_id: &str) -> bool {
        self.turns
            .try_update_value(|turns| {
                turns
                    .get_mut(thread_id)
                    .and_then(|turn| turn.abort.take())
                    .map(|controller| controller.abort())
                    .is_some()
            })
            .unwrap_or(false)
    }
}

impl Default for Turns {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns() -> (Owner, Turns) {
        let owner = Owner::new();
        let turns = owner.with(Turns::new);
        (owner, turns)
    }

    #[test]
    fn threads_run_side_by_side_and_each_keeps_its_own_reply() {
        let (_owner, turns) = turns();
        let a = turns.begin(
            "a",
            vec![AgentMessage::user("ask a".into(), 0.0)],
            AgentMessage::assistant_placeholder("m".into(), 0.0),
            None,
        );
        let b = turns.begin(
            "b",
            vec![AgentMessage::user("ask b".into(), 0.0)],
            AgentMessage::assistant_placeholder("m".into(), 0.0),
            None,
        );
        assert_eq!(turns.running_untracked(), ["a", "b"]);
        assert!(turns.is_running_untracked("a") && !turns.is_running_untracked("c"));

        // A token for one reply leaves the other alone.
        a.update(|reply| {
            if let Some(reply) = reply {
                reply
                    .parts
                    .push(crate::state::MessagePart::Text("for a".into()));
            }
        });
        assert_eq!(b.get_untracked().unwrap().text(), "");
        assert_eq!(turns.snapshot("a").unwrap()[1].text(), "for a");
        assert_eq!(turns.messages("a").unwrap().len(), 1);

        // Ending one joins its reply and leaves the other running.
        let settled = turns.end("a").unwrap();
        assert_eq!(settled.len(), 2);
        assert_eq!(settled[1].text(), "for a");
        assert_eq!(turns.running_untracked(), ["b"]);
        assert!(turns.messages("a").is_none() && turns.reply("a").is_none());
        assert!(turns.end("a").is_none());
        assert!(turns.reply("b").is_some());
    }

    #[test]
    fn room_counts_running_and_prepared_turns() {
        let (_owner, turns) = turns();
        for id in ["a", "b"] {
            turns.begin(
                id,
                Vec::new(),
                AgentMessage::assistant_placeholder("m".into(), 0.0),
                None,
            );
        }
        assert!(turns.has_room_untracked());
        assert!(turns.reserve("c"));
        assert!(!turns.reserve("c"), "already being prepared");
        assert!(!turns.reserve("a"), "already running");
        assert!(!turns.has_room_untracked());
        assert!(turns.is_busy_untracked("c") && !turns.is_running_untracked("c"));
        turns.release("c");
        assert!(turns.has_room_untracked() && !turns.is_busy_untracked("c"));
        // Starting a prepared turn moves it from prepared to running.
        assert!(turns.reserve("c"));
        turns.begin(
            "c",
            Vec::new(),
            AgentMessage::assistant_placeholder("m".into(), 0.0),
            None,
        );
        assert!(turns.is_running_untracked("c") && !turns.has_room_untracked());
        assert_eq!(MAX_RUNNING_TURNS, 3);
    }

    #[test]
    fn settled_messages_change_in_place_and_abort_is_one_shot() {
        let (_owner, turns) = turns();
        turns.begin(
            "a",
            vec![AgentMessage::user("ask".into(), 0.0)],
            AgentMessage::assistant_placeholder("m".into(), 0.0),
            None,
        );
        turns.update_messages("a", |messages| {
            messages[0].checkpoint = Some("tree".into());
        });
        assert_eq!(
            turns.messages("a").unwrap()[0].checkpoint.as_deref(),
            Some("tree")
        );
        turns.update_messages("missing", |messages| messages.clear());
        assert!(!turns.abort("a"), "no controller was given");
        assert!(!turns.abort("missing"));
    }
}
