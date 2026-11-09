//! Answers an agent's requests about other threads (`thread_request` stream
//! events): list them, read one, send one a message. The rules and the
//! reply text live in `bruhs_models::thread_link`, shared with desktop.
//!
//! A message joins its recipient's prompt queue. The web runs one turn at a
//! time, so it starts when the sender's turn ends: the view follows the
//! message to the recipient ([`ThreadLink::follow`]) and the queue drains
//! there like any queued prompt. For the same reason a sender cannot wait
//! for the reply inside its tool call here; the server does not offer that,
//! and a request that asks anyway is delivered without waiting.

use bruhs_models::agent::Role;
use bruhs_models::thread_link::{
    delivered_reply, list_reply, plan_message, read_reply, resolve_thread, Delivery, DeliveryState,
    Outgoing, Recipient, ThreadCard, ThreadLine, ThreadOperation, ThreadReply,
    ThreadReplySubmission, ThreadRequest,
};
use leptos::prelude::*;

use crate::queue::PromptQueue;
use crate::state::{AgentMessage, StreamBuffer, Thread};

/// Shown in the sender's thread when the view cannot follow its message.
pub const QUEUED_ELSEWHERE: &str =
    "This thread's agent queued a message in another thread. Open that thread to run it.";

#[derive(Clone, Copy)]
pub struct ThreadLink {
    threads: ReadSignal<Vec<Thread>>,
    set_threads: WriteSignal<Vec<Thread>>,
    active: ReadSignal<Option<String>>,
    messages: ReadSignal<Vec<AgentMessage>>,
    buffer: RwSignal<StreamBuffer>,
    streaming: Memo<Option<String>>,
    queue: RwSignal<PromptQueue>,
    catalog: crate::thread_catalog::ThreadCatalog,
    saves: crate::task_saves::TaskSaves,
    /// The user's limit on agent messages in a row.
    hops: RwSignal<u32>,
    /// `(sender, recipient)` of each message not yet followed, oldest first.
    sent: StoredValue<Vec<(String, String)>>,
}

/// Threads as cards, most recent first.
fn cards<'a>(
    threads: &'a [Thread],
    streaming: Option<&str>,
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
            running: streaming == Some(thread.id.as_str()),
            queued: queue.for_thread(&thread.id).len(),
        })
        .collect()
}

/// The prompt of the turn `thread_id` is running: the last user message in
/// the stream buffer, which holds that thread up to its prompt.
fn running_prompt(buffer: &StreamBuffer, thread_id: &str) -> String {
    if buffer.thread_id.as_deref() != Some(thread_id) {
        return String::new();
    }
    buffer
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(AgentMessage::text)
        .unwrap_or_default()
}

/// The recipient the view should follow to when `finished`'s turn ends: the
/// first thread it wrote to that still has the message queued. Clears every
/// message `finished` sent.
fn take_follow(
    sent: &mut Vec<(String, String)>,
    finished: &str,
    queue: &PromptQueue,
) -> Option<String> {
    let next = sent
        .iter()
        .find(|(from, to)| from == finished && queue.has_for(to))
        .map(|(_, to)| to.clone());
    sent.retain(|(from, _)| from != finished);
    next
}

impl ThreadLink {
    pub fn install() {
        let crate::state::StreamBufferSignal(buffer) = expect_context();
        let crate::state::StreamingThreadId(streaming) = expect_context();
        let crate::state::PromptQueueSignal(queue) = expect_context();
        provide_context(Self {
            threads: expect_context::<crate::state::ThreadsSignal>().0,
            set_threads: expect_context::<crate::state::SetThreadsSignal>().0,
            active: expect_context::<crate::state::ActiveThreadId>().0,
            messages: expect_context::<crate::state::MessagesSignal>().0,
            buffer,
            streaming,
            queue,
            catalog: expect_context(),
            saves: expect_context(),
            hops: expect_context::<crate::state::ThreadHopsSignal>().0,
            sent: StoredValue::new(Vec::new()),
        });
    }

    /// Answer `request` and hand the answer back to the waiting tool call.
    pub fn answer(self, request: ThreadRequest) {
        leptos::task::spawn_local(async move {
            let caller = request.thread_id.as_str().to_string();
            let reply = self.reply(&caller, request.operation).await;
            let body = ThreadReplySubmission {
                thread_id: request.thread_id,
                request_id: request.id,
                reply,
            };
            if let Err(error) = crate::api::submit_thread_reply(&body).await {
                // The turn ended or the server restarted; nobody is waiting.
                log::warn!("thread request answer was not delivered: {error}");
            }
        });
    }

    /// The thread to open now that `finished`'s turn ended, if its agent
    /// wrote to one whose message still waits. Asked once per turn end.
    pub fn follow(self, finished: &str) -> Option<String> {
        let queue = self.queue;
        self.sent
            .try_update_value(|sent| {
                queue.with_untracked(|queue| take_follow(sent, finished, queue))
            })
            .flatten()
    }

    async fn reply(self, caller: &str, operation: ThreadOperation) -> ThreadReply {
        let threads = self.threads.get_untracked();
        let streaming = self.streaming.get_untracked();
        match operation {
            ThreadOperation::List => self.queue.with_untracked(|queue| {
                list_reply(&cards(&threads, streaming.as_deref(), queue), caller)
            }),
            ThreadOperation::Read { thread_id, limit } => {
                let ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
                let id = match resolve_thread(&thread_id, &ids) {
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
                read_reply(
                    &title,
                    streaming.as_deref() == Some(id.as_str()),
                    &lines,
                    limit,
                )
            }
            ThreadOperation::Message {
                thread_id,
                title,
                message,
                wait,
            } => self.send(
                caller,
                &threads,
                thread_id.as_deref(),
                title.as_deref(),
                &message,
                wait,
            ),
        }
    }

    /// The settled messages of `id`, from wherever its newest copy is: the
    /// running turn's buffer, the open transcript, an unsaved edit, or disk.
    async fn settled_messages(self, id: &str) -> Result<Vec<AgentMessage>, ThreadReply> {
        let buffered = self.buffer.with_untracked(|buffer| {
            (buffer.thread_id.as_deref() == Some(id)).then(|| buffer.messages.clone())
        });
        if let Some(messages) = buffered {
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

    fn send(
        self,
        caller: &str,
        threads: &[Thread],
        thread_id: Option<&str>,
        title: Option<&str>,
        message: &str,
        wants_wait: bool,
    ) -> ThreadReply {
        let Some(sender) = threads.iter().find(|thread| thread.id == caller) else {
            return ThreadReply::refused("Not sent: this thread no longer exists in the app.");
        };
        let mut ids: Vec<&str> = threads.iter().map(|thread| thread.id.as_str()).collect();
        let prompt = self
            .buffer
            .with_untracked(|buffer| running_prompt(buffer, caller));
        // One turn runs at a time here, so the recipient cannot answer while
        // the sender's call is held: the message is never awaited.
        let Delivery { recipient, prompt } = match plan_message(&Outgoing {
            caller_id: caller,
            caller_title: &sender.title,
            caller_prompt: &prompt,
            ids: &ids,
            thread_id,
            title,
            message,
            wait: false,
            max_hops: self.hops.get_untracked(),
        }) {
            Ok(delivery) => delivery,
            Err(refusal) => return refusal,
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
        if self
            .queue
            .try_update(|queue| queue.push(&id, &prompt, js_sys::Date::now()))
            .flatten()
            .is_none()
        {
            return ThreadReply::refused("Not sent: the message could not be queued.");
        }
        if let Some(thread) = created.clone() {
            self.set_threads
                .update(|rows| rows.insert(0, thread.clone()));
            self.catalog.persist(None, thread);
        }
        self.sent
            .update_value(|sent| sent.push((caller.to_string(), id.clone())));
        if created.is_some() {
            ids.push(&id);
        }
        let state = if wants_wait {
            DeliveryState::QueuedNotAwaited
        } else {
            DeliveryState::Queued
        };
        delivered_reply(&id, &title, &ids, created.is_some(), state)
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

    #[test]
    fn cards_are_most_recent_first_with_state() {
        let threads = [
            thread("old", "Old", "", 1),
            thread("new", "New", "bruhs-ai", 9),
        ];
        let mut queue = PromptQueue::default();
        queue.push("old", "wait", 0.0);
        let cards = cards(&threads, Some("new"), &queue);
        assert_eq!(
            cards
                .iter()
                .map(|card| (card.id, card.project, card.running, card.queued))
                .collect::<Vec<_>>(),
            [("new", Some("bruhs-ai"), true, 0), ("old", None, false, 1)]
        );
    }

    #[test]
    fn the_running_prompt_is_the_buffered_threads_last_user_message() {
        let buffer = StreamBuffer {
            thread_id: Some("a".into()),
            messages: vec![
                AgentMessage::user("first".into(), 0.0),
                AgentMessage::user("second".into(), 1.0),
            ],
        };
        assert_eq!(running_prompt(&buffer, "a"), "second");
        assert_eq!(running_prompt(&buffer, "b"), "");
        assert_eq!(running_prompt(&StreamBuffer::default(), "a"), "");
    }

    #[test]
    fn the_view_follows_a_senders_first_still_queued_message_once() {
        let mut queue = PromptQueue::default();
        queue.push("c", "hello", 0.0);
        let mut sent = vec![
            ("a".to_string(), "b".to_string()),
            ("a".to_string(), "c".to_string()),
            ("x".to_string(), "c".to_string()),
        ];
        // `b` already ran or was removed from its queue, so `c` is next.
        assert_eq!(take_follow(&mut sent, "a", &queue), Some("c".into()));
        assert_eq!(sent, [("x".to_string(), "c".to_string())]);
        assert_eq!(take_follow(&mut sent, "a", &queue), None);
    }
}
