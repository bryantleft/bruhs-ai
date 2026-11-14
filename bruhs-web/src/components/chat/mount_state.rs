//! The identity/presence changes that require mounting chat subtrees. Detailed
//! thread/message signals belong to the already mounted header and transcript.

use crate::state::{AgentMessage, Thread};
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub(super) struct ChatMount {
    pub thread_id: Memo<Option<String>>,
    pub transcript_empty: Memo<bool>,
}

impl ChatMount {
    pub fn new(
        active: ReadSignal<Option<String>>,
        threads: ReadSignal<Vec<Thread>>,
        messages: ReadSignal<Vec<AgentMessage>>,
        streaming: impl Into<Signal<bool>>,
    ) -> Self {
        let streaming = streaming.into();
        Self {
            thread_id: Memo::new(move |_| {
                let id = active.get()?;
                threads
                    .with(|threads| threads.iter().any(|thread| thread.id == id))
                    .then_some(id)
            }),
            transcript_empty: Memo::new(move |_| {
                messages.with(|messages| messages.is_empty()) && !streaming.get()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MessagePart, MessageStatus};
    use bruhs_models::EpochMs;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn thread(title: &str) -> Thread {
        Thread::new(
            title.into(),
            String::new(),
            None,
            EpochMs::saturating_from_i64(0),
        )
    }

    #[test]
    fn metadata_edits_and_reply_completion_keep_existing_chat_mounts() {
        Owner::new().with(|| {
            let first = thread("first");
            let id = first.id.clone();
            let (threads, set_threads) = signal(vec![first, thread("other")]);
            let (active, _) = signal(Some(id.clone()));
            let (messages, set_messages) = signal(vec![AgentMessage::user("question".into(), 0.0)]);
            let (streaming, set_streaming) = signal(true);
            let mounts = ChatMount::new(active, threads, messages, streaming);
            let shell_runs = Arc::new(AtomicUsize::new(0));
            let shell_count = Arc::clone(&shell_runs);
            let shell = Memo::new(move |_| {
                shell_count.fetch_add(1, Ordering::Relaxed);
                mounts.thread_id.get()
            });
            let body_runs = Arc::new(AtomicUsize::new(0));
            let body_count = Arc::clone(&body_runs);
            let body = Memo::new(move |_| {
                body_count.fetch_add(1, Ordering::Relaxed);
                mounts.transcript_empty.get()
            });
            assert_eq!(shell.get_untracked(), Some(id.clone()));
            assert!(!body.get_untracked());

            set_threads.update(|threads| {
                threads[0].title = "renamed".into();
                threads[0].pinned = true;
                threads[1].title = "unrelated edit".into();
            });
            set_messages.update(|messages| {
                messages[0].parts.push(MessagePart::Text(" more".into()));
                let mut reply = AgentMessage::assistant_placeholder("m".into(), 1.0);
                reply.parts.push(MessagePart::Text("complete reply".into()));
                reply.status = MessageStatus::Finished { at_ms: Some(2.0) };
                messages.push(reply);
            });
            set_streaming.set(false);

            assert_eq!(shell.get_untracked(), Some(id));
            assert!(!body.get_untracked());
            assert_eq!(shell_runs.load(Ordering::Relaxed), 1);
            assert_eq!(body_runs.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn navigation_deletion_and_empty_transitions_still_change_mount_state() {
        Owner::new().with(|| {
            let first = thread("first");
            let first_id = first.id.clone();
            let second = thread("second");
            let second_id = second.id.clone();
            let (threads, set_threads) = signal(vec![first, second]);
            let (active, set_active) = signal(Some(first_id.clone()));
            let (messages, set_messages) = signal(Vec::<AgentMessage>::new());
            let (streaming, set_streaming) = signal(false);
            let mounts = ChatMount::new(active, threads, messages, streaming);
            assert!(mounts.transcript_empty.get_untracked());
            set_streaming.set(true);
            assert!(!mounts.transcript_empty.get_untracked());
            set_messages.set(vec![AgentMessage::user("first prompt".into(), 0.0)]);
            set_streaming.set(false);
            assert!(!mounts.transcript_empty.get_untracked());
            set_messages.set(Vec::new());
            assert!(mounts.transcript_empty.get_untracked());
            set_active.set(Some(second_id.clone()));
            assert_eq!(mounts.thread_id.get_untracked(), Some(second_id.clone()));
            set_threads.update(|threads| threads.retain(|thread| thread.id != second_id));
            assert_eq!(mounts.thread_id.get_untracked(), None);
            set_active.set(Some(first_id.clone()));
            assert_eq!(mounts.thread_id.get_untracked(), Some(first_id));
            set_active.set(None);
            assert_eq!(mounts.thread_id.get_untracked(), None);
        });
    }
}
