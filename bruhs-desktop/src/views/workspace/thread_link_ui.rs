//! Answers an agent's requests about other threads: list them, read one,
//! send one a message. The rules and reply text are shared with the web in
//! `bruhs_models::thread_link`.
//!
//! A message goes through the recipient's own chat: it starts a turn there
//! at once when that thread is free, and otherwise waits in its prompt
//! queue. Threads run side by side here, so the view never moves, and a
//! sender may wait: its tool call is held ([`ThreadWait`]) until the
//! recipient's turn for that message ends, and gets that turn's reply.

use super::Workspace;
use crate::app_state::DesktopState;
use crate::views::chat::{ChatView, MessageRole, ThreadMessageAnswer};
use bruhs_models::ids::ThreadId;
use bruhs_models::thread_link::{
    answer_reply, delivered_reply, list_reply, plan_message, read_reply, recipient_waiting_reply,
    resolve_thread, Answer, Delivery, DeliveryState, Outgoing, Recipient, ThreadCard, ThreadLine,
    ThreadOperation, ThreadReply, ThreadRequest,
};
use gpui::{Context, Entity, Window};

/// A tool call held until `recipient` has answered `prompt`.
pub(super) struct ThreadWait {
    request_id: String,
    /// The thread whose turn waits.
    caller: ThreadId,
    recipient: String,
    /// The delivered prompt, header included: what names the turn to wait
    /// for among the recipient's other turns.
    prompt: String,
}

/// What the app does with one request.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ThreadAnswer {
    /// Answer the tool call now.
    Now(ThreadReply),
    /// Hold the tool call until `recipient` has answered `prompt`.
    Later { recipient: String, prompt: String },
}

impl Workspace {
    /// Answer `request`, raised by a turn of `chat`, and hand the answer
    /// back to its waiting tool call, now or once the recipient has replied.
    pub(super) fn answer_thread_request(
        &mut self,
        chat: &Entity<ChatView>,
        request: &ThreadRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match self.thread_reply(chat, &request.operation, window, cx) {
            ThreadAnswer::Now(reply) => {
                Self::resolve_thread_request(&request.thread_id, &request.id, reply, cx)
            }
            ThreadAnswer::Later { recipient, prompt } => {
                self.thread_waits.push(ThreadWait {
                    request_id: request.id.clone(),
                    caller: request.thread_id.clone(),
                    recipient: recipient.clone(),
                    prompt,
                });
                // A message that could not start a turn has already ended.
                let recipient = self
                    .threads
                    .iter()
                    .find(|thread| thread.id == recipient)
                    .map(|thread| thread.chat.clone());
                if let Some(recipient) = recipient {
                    self.settle_thread_waits(&recipient, cx);
                }
            }
        }
    }

    fn resolve_thread_request(
        thread_id: &ThreadId,
        request_id: &str,
        reply: ThreadReply,
        cx: &Context<'_, Self>,
    ) {
        let links = cx.global::<DesktopState>().app.thread_links().clone();
        if let Err(error) = links.resolve(thread_id, request_id, reply) {
            // The turn ended while this was answered; nobody is waiting.
            tracing::debug!("thread request answer was not delivered: {error}");
        }
    }

    /// `chat` changed: answer the calls waiting on a message it has now
    /// finished, and forget the calls its own ended turn was holding.
    pub(super) fn settle_thread_waits(
        &mut self,
        chat: &Entity<ChatView>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.thread_waits.is_empty() {
            return;
        }
        for (wait, reply) in self.due_thread_answers(chat, cx) {
            Self::resolve_thread_request(&wait.caller, &wait.request_id, reply, cx);
        }
    }

    /// Take the waits `chat` settles, each with its answer.
    fn due_thread_answers(
        &mut self,
        chat: &Entity<ChatView>,
        cx: &Context<'_, Self>,
    ) -> Vec<(ThreadWait, ThreadReply)> {
        let Some(id) = self
            .threads
            .iter()
            .find(|thread| thread.chat.entity_id() == chat.entity_id())
            .map(|thread| thread.id.clone())
        else {
            return Vec::new();
        };
        let view = chat.read(cx);
        if !view.is_streaming {
            // Its turn is over, so the broker already ended its calls.
            self.thread_waits.retain(|wait| wait.caller.as_str() != id);
        }
        let ids: Vec<&str> = self
            .threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect();
        let mut due = Vec::new();
        let mut waiting = Vec::new();
        for wait in std::mem::take(&mut self.thread_waits) {
            let answer = (wait.recipient == id)
                .then(|| view.thread_message_answer(&wait.prompt))
                .flatten();
            match answer {
                Some(answer) => {
                    let answer = match &answer {
                        ThreadMessageAnswer::Replied(text) => Answer::Replied(text),
                        ThreadMessageAnswer::Stopped => Answer::Stopped,
                        ThreadMessageAnswer::Failed(error) => Answer::Failed(error),
                        ThreadMessageAnswer::Removed => Answer::Removed,
                    };
                    let reply = answer_reply(&id, &view.thread_title, &ids, answer);
                    due.push((wait, reply));
                }
                None => waiting.push(wait),
            }
        }
        self.thread_waits = waiting;
        due
    }

    /// Thread `id` is being deleted: end the calls waiting on it, and forget
    /// the ones it was holding.
    pub(super) fn end_thread_waits_on(&mut self, id: &str, cx: &Context<'_, Self>) {
        if self.thread_waits.is_empty() {
            return;
        }
        let ids: Vec<&str> = self
            .threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect();
        let title = self
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .map(|thread| thread.chat.read(cx).thread_title.clone())
            .unwrap_or_default();
        let reply = answer_reply(id, &title, &ids, Answer::Removed);
        let (ended, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.thread_waits)
            .into_iter()
            .filter(|wait| wait.caller.as_str() != id)
            .partition(|wait| wait.recipient == id);
        self.thread_waits = kept;
        for wait in ended {
            Self::resolve_thread_request(&wait.caller, &wait.request_id, reply.clone(), cx);
        }
    }

    /// What the app does with `operation`, asked by the agent of `chat`.
    pub(super) fn thread_reply(
        &mut self,
        chat: &Entity<ChatView>,
        operation: &ThreadOperation,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ThreadAnswer {
        let Some(caller) = self
            .threads
            .iter()
            .find(|thread| thread.chat.entity_id() == chat.entity_id())
            .map(|thread| thread.id.clone())
        else {
            return ThreadAnswer::Now(ThreadReply::refused(
                "This thread no longer exists in the app.",
            ));
        };
        match operation {
            ThreadOperation::List => ThreadAnswer::Now(self.thread_list_reply(&caller, cx)),
            ThreadOperation::Read { thread_id, limit } => {
                ThreadAnswer::Now(self.thread_read_reply(thread_id, *limit, cx))
            }
            ThreadOperation::Message {
                thread_id,
                title,
                message,
                wait,
            } => self.thread_message_reply(
                &caller,
                thread_id.as_deref(),
                title.as_deref(),
                message,
                *wait,
                window,
                cx,
            ),
        }
    }

    fn thread_list_reply(&self, caller: &str, cx: &Context<'_, Self>) -> ThreadReply {
        let mut recent: Vec<_> = self.threads.iter().collect();
        recent.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at_ms));
        let cards: Vec<ThreadCard<'_>> = recent
            .into_iter()
            .map(|thread| {
                let chat = thread.chat.read(cx);
                ThreadCard {
                    id: &thread.id,
                    title: &chat.thread_title,
                    project: Some(thread.project_name.as_str())
                        .filter(|name| !name.trim().is_empty()),
                    running: chat.is_streaming,
                    queued: chat.prompt_queue.for_thread(&thread.id).len(),
                }
            })
            .collect();
        list_reply(&cards, caller)
    }

    fn thread_read_reply(
        &self,
        reference: &str,
        limit: Option<u32>,
        cx: &Context<'_, Self>,
    ) -> ThreadReply {
        let ids: Vec<&str> = self
            .threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect();
        let id = match resolve_thread(reference, &ids) {
            Ok(id) => id,
            Err(refusal) => return refusal,
        };
        let Some(thread) = self.threads.iter().find(|thread| thread.id == id) else {
            return ThreadReply::refused("No thread has that id.");
        };
        let chat = thread.chat.read(cx);
        // The reply still being written is not a settled message.
        let settled = if chat.is_streaming
            && chat
                .messages
                .last()
                .is_some_and(|message| message.role == MessageRole::Assistant)
        {
            &chat.messages[..chat.messages.len() - 1]
        } else {
            &chat.messages[..]
        };
        let lines: Vec<ThreadLine<'_>> = settled
            .iter()
            .map(|message| ThreadLine {
                from_user: message.role == MessageRole::User,
                text: &message.content,
            })
            .collect();
        read_reply(&chat.thread_title, chat.is_streaming, &lines, limit)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the fields of one message_thread call, plus the window to create a thread in"
    )]
    fn thread_message_reply(
        &mut self,
        caller: &str,
        thread_id: Option<&str>,
        title: Option<&str>,
        message: &str,
        wants_wait: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ThreadAnswer {
        let Some(sender) = self.threads.iter().find(|thread| thread.id == caller) else {
            return ThreadAnswer::Now(ThreadReply::refused(
                "Not sent: this thread no longer exists in the app.",
            ));
        };
        let sender_chat = sender.chat.read(cx);
        // The prompt of the turn that is sending.
        let prompt = sender_chat
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.clone())
            .unwrap_or_default();
        let sender_title = sender_chat.thread_title.clone();
        let place = (
            sender.project_name.clone(),
            sender.project_path.clone(),
            sender_chat.model_id.clone(),
            sender_chat.worktree_path.clone(),
            sender_chat.run_locked,
            sender.branch.clone(),
        );
        let ids: Vec<String> = self
            .threads
            .iter()
            .map(|thread| thread.id.clone())
            .collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();

        // A wait needs a recipient that can answer inside this call: one
        // that is not itself waiting (that could never finish), and whose
        // queue is not paused (that would hold the call until the user
        // resumes it).
        let mut wait = wants_wait;
        if let Some(known) = thread_id
            .filter(|_| wants_wait)
            .and_then(|reference| resolve_thread(reference, &id_refs).ok())
            .and_then(|id| self.threads.iter().find(|thread| thread.id == id))
        {
            let chat = known.chat.read(cx);
            if self
                .thread_waits
                .iter()
                .any(|held| held.caller.as_str() == known.id)
            {
                return ThreadAnswer::Now(recipient_waiting_reply(
                    &known.id,
                    &chat.thread_title,
                    &id_refs,
                ));
            }
            wait = !chat.queue_held;
        }

        let Delivery { recipient, prompt } = match plan_message(&Outgoing {
            caller_id: caller,
            caller_title: &sender_title,
            caller_prompt: &prompt,
            ids: &id_refs,
            thread_id,
            title,
            message,
            wait,
            max_hops: self.thread_hops,
        }) {
            Ok(delivery) => delivery,
            Err(refusal) => return ThreadAnswer::Now(refusal),
        };
        let (id, created) = match recipient {
            Recipient::Existing(id) => (id, false),
            Recipient::New { title } => {
                // Runs where the sender runs: same project, worktree and
                // model. It joins the sidebar without taking the view.
                let (project_name, project_path, model_id, worktree_path, run_locked, branch) =
                    place;
                let mut thread = Self::create_thread_with_title(
                    project_name,
                    project_path,
                    model_id,
                    self.permission_mode,
                    &title,
                    window,
                    cx,
                );
                thread.branch = branch;
                self.stamp_chat_run_prefs(&thread.chat, cx);
                thread.chat.update(cx, |chat, _| {
                    chat.worktree_path = worktree_path;
                    chat.run_locked = run_locked;
                });
                let id = thread.id.clone();
                self.threads.push(thread);
                (id, true)
            }
        };
        let Some(recipient) = self.threads.iter().find(|thread| thread.id == id) else {
            return ThreadAnswer::Now(ThreadReply::refused(
                "Not sent: that thread no longer exists.",
            ));
        };
        let chat = recipient.chat.clone();
        let started = chat.update(cx, |chat, cx| chat.accept_thread_message(&prompt, cx));
        self.persist_and_notify(cx);
        if wait {
            return ThreadAnswer::Later {
                recipient: id,
                prompt,
            };
        }
        let title = chat.read(cx).thread_title.clone();
        let mut all: Vec<&str> = id_refs;
        if created {
            all.push(&id);
        }
        let state = match (started, wants_wait) {
            (true, _) => DeliveryState::Started,
            (false, true) => DeliveryState::QueuedNotAwaited,
            (false, false) => DeliveryState::Queued,
        };
        ThreadAnswer::Now(delivered_reply(&id, &title, &all, created, state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use crate::views::chat::{Message, TurnState};
    use bruhs_models::thread_link::{delivery_prompt, parse_delivery, prompt_hops, MAX_HOPS};
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let captured = Rc::new(RefCell::new(None));
        let capture = captured.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some(workspace.clone());
            gpui_component::Root::new(workspace, window, cx)
        });
        let workspace = captured.borrow().clone().expect("workspace");
        cx.run_until_parked();
        (workspace, cx)
    }

    /// Two threads, "Sender" and "Recipient". The recipient looks busy, so a
    /// message to it queues instead of starting a real provider turn.
    fn two_threads(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> (Entity<ChatView>, String, Entity<ChatView>, String) {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.threads.clear();
            for title in ["Sender", "Recipient"] {
                let thread = Workspace::create_thread_with_title(
                    String::new(),
                    String::new(),
                    workspace.current_model.clone(),
                    workspace.permission_mode,
                    title,
                    window,
                    cx,
                );
                workspace.threads.push(thread);
            }
            let sender = &workspace.threads[0];
            let recipient = &workspace.threads[1];
            recipient
                .chat
                .update(cx, |chat, _| chat.is_streaming = true);
            (
                sender.chat.clone(),
                sender.id.clone(),
                recipient.chat.clone(),
                recipient.id.clone(),
            )
        })
    }

    fn ask(
        workspace: &Entity<Workspace>,
        chat: &Entity<ChatView>,
        operation: ThreadOperation,
        cx: &mut VisualTestContext,
    ) -> ThreadAnswer {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.thread_reply(chat, &operation, window, cx)
        })
    }

    fn now(answer: ThreadAnswer) -> ThreadReply {
        match answer {
            ThreadAnswer::Now(reply) => reply,
            ThreadAnswer::Later { .. } => panic!("expected an answer now"),
        }
    }

    fn message(to: Option<&str>, text: &str, wait: bool) -> ThreadOperation {
        ThreadOperation::Message {
            thread_id: to.map(str::to_string),
            title: None,
            message: text.into(),
            wait,
        }
    }

    fn reply(text: &str, state: TurnState) -> Message {
        let mut message = Message::assistant_streaming(0, None);
        message.content = text.to_string();
        message.state = state;
        message
    }

    /// Hold a call on `recipient`'s answer to `prompt`, as a waiting request
    /// from `caller` would.
    fn hold(
        workspace: &Entity<Workspace>,
        caller: &str,
        recipient: &str,
        prompt: &str,
        cx: &mut VisualTestContext,
    ) {
        workspace.update(cx, |workspace, _| {
            workspace.thread_waits.push(ThreadWait {
                request_id: "ask-1".into(),
                caller: ThreadId::try_from(caller).unwrap(),
                recipient: recipient.into(),
                prompt: prompt.into(),
            });
        });
    }

    fn due(
        workspace: &Entity<Workspace>,
        chat: &Entity<ChatView>,
        cx: &mut VisualTestContext,
    ) -> Vec<ThreadReply> {
        workspace.update(cx, |workspace, cx| {
            workspace
                .due_thread_answers(chat, cx)
                .into_iter()
                .map(|(_, reply)| reply)
                .collect()
        })
    }

    #[gpui::test]
    fn an_agent_lists_and_reads_the_other_threads(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, sender_id, recipient, recipient_id) = two_threads(&workspace, cx);
        recipient.update(cx, |chat, _| {
            chat.messages.push(Message::user("What broke?"));
            chat.messages
                .push(reply("The parser.", TurnState::Finished));
            // The reply of the running turn must not be read as settled.
            chat.messages
                .push(reply("still typing", TurnState::Streaming));
        });

        let listed = now(ask(&workspace, &sender, ThreadOperation::List, cx));
        assert!(listed.ok);
        assert!(listed.text.contains(&format!(
            "{} · Sender · no project · this thread",
            &sender_id[..8]
        )));
        assert!(listed.text.contains(&format!(
            "{} · Recipient · no project · running",
            &recipient_id[..8]
        )));

        let read = now(ask(
            &workspace,
            &sender,
            ThreadOperation::Read {
                thread_id: recipient_id[..8].to_string(),
                limit: None,
            },
            cx,
        ));
        assert!(read.ok && read.text.contains("Last 2 of 2 messages"));
        assert!(read.text.contains("User:\nWhat broke?") && read.text.contains("The parser."));
        assert!(!read.text.contains("still typing"));
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_message_to_a_busy_thread_waits_in_its_queue(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, sender_id, recipient, recipient_id) = two_threads(&workspace, cx);

        let sent = now(ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "Report what broke.", false),
            cx,
        ));
        assert!(sent.ok, "{}", sent.text);
        assert!(sent.text.contains("\"Recipient\". It is queued"));
        let queued = recipient.read_with(cx, |chat, _| chat.prompt_queue.for_thread(&recipient_id));
        assert_eq!(queued.len(), 1);
        assert_eq!(prompt_hops(&queued[0].text), 1);
        assert!(queued[0].text.contains(&format!(
            "[Message from thread \"Sender\" (id {})",
            &sender_id[..8]
        )));
        assert!(queued[0].text.ends_with("Report what broke."));
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_thread_cannot_message_itself_an_unknown_thread_or_past_the_users_limit(
        cx: &mut TestAppContext,
    ) {
        let (workspace, cx) = fixture(cx);
        let (sender, sender_id, recipient, recipient_id) = two_threads(&workspace, cx);

        let own = now(ask(
            &workspace,
            &sender,
            message(Some(&sender_id), "hi", false),
            cx,
        ));
        assert!(!own.ok && own.text.contains("this thread"));
        let unknown = now(ask(
            &workspace,
            &sender,
            message(Some("ffffffff"), "hi", false),
            cx,
        ));
        assert!(!unknown.ok && unknown.text.contains("No thread"));

        // The sender's own turn was the last allowed hop of a chain.
        sender.update(cx, |chat, _| {
            chat.messages.push(Message::user(delivery_prompt(
                "Recipient",
                &recipient_id[..8],
                MAX_HOPS,
                MAX_HOPS,
                false,
                "ping",
            )));
        });
        let limited = now(ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "pong", false),
            cx,
        ));
        assert!(!limited.ok && limited.text.contains("limit of 6"));
        assert!(recipient.read_with(cx, |chat, _| chat.prompt_queue.is_empty()));

        // The limit is the user's setting.
        workspace.update(cx, |workspace, _| workspace.thread_hops = 10);
        let allowed = now(ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "pong", false),
            cx,
        ));
        assert!(allowed.ok, "{}", allowed.text);
        let queued = recipient.read_with(cx, |chat, _| chat.prompt_queue.for_thread(&recipient_id));
        assert!(queued[0].text.contains("hop 7/10."));
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_waiting_call_is_held_until_the_recipient_has_answered_its_message(
        cx: &mut TestAppContext,
    ) {
        let (workspace, cx) = fixture(cx);
        let (sender, _sender_id, recipient, recipient_id) = two_threads(&workspace, cx);

        let answer = ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "What is 2 + 2?", true),
            cx,
        );
        let ThreadAnswer::Later {
            recipient: held_on,
            prompt,
        } = answer
        else {
            panic!("expected the call to be held");
        };
        assert_eq!(held_on, recipient_id);
        let view = parse_delivery(&prompt).expect("a delivery header");
        assert!(view.awaited && view.body == "What is 2 + 2?");
        workspace.update(cx, |workspace, _| {
            workspace.thread_waits.push(ThreadWait {
                request_id: "ask-1".into(),
                caller: ThreadId::new_v4(),
                recipient: recipient_id.clone(),
                prompt: prompt.clone(),
            });
        });

        // Still queued behind the recipient's running turn: nothing is due.
        assert!(due(&workspace, &recipient, cx).is_empty());
        // That turn ends (an earlier prompt's reply); ours is still queued.
        recipient.update(cx, |chat, _| {
            chat.messages.push(Message::user("an earlier prompt"));
            chat.messages
                .push(reply("earlier reply", TurnState::Finished));
            chat.is_streaming = false;
        });
        assert!(due(&workspace, &recipient, cx).is_empty());
        // Our message runs: its reply is streaming, so still nothing.
        recipient.update(cx, |chat, _| {
            let next = chat.prompt_queue.next_for(&recipient_id).unwrap();
            chat.messages.push(Message::user(next.text));
            chat.messages.push(reply("thinking", TurnState::Streaming));
            chat.is_streaming = true;
        });
        assert!(due(&workspace, &recipient, cx).is_empty());
        // It finishes: the call gets that reply, once.
        recipient.update(cx, |chat, _| {
            let last = chat.messages.last_mut().unwrap();
            last.content = "4".into();
            last.state = TurnState::Finished;
            chat.is_streaming = false;
        });
        let answers = due(&workspace, &recipient, cx);
        assert_eq!(answers.len(), 1);
        assert_eq!(
            answers[0],
            ThreadReply::ok(format!(
                "Thread {} \"Recipient\" answered:\n\n4",
                &recipient_id[..8]
            ))
        );
        assert!(due(&workspace, &recipient, cx).is_empty());
        assert!(workspace.read_with(cx, |workspace, _| workspace.thread_waits.is_empty()));
    }

    #[gpui::test]
    fn a_waiting_call_learns_why_no_answer_came(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (_sender, _sender_id, recipient, recipient_id) = two_threads(&workspace, cx);
        let caller = ThreadId::new_v4();
        recipient.update(cx, |chat, _| chat.is_streaming = false);

        // The message never ran: it is neither queued nor in the transcript.
        hold(&workspace, caller.as_str(), &recipient_id, "gone", cx);
        let answers = due(&workspace, &recipient, cx);
        assert!(!answers[0].ok && answers[0].text.contains("never ran the message"));

        for (state, said) in [
            (TurnState::Stopped, "stopped by the user"),
            (
                TurnState::Failed {
                    error: "rate limited".into(),
                },
                "could not answer: rate limited",
            ),
        ] {
            recipient.update(cx, |chat, _| {
                chat.messages.clear();
                chat.messages.push(Message::user("ask"));
                chat.messages.push(reply("partial", state.clone()));
            });
            hold(&workspace, caller.as_str(), &recipient_id, "ask", cx);
            let answers = due(&workspace, &recipient, cx);
            assert!(
                !answers[0].ok && answers[0].text.contains(said),
                "{}",
                answers[0].text
            );
        }

        // Deleting the recipient ends the call; the caller's own waits go
        // when the caller is deleted.
        hold(&workspace, caller.as_str(), &recipient_id, "ask", cx);
        workspace.update(cx, |workspace, cx| {
            workspace.end_thread_waits_on(&recipient_id, cx)
        });
        assert!(workspace.read_with(cx, |workspace, _| workspace.thread_waits.is_empty()));
        hold(&workspace, &recipient_id, "another-thread", "ask", cx);
        workspace.update(cx, |workspace, cx| {
            workspace.end_thread_waits_on(&recipient_id, cx)
        });
        assert!(workspace.read_with(cx, |workspace, _| workspace.thread_waits.is_empty()));
    }

    #[gpui::test]
    fn a_call_cannot_wait_on_a_thread_that_is_waiting_or_paused(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, sender_id, recipient, recipient_id) = two_threads(&workspace, cx);

        // The recipient's own turn is held on another thread: a second wait
        // on it could never finish.
        hold(&workspace, &recipient_id, &sender_id, "earlier", cx);
        let blocked = now(ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "hi", true),
            cx,
        ));
        assert!(!blocked.ok && blocked.text.contains("Send again without wait"));
        assert!(recipient.read_with(cx, |chat, _| chat.prompt_queue.is_empty()));
        // Without wait the same message goes through.
        let sent = now(ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "hi", false),
            cx,
        ));
        assert!(sent.ok, "{}", sent.text);

        // A paused queue: delivered, not awaited, and the header says so.
        workspace.update(cx, |workspace, _| workspace.thread_waits.clear());
        recipient.update(cx, |chat, _| {
            chat.prompt_queue = Default::default();
            chat.is_streaming = false;
            chat.queue_held = true;
        });
        let paused = now(ask(
            &workspace,
            &sender,
            message(Some(&recipient_id), "later", true),
            cx,
        ));
        assert!(paused.ok && paused.text.contains("could not wait for it"));
        let queued = recipient.read_with(cx, |chat, _| chat.prompt_queue.for_thread(&recipient_id));
        assert_eq!(queued.len(), 1);
        assert!(!parse_delivery(&queued[0].text).unwrap().awaited);
    }

    #[gpui::test]
    fn the_settings_reach_every_thread_and_only_non_defaults_are_saved(cx: &mut TestAppContext) {
        use super::super::settings_page_ui::{
            SETTINGS_THREAD_ACCESS_DROPDOWN_ID, SETTINGS_THREAD_HOPS_DROPDOWN_ID,
        };
        use bruhs_models::thread_link::ThreadAccess;

        let (workspace, cx) = fixture(cx);
        let (sender, _sender_id, recipient, _recipient_id) = two_threads(&workspace, cx);
        recipient.update(cx, |chat, _| chat.is_streaming = false);
        workspace.read_with(cx, |workspace, cx| {
            let saved = workspace.snapshot(cx);
            assert_eq!((saved.thread_access, saved.thread_hops), (None, None));
            assert_eq!(workspace.thread_hops, MAX_HOPS);
        });

        workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_settings_option(
                SETTINGS_THREAD_ACCESS_DROPDOWN_ID,
                "read",
                window,
                cx,
            );
            workspace.select_settings_option(SETTINGS_THREAD_HOPS_DROPDOWN_ID, "10", window, cx);
        });
        for chat in [&sender, &recipient] {
            assert_eq!(
                chat.read_with(cx, |chat, _| chat.thread_access),
                ThreadAccess::Read
            );
        }
        workspace.read_with(cx, |workspace, cx| {
            let saved = workspace.snapshot(cx);
            assert_eq!(saved.thread_access.as_deref(), Some("read"));
            assert_eq!(saved.thread_hops, Some(10));
        });

        // Back to the defaults: nothing is written, so older files stay as
        // they were.
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_settings_option(
                SETTINGS_THREAD_ACCESS_DROPDOWN_ID,
                "message",
                window,
                cx,
            );
            workspace.select_settings_option(SETTINGS_THREAD_HOPS_DROPDOWN_ID, "6", window, cx);
        });
        workspace.read_with(cx, |workspace, cx| {
            let saved = workspace.snapshot(cx);
            assert_eq!((saved.thread_access, saved.thread_hops), (None, None));
        });
        assert_eq!(
            sender.read_with(cx, |chat, _| chat.thread_access),
            ThreadAccess::Message
        );
    }
}
