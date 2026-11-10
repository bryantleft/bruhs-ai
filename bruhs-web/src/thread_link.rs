//! Answers an agent's requests about other threads (`thread_request` stream
//! events): list them, read one, send one a message. The rules and the
//! reply text live in `bruhs_models::thread_link`, shared with desktop.
//!
//! A message joins its recipient's prompt queue. Threads run side by side
//! (see `state::Turns`), so the chat page starts it as soon as that thread is
//! idle and a lane is free, without moving the view.
//!
//! A sender may wait: its tool call is held ([`Wait`]) until the recipient's
//! turn for that message ends, and gets that turn's reply. The call is not
//! held when nothing could answer it soon: the chat page is not mounted, the
//! queue is paused, or no lane is free.

use bruhs_models::agent::Role;
use bruhs_models::ids::ThreadId;
use bruhs_models::thread_link::{
    answer_reply, delivered_reply, list_reply, plan_message, read_reply, recipient_waiting_reply,
    resolve_thread, Answer, Delivery, DeliveryState, Outgoing, Recipient, ThreadCard, ThreadLine,
    ThreadOperation, ThreadReply, ThreadReplySubmission, ThreadRequest,
};
use leptos::prelude::*;

use crate::queue::PromptQueue;
use crate::state::{AgentMessage, Thread, Turns};

/// A tool call held until `recipient` has answered `prompt`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Wait {
    request_id: String,
    /// The thread whose turn waits.
    caller: ThreadId,
    recipient: String,
    /// The delivered prompt, header included: what names the turn to wait
    /// for among the recipient's other turns.
    prompt: String,
}

/// How the turn a call waited on ended.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ended {
    Replied(String),
    Stopped,
    Failed(String),
}

#[derive(Clone, Copy)]
pub struct ThreadLink {
    threads: ReadSignal<Vec<Thread>>,
    set_threads: WriteSignal<Vec<Thread>>,
    active: ReadSignal<Option<String>>,
    messages: ReadSignal<Vec<AgentMessage>>,
    turns: Turns,
    queue: RwSignal<PromptQueue>,
    /// Interrupt and a reload pause every queue until Resume or a send.
    queue_held: RwSignal<bool>,
    catalog: crate::thread_catalog::ThreadCatalog,
    saves: crate::task_saves::TaskSaves,
    /// The user's limit on agent messages in a row.
    hops: RwSignal<u32>,
    /// The chat page is mounted. It owns the turn lifecycle, so a queued
    /// message can only start while it is.
    chat_mounted: RwSignal<bool>,
    waits: StoredValue<Vec<Wait>>,
}

/// Threads as cards, most recent first.
fn cards<'a>(
    threads: &'a [Thread],
    running: &[String],
    queue: &PromptQueue,
) -> Vec<ThreadCard<'a>> {
    let mut recent: Vec<&Thread> = threads.iter().collect();
    recent.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
    recent
        .into_iter()
        .map(|thread| ThreadCard {
            id: &thread.id,
            title: &thread.title,
            project: Some(thread.project_name.as_str()).filter(|name| !name.trim().is_empty()),
            running: running.contains(&thread.id),
            queued: queue.for_thread(&thread.id).len(),
        })
        .collect()
}

/// The prompt of a running turn: the last user message of its settled
/// messages, which end with it.
fn running_prompt(settled: &[AgentMessage]) -> String {
    settled
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(AgentMessage::text)
        .unwrap_or_default()
}

/// How the turn for `prompt` ended, given the thread's messages once a turn
/// there is over. `None` when that turn ran another prompt: ours still waits.
fn ended_for(prompt: &str, settled: &[AgentMessage], stopped: bool) -> Option<Ended> {
    let prompt = prompt.trim();
    let asked = settled
        .iter()
        .rposition(|message| message.role == Role::User && message.text().trim() == prompt)?;
    let reply = settled[asked + 1..]
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant);
    Some(match reply {
        Some(reply) => match &reply.error {
            Some(error) => Ended::Failed(error.clone()),
            None if stopped => Ended::Stopped,
            None => Ended::Replied(reply.text()),
        },
        None if stopped => Ended::Stopped,
        None => Ended::Failed("its turn ended without a reply".to_string()),
    })
}

/// Whether a waited-for message can no longer run: its thread is gone, or
/// the message left the queue while no turn there is running or starting.
fn is_removed(wait: &Wait, queue: &PromptQueue, exists: bool, busy: bool) -> bool {
    let queued = queue
        .for_thread(&wait.recipient)
        .iter()
        .any(|item| item.text.trim() == wait.prompt.trim());
    !exists || !(queued || busy)
}

impl ThreadLink {
    pub fn install() {
        let crate::state::PromptQueueSignal(queue) = expect_context();
        let crate::state::QueueHeldSignal(queue_held) = expect_context();
        let link = Self {
            threads: expect_context::<crate::state::ThreadsSignal>().0,
            set_threads: expect_context::<crate::state::SetThreadsSignal>().0,
            active: expect_context::<crate::state::ActiveThreadId>().0,
            messages: expect_context::<crate::state::MessagesSignal>().0,
            turns: expect_context(),
            queue,
            queue_held,
            catalog: expect_context(),
            saves: expect_context(),
            hops: expect_context::<crate::state::ThreadHopsSignal>().0,
            chat_mounted: RwSignal::new(false),
            waits: StoredValue::new(Vec::new()),
        };
        provide_context(link);
        // A held call must end when its message can no longer run: the user
        // removed it from the queue, or deleted the thread.
        Effect::new(move |_| {
            link.turns.track();
            link.queue.track();
            link.threads.track();
            link.end_removed();
        });
    }

    /// The chat page mounted or unmounted.
    pub fn set_chat_mounted(self, mounted: bool) {
        self.chat_mounted.set(mounted);
    }

    /// Answer `request` and hand the answer back to the waiting tool call,
    /// now or once the recipient has replied.
    pub fn answer(self, request: ThreadRequest) {
        leptos::task::spawn_local(async move {
            let caller = request.thread_id.as_str().to_string();
            let reply = match request.operation {
                ThreadOperation::List => self.list(&caller),
                ThreadOperation::Read { thread_id, limit } => self.read(&thread_id, limit).await,
                ThreadOperation::Message {
                    thread_id,
                    title,
                    message,
                    wait,
                } => {
                    let held = Wait {
                        request_id: request.id.clone(),
                        caller: request.thread_id.clone(),
                        recipient: String::new(),
                        prompt: String::new(),
                    };
                    match self.send(
                        &caller,
                        thread_id.as_deref(),
                        title.as_deref(),
                        &message,
                        wait,
                        held,
                    ) {
                        Some(reply) => reply,
                        // Held: answered when the recipient's turn ends.
                        None => return,
                    }
                }
            };
            Self::post(request.thread_id, request.id, reply);
        });
    }

    fn post(thread_id: ThreadId, request_id: String, reply: ThreadReply) {
        leptos::task::spawn_local(async move {
            let body = ThreadReplySubmission {
                thread_id,
                request_id,
                reply,
            };
            if let Err(error) = crate::api::submit_thread_reply(&body).await {
                // The turn ended or the server restarted; nobody is waiting.
                log::warn!("thread request answer was not delivered: {error}");
            }
        });
    }

    /// `thread_id`'s turn ended with these messages. Answer the calls that
    /// waited on it, and forget the calls its own turn was holding.
    pub fn turn_ended(self, thread_id: &str, settled: &[AgentMessage], stopped: bool) {
        let due = self
            .waits
            .try_update_value(|waits| {
                // The broker ended the calls of a turn that is over.
                waits.retain(|wait| wait.caller.as_str() != thread_id);
                let mut due = Vec::new();
                waits.retain(|wait| {
                    if wait.recipient != thread_id {
                        return true;
                    }
                    match ended_for(&wait.prompt, settled, stopped) {
                        Some(ended) => {
                            due.push((wait.clone(), ended));
                            false
                        }
                        None => true,
                    }
                });
                due
            })
            .unwrap_or_default();
        if due.is_empty() {
            return;
        }
        let threads = self.threads.get_untracked();
        let ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        let title = threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .map(|thread| thread.title.as_str())
            .unwrap_or_default();
        for (wait, ended) in due {
            let answer = match &ended {
                Ended::Replied(text) => Answer::Replied(text),
                Ended::Stopped => Answer::Stopped,
                Ended::Failed(error) => Answer::Failed(error),
            };
            Self::post(
                wait.caller,
                wait.request_id,
                answer_reply(thread_id, title, &ids, answer),
            );
        }
    }

    /// End the held calls whose message can no longer run.
    fn end_removed(self) {
        let threads = self.threads.get_untracked();
        let gone = self
            .waits
            .try_update_value(|waits| {
                let mut gone = Vec::new();
                waits.retain(|wait| {
                    let exists = threads.iter().any(|thread| thread.id == wait.recipient);
                    let busy = self.turns.is_busy_untracked(&wait.recipient);
                    let removed = self
                        .queue
                        .with_untracked(|queue| is_removed(wait, queue, exists, busy));
                    if removed {
                        gone.push(wait.clone());
                    }
                    !removed
                });
                gone
            })
            .unwrap_or_default();
        let ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        for wait in gone {
            let title = threads
                .iter()
                .find(|thread| thread.id == wait.recipient)
                .map(|thread| thread.title.as_str())
                .unwrap_or_default();
            let reply = answer_reply(&wait.recipient, title, &ids, Answer::Removed);
            Self::post(wait.caller, wait.request_id, reply);
        }
    }

    fn list(self, caller: &str) -> ThreadReply {
        let threads = self.threads.get_untracked();
        let running = self.turns.running_untracked();
        self.queue
            .with_untracked(|queue| list_reply(&cards(&threads, &running, queue), caller))
    }

    async fn read(self, reference: &str, limit: Option<u32>) -> ThreadReply {
        let threads = self.threads.get_untracked();
        let ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        let id = match resolve_thread(reference, &ids) {
            Ok(id) => id.to_string(),
            Err(refusal) => return refusal,
        };
        let title = threads
            .iter()
            .find(|thread| thread.id == id)
            .map(|thread| thread.title.clone())
            .unwrap_or_default();
        let messages = match self.settled_messages(&id).await {
            Ok(messages) => messages,
            Err(refusal) => return refusal,
        };
        let texts: Vec<(bool, String)> = messages
            .iter()
            .map(|message| (message.role == Role::User, message.text()))
            .collect();
        let lines: Vec<ThreadLine<'_>> = texts
            .iter()
            .map(|(from_user, text)| ThreadLine {
                from_user: *from_user,
                text,
            })
            .collect();
        read_reply(&title, self.turns.is_running_untracked(&id), &lines, limit)
    }

    /// The settled messages of `id`, from wherever its newest copy is: its
    /// running turn, the open transcript, an unsaved edit, or disk.
    async fn settled_messages(self, id: &str) -> Result<Vec<AgentMessage>, ThreadReply> {
        if let Some(messages) = self.turns.messages(id) {
            return Ok(messages);
        }
        if self
            .active
            .with_untracked(|active| active.as_deref() == Some(id))
        {
            return Ok(self.messages.get_untracked());
        }
        match crate::storage::load_messages(id).await {
            Ok(loaded) => Ok(self.saves.loaded_conversation(id, loaded)),
            Err(error) => {
                log::warn!("thread read could not load saved messages: {error:?}");
                Err(ThreadReply::refused(
                    "That thread's saved messages could not be loaded.",
                ))
            }
        }
    }

    /// Deliver a message. `None` when the call is held for the reply; `held`
    /// then names the request to answer later.
    fn send(
        self,
        caller: &str,
        thread_id: Option<&str>,
        title: Option<&str>,
        message: &str,
        wants_wait: bool,
        held: Wait,
    ) -> Option<ThreadReply> {
        let threads = self.threads.get_untracked();
        let Some(sender) = threads.iter().find(|thread| thread.id == caller) else {
            return Some(ThreadReply::refused(
                "Not sent: this thread no longer exists in the app.",
            ));
        };
        let mut ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        let prompt = self
            .turns
            .messages(caller)
            .map(|settled| running_prompt(&settled))
            .unwrap_or_default();
        // The queue can start a message only while the chat page is mounted
        // and nothing is paused.
        let draining = self.chat_mounted.get_untracked() && !self.queue_held.get_untracked();
        let known = thread_id
            .and_then(|reference| resolve_thread(reference, &ids).ok())
            .map(str::to_string);
        // A wait needs a recipient that can answer inside this call: one
        // that is not itself waiting (that could never finish), and a turn
        // that can start (or is running, with this queued behind it).
        let mut wait = false;
        if wants_wait {
            if let Some(known) = &known {
                let waiting = self
                    .waits
                    .with_value(|waits| waits.iter().any(|held| held.caller.as_str() == known));
                if waiting {
                    let title = threads
                        .iter()
                        .find(|thread| &thread.id == known)
                        .map(|thread| thread.title.as_str())
                        .unwrap_or_default();
                    return Some(recipient_waiting_reply(known, title, &ids));
                }
            }
            let running = known
                .as_deref()
                .is_some_and(|id| self.turns.is_running_untracked(id));
            wait = draining && (running || self.turns.has_room_untracked());
        }
        let Delivery { recipient, prompt } = match plan_message(&Outgoing {
            caller_id: caller,
            caller_title: &sender.title,
            caller_prompt: &prompt,
            ids: &ids,
            thread_id,
            title,
            message,
            wait,
            max_hops: self.hops.get_untracked(),
        }) {
            Ok(delivery) => delivery,
            Err(refusal) => return Some(refusal),
        };
        let created = match &recipient {
            Recipient::Existing(_) => None,
            Recipient::New { title } => {
                // Runs where the sender runs: same project, worktree and model.
                let mut thread = Thread::new(
                    title.clone(),
                    sender.project_name.clone(),
                    sender.project_path.clone(),
                    crate::state::now_epoch_ms(),
                );
                thread.worktree_path.clone_from(&sender.worktree_path);
                thread.branch.clone_from(&sender.branch);
                thread.model.clone_from(&sender.model);
                thread.run_locked = sender.run_locked;
                Some(thread)
            }
        };
        let (id, title) = match (&recipient, &created) {
            (_, Some(thread)) => (thread.id.clone(), thread.title.clone()),
            (Recipient::Existing(id), None) => (
                id.clone(),
                threads
                    .iter()
                    .find(|thread| &thread.id == id)
                    .map(|thread| thread.title.clone())
                    .unwrap_or_default(),
            ),
            (Recipient::New { .. }, None) => unreachable!("a new recipient was just created"),
        };
        // Whether its turn starts as soon as it is queued.
        let starts =
            draining && !self.turns.is_busy_untracked(&id) && self.turns.has_room_untracked();
        // The thread first: the queue names it, and the chat page only starts
        // prompts for threads that exist.
        if let Some(thread) = created.clone() {
            self.set_threads
                .update(|rows| rows.insert(0, thread.clone()));
            self.catalog.persist(None, thread);
        }
        if wait {
            // Before the message is queued, so a turn that starts and ends at
            // once still finds the call it must answer.
            self.waits.update_value(|waits| {
                waits.push(Wait {
                    recipient: id.clone(),
                    prompt: prompt.clone(),
                    ..held
                });
            });
        }
        if self
            .queue
            .try_update(|queue| queue.push(&id, &prompt, js_sys::Date::now()))
            .flatten()
            .is_none()
        {
            self.waits
                .update_value(|waits| waits.retain(|wait| wait.prompt != prompt));
            return Some(ThreadReply::refused(
                "Not sent: the message could not be queued.",
            ));
        }
        if wait {
            return None;
        }
        if created.is_some() {
            ids.push(&id);
        }
        let state = match (starts, wants_wait) {
            (true, _) => DeliveryState::Started,
            (false, true) => DeliveryState::QueuedNotAwaited,
            (false, false) => DeliveryState::Queued,
        };
        Some(delivered_reply(&id, &title, &ids, created.is_some(), state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::EpochMs;

    fn thread(id: &str, title: &str, project: &str, updated: i64) -> Thread {
        let mut thread = Thread::new(
            title.into(),
            project.into(),
            None,
            EpochMs::new(updated).unwrap(),
        );
        thread.id = id.into();
        thread
    }

    fn reply(text: &str) -> AgentMessage {
        let mut message = AgentMessage::assistant_placeholder("m".into(), 0.0);
        message
            .parts
            .push(crate::state::MessagePart::Text(text.into()));
        message
    }

    fn wait(recipient: &str, prompt: &str) -> Wait {
        Wait {
            request_id: "ask-1".into(),
            caller: ThreadId::new_v4(),
            recipient: recipient.into(),
            prompt: prompt.into(),
        }
    }

    #[test]
    fn cards_are_most_recent_first_with_state() {
        let threads = [
            thread("old", "Old", "", 1),
            thread("new", "New", "bruhs-ai", 9),
            thread("mid", "Mid", "", 5),
        ];
        let mut queue = PromptQueue::default();
        queue.push("old", "wait", 0.0);
        // Several threads can be running at once.
        let running = ["new".to_string(), "mid".to_string()];
        let cards = cards(&threads, &running, &queue);
        assert_eq!(
            cards
                .iter()
                .map(|card| (card.id, card.project, card.running, card.queued))
                .collect::<Vec<_>>(),
            [
                ("new", Some("bruhs-ai"), true, 0),
                ("mid", None, true, 0),
                ("old", None, false, 1)
            ]
        );
    }

    #[test]
    fn the_running_prompt_is_the_turns_last_user_message() {
        let settled = [
            AgentMessage::user("first".into(), 0.0),
            reply("answer"),
            AgentMessage::user("second".into(), 1.0),
        ];
        assert_eq!(running_prompt(&settled), "second");
        assert_eq!(running_prompt(&[]), "");
    }

    #[test]
    fn a_waiting_call_gets_the_reply_to_its_own_message() {
        let mut settled = vec![
            AgentMessage::user("an earlier prompt".into(), 0.0),
            reply("earlier reply"),
        ];
        // The turn that just ended ran another prompt: ours still waits.
        assert_eq!(ended_for("ask", &settled, false), None);
        settled.push(AgentMessage::user("  ask  ".into(), 1.0));
        settled.push(reply("4"));
        assert_eq!(
            ended_for("ask", &settled, false),
            Some(Ended::Replied("4".into()))
        );
        assert_eq!(ended_for("ask", &settled, true), Some(Ended::Stopped));
        // A steer after the prompt does not hide the reply.
        settled.insert(3, AgentMessage::user("and hurry".into(), 2.0));
        assert_eq!(
            ended_for("ask", &settled, false),
            Some(Ended::Replied("4".into()))
        );
        settled.last_mut().unwrap().error = Some("rate limited".into());
        assert_eq!(
            ended_for("ask", &settled, true),
            Some(Ended::Failed("rate limited".into())),
            "an error says more than the stop that followed it"
        );
        let unanswered = [AgentMessage::user("ask".into(), 0.0)];
        assert_eq!(ended_for("ask", &unanswered, true), Some(Ended::Stopped));
        assert!(matches!(
            ended_for("ask", &unanswered, false),
            Some(Ended::Failed(_))
        ));
    }

    #[test]
    fn a_waiting_call_ends_when_its_message_can_no_longer_run() {
        let held = wait("b", "ask");
        let mut queue = PromptQueue::default();
        // Popped for a turn that is starting or running: still coming.
        assert!(!is_removed(&held, &queue, true, true));
        // Neither queued nor running: the user removed it.
        assert!(is_removed(&held, &queue, true, false));
        queue.push("b", "something else", 0.0);
        assert!(is_removed(&held, &queue, true, false));
        queue.push("b", "ask", 0.0);
        assert!(!is_removed(&held, &queue, true, false));
        // Its thread was deleted, queue row or not.
        assert!(is_removed(&held, &queue, false, false));
        assert!(is_removed(&held, &queue, false, true));
    }
}
