//! Answers an agent's requests about other threads: list them, read one,
//! send one a message. The rules and reply text are shared with the web in
//! `bruhs_models::thread_link`.
//!
//! A message goes through the recipient's own chat: it starts a turn there
//! at once when that thread is free, and otherwise waits in its prompt
//! queue. Threads run side by side here, so the view never moves, and a
//! sender may wait: its tool call is held ([`ThreadWait`]) until the
//! recipient's turn for that message ends, and gets that turn's reply.
//!
//! `message_bruh` hands work to a bruh instead. A bruh's chat lives in the
//! background service, so the message goes there as a `Chat` with a run id
//! picked here, and a sender that waits is held ([`BruhWait`]) while that
//! run is read on every crew change and every [`BRUH_WAIT_POLL_MS`].

use super::Workspace;
use crate::app_state::DesktopState;
use crate::views::chat::{ChatView, MessageRole, ThreadMessageAnswer};
use bruhs_models::ids::ThreadId;
use bruhs_models::reactive::{ReactiveCommand, ReactiveRun};
use bruhs_models::thread_link::{
    answer_reply, bruh_sent_reply, bruh_service_refusal, delivered_reply, due_bruh_answers,
    forget_bruh_waits_of, list_reply, pick_role, plan_bruh_message, plan_message, read_reply,
    recipient_waiting_reply, resolve_thread, same_turn, Answer, BruhOutgoing, BruhWait, Created,
    Delivery, DeliveryState, Outgoing, Recipient, ThreadCard, ThreadLine, ThreadOperation,
    ThreadReply, ThreadRequest, BRUH_WAIT_POLL_MS,
};
use gpui::{Context, Entity, Task, Window};

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

/// The held `message_bruh` calls, and whether a loop reads them.
#[derive(Default)]
pub(super) struct BruhWaits {
    pub(super) waits: Vec<BruhWait>,
    polling: bool,
    /// A check is in flight; one at a time, so a page read earlier never
    /// lands after a newer one.
    checking: bool,
    /// A check was asked for during the one in flight: run once more.
    recheck: bool,
}

#[cfg(test)]
thread_local! {
    /// Every answer handed to the broker, for tests: no turn waits there.
    static RESOLVED: std::cell::RefCell<Vec<(String, ThreadReply)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The answers handed to the broker since the last call.
#[cfg(test)]
pub(super) fn take_resolved_for_test() -> Vec<(String, ThreadReply)> {
    RESOLVED.with(|resolved| std::mem::take(&mut *resolved.borrow_mut()))
}

/// One `message_thread` call's arguments.
#[derive(Clone, Copy)]
struct Outbound<'a> {
    thread_id: Option<&'a str>,
    title: Option<&'a str>,
    message: &'a str,
    /// Name of the role a new thread should start with.
    role: Option<&'a str>,
    wait: bool,
}

/// One `message_bruh` call's arguments.
#[derive(Clone, Copy)]
struct BruhCall<'a> {
    name_or_id: &'a str,
    message: &'a str,
    wait: bool,
    /// Set by the broker: the calling turn runs with Full access.
    caller_full_access: bool,
}

/// What the app does with one request.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ThreadAnswer {
    /// Answer the tool call now.
    Now(ThreadReply),
    /// Hold the tool call until `recipient` has answered `prompt`.
    Later { recipient: String, prompt: String },
    /// A task answers the tool call through `resolve_thread_request`.
    Pending,
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
        if let ThreadOperation::Whiteboard {
            turn_id,
            request: operation,
        } = &request.operation
        {
            let Some(project) = self.whiteboard_request_project(chat, cx) else {
                Self::resolve_thread_request(
                    &request.thread_id,
                    &request.id,
                    ThreadReply::refused(
                        "Attach a project folder before using whiteboard agent tools.",
                    ),
                    cx,
                );
                return;
            };
            let library = self.whiteboard_library(project, window, cx);
            let reply = library.update(cx, |library, cx| {
                library.answer(
                    operation.clone(),
                    bruhs_models::whiteboard::BoardRequester {
                        thread_id: request.thread_id.to_string(),
                        turn_id: turn_id.clone(),
                    },
                    cx,
                )
            });
            let thread_id = request.thread_id.clone();
            let id = request.id.clone();
            cx.spawn(async move |this, cx| {
                let reply = reply.await;
                let _ = this.update(cx, |_, cx| {
                    Self::resolve_thread_request(&thread_id, &id, reply, cx)
                });
            })
            .detach();
            return;
        }
        match self.thread_reply(chat, &request.operation, &request.id, window, cx) {
            ThreadAnswer::Pending => {}
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
        #[cfg(test)]
        RESOLVED.with(|resolved| {
            resolved
                .borrow_mut()
                .push((request_id.to_string(), reply.clone()));
        });
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
        if !self.bruh_waits.waits.is_empty() && !chat.read(cx).is_streaming {
            // Its turn is over, so the broker already ended its calls.
            if let Some(thread) = self
                .threads
                .iter()
                .find(|thread| thread.chat.entity_id() == chat.entity_id())
            {
                forget_bruh_waits_of(&mut self.bruh_waits.waits, &thread.id);
            }
        }
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
        forget_bruh_waits_of(&mut self.bruh_waits.waits, id);
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
        request_id: &str,
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
            ThreadOperation::Whiteboard { .. } => ThreadAnswer::Now(ThreadReply::refused(
                "Whiteboard requests require their turn-scoped UI route.",
            )),
            ThreadOperation::List => ThreadAnswer::Now(self.thread_list_reply(&caller, cx)),
            ThreadOperation::Read { thread_id, limit } => {
                ThreadAnswer::Now(self.thread_read_reply(thread_id, *limit, cx))
            }
            ThreadOperation::Message {
                thread_id,
                title,
                message,
                wait,
                role,
            } => self.thread_message_reply(
                &caller,
                &Outbound {
                    thread_id: thread_id.as_deref(),
                    title: title.as_deref(),
                    message,
                    role: role.as_deref(),
                    wait: *wait,
                },
                window,
                cx,
            ),
            ThreadOperation::MessageBruh {
                name_or_id,
                message,
                wait,
                caller_full_access,
            } => self.bruh_message_reply(
                &caller,
                &BruhCall {
                    name_or_id,
                    message,
                    wait: *wait,
                    caller_full_access: *caller_full_access,
                },
                request_id,
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
                    model: Some(chat.model_id.as_str()),
                    // A thread whose role was deleted reads as having none.
                    role: chat
                        .role
                        .as_deref()
                        .and_then(|id| self.roles.find(id))
                        .map(|role| role.name.as_str()),
                }
            })
            .collect();
        // Roles as they run in the caller's project.
        let project = self
            .threads
            .iter()
            .find(|thread| thread.id == caller)
            .map(|thread| thread.project_name.as_str())
            .unwrap_or_default();
        list_reply(
            &cards,
            caller,
            &self.roles.resolved(project),
            self.reactive_automations.read(cx).crew(),
        )
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

    fn thread_message_reply(
        &mut self,
        caller: &str,
        outbound: &Outbound<'_>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> ThreadAnswer {
        let Outbound {
            thread_id,
            title,
            message,
            role: wanted_role,
            wait: wants_wait,
        } = *outbound;
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
        // The role a new thread starts with, as it runs in this project.
        let role = match pick_role(&self.roles, &sender.project_name, wanted_role) {
            Ok(role) => role,
            Err(refusal) => return ThreadAnswer::Now(refusal),
        };
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
                // Runs where the sender runs: same project and worktree, and
                // the role's model or else the sender's. It joins the
                // sidebar without taking the view.
                let (project_name, project_path, model_id, worktree_path, run_locked, branch) =
                    place;
                let model_id = role
                    .as_ref()
                    .and_then(|role| role.model.clone())
                    .unwrap_or(model_id);
                let role_id = role.as_ref().map(|role| role.id.clone());
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
                // Before the prefs are stamped: they resolve the role.
                thread.chat.update(cx, |chat, _| {
                    chat.role = role_id;
                    chat.worktree_path = worktree_path;
                    chat.run_locked = run_locked;
                });
                self.stamp_chat_run_prefs(&thread.chat, cx);
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
        let started_thread = created.then(|| Created {
            role: role.as_ref().map(|role| role.name.as_str()),
        });
        ThreadAnswer::Now(delivered_reply(&id, &title, &all, started_thread, state))
    }

    /// Send a `message_bruh` to the bruh's chat. A task answers the call
    /// once the service took it, or holds it for the bruh's reply.
    fn bruh_message_reply(
        &mut self,
        caller: &str,
        call: &BruhCall<'_>,
        request_id: &str,
        cx: &mut Context<'_, Self>,
    ) -> ThreadAnswer {
        let BruhCall {
            name_or_id,
            message,
            wait,
            caller_full_access,
        } = *call;
        let panel = self.reactive_automations.clone();
        if let Some(reason) = bruh_service_refusal(panel.read(cx).crew_protocol()) {
            return ThreadAnswer::Now(ThreadReply::refused(reason));
        }
        let (Ok(caller_id), Some(sender)) = (
            ThreadId::try_from(caller),
            self.threads.iter().find(|thread| thread.id == caller),
        ) else {
            return ThreadAnswer::Now(ThreadReply::refused(
                "Not sent: this thread no longer exists in the app.",
            ));
        };
        let sender_chat = sender.chat.read(cx);
        let turn = sender_chat.streaming_turn();
        // The prompt of the turn that is sending.
        let prompt = sender_chat
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
            .unwrap_or_default();
        let ids: Vec<&str> = self
            .threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect();
        let plan = match plan_bruh_message(
            &BruhOutgoing {
                caller_id: caller,
                caller_title: &sender_chat.thread_title,
                caller_prompt: prompt,
                ids: &ids,
                name_or_id,
                message,
                wait,
                max_hops: self.thread_hops,
                caller_full_access,
            },
            panel.read(cx).crew(),
        ) {
            Ok(plan) => plan,
            Err(refusal) => return ThreadAnswer::Now(refusal),
        };
        let run_id = uuid::Uuid::new_v4().to_string();
        let sending = panel.update(cx, |panel, cx| {
            panel.send_now(
                ReactiveCommand::Chat {
                    agent_id: plan.agent_id.clone(),
                    message: plan.prompt,
                    run_id: Some(run_id.clone()),
                },
                cx,
            )
        });
        let request_id = request_id.to_string();
        cx.spawn(async move |this, cx| {
            let sent = sending.await;
            let _ = this.update(cx, |this, cx| match sent {
                Err(error) => Self::resolve_thread_request(
                    &caller_id,
                    &request_id,
                    ThreadReply::refused(format!("Not sent: {error}")),
                    cx,
                ),
                Ok(()) if !wait => Self::resolve_thread_request(
                    &caller_id,
                    &request_id,
                    bruh_sent_reply(&plan.name),
                    cx,
                ),
                // The turn ended during the send, or another began, so the
                // broker has ended the call already, as it does for thread
                // waits.
                Ok(()) if !same_turn(turn, this.caller_turn(&caller_id, cx)) => {}
                Ok(()) => {
                    this.bruh_waits.waits.push(BruhWait {
                        request_id,
                        caller: caller_id,
                        agent_id: plan.agent_id,
                        name: plan.name,
                        run_id,
                        seen: false,
                    });
                    this.poll_bruh_waits(cx);
                }
            });
        })
        .detach();
        ThreadAnswer::Pending
    }

    /// The turn `caller` is streaming, if any.
    fn caller_turn(&self, caller: &ThreadId, cx: &Context<'_, Self>) -> Option<u64> {
        self.threads
            .iter()
            .find(|thread| thread.id == caller.as_str())
            .and_then(|thread| thread.chat.read(cx).streaming_turn())
    }

    /// Read the held bruh runs every [`BRUH_WAIT_POLL_MS`] while any wait.
    fn poll_bruh_waits(&mut self, cx: &mut Context<'_, Self>) {
        if self.bruh_waits.polling {
            return;
        }
        self.bruh_waits.polling = true;
        let every = std::time::Duration::from_millis(u64::from(BRUH_WAIT_POLL_MS));
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(every).await;
            let waiting = this
                .update(cx, |this, cx| {
                    if this.bruh_waits.waits.is_empty() {
                        this.bruh_waits.polling = false;
                        return false;
                    }
                    this.check_bruh_waits(cx);
                    true
                })
                .unwrap_or(false);
            if !waiting {
                break;
            }
        })
        .detach();
    }

    /// Read the newest runs of each bruh a call waits on, and answer the
    /// calls whose wait is over. One check runs at a time; one asked for
    /// meanwhile runs once more after it.
    pub(super) fn check_bruh_waits(&mut self, cx: &mut Context<'_, Self>) {
        if self.bruh_waits.waits.is_empty() {
            return;
        }
        if self.bruh_waits.checking {
            self.bruh_waits.recheck = true;
            return;
        }
        self.bruh_waits.checking = true;
        self.bruh_waits.recheck = false;
        let mut agents: Vec<String> = self
            .bruh_waits
            .waits
            .iter()
            .map(|wait| wait.agent_id.clone())
            .collect();
        agents.sort();
        agents.dedup();
        // Tests never reach a server: they settle waits with their own pages.
        let reading: Task<Vec<(String, Vec<ReactiveRun>)>> = if cfg!(test) {
            Task::ready(Vec::new())
        } else {
            let client = self.reactive_automations.read(cx).client();
            crate::app_state::spawn_tokio(cx, async move {
                let mut pages = Vec::new();
                for agent_id in agents {
                    match client.agent_runs(&agent_id, None).await {
                        Ok(runs) => pages.push((agent_id, runs)),
                        // Read again on the next tick.
                        Err(error) => tracing::debug!("bruh runs could not be read: {error}"),
                    }
                }
                pages
            })
        };
        cx.spawn(async move |this, cx| {
            let pages = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.settle_bruh_waits(&pages, cx);
                this.bruh_waits.checking = false;
                if std::mem::take(&mut this.bruh_waits.recheck) {
                    this.check_bruh_waits(cx);
                }
            });
        })
        .detach();
    }

    /// Answer the held bruh calls whose wait `pages` shows is over.
    fn settle_bruh_waits(
        &mut self,
        pages: &[(String, Vec<ReactiveRun>)],
        cx: &mut Context<'_, Self>,
    ) {
        let due = due_bruh_answers(
            &mut self.bruh_waits.waits,
            self.reactive_automations.read(cx).crew(),
            pages,
        );
        for (wait, reply) in due {
            Self::resolve_thread_request(&wait.caller, &wait.request_id, reply, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use crate::views::chat::{ChatViewEvent, Message, TurnState};
    use bruhs_models::thread_link::{delivery_prompt, parse_delivery, prompt_hops, MAX_HOPS};
    use gpui::{AppContext, TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let captured = Rc::new(RefCell::new(None));
        let capture = captured.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some(workspace.clone());
            bruhs_ui::Root::new(workspace, window, cx)
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
            workspace.thread_reply(chat, &operation, "ask-1", window, cx)
        })
    }

    fn now(answer: ThreadAnswer) -> ThreadReply {
        match answer {
            ThreadAnswer::Now(reply) => reply,
            ThreadAnswer::Later { .. } | ThreadAnswer::Pending => {
                panic!("expected an answer now")
            }
        }
    }

    fn message(to: Option<&str>, text: &str, wait: bool) -> ThreadOperation {
        ThreadOperation::Message {
            thread_id: to.map(str::to_string),
            title: None,
            message: text.into(),
            wait,
            role: None,
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

    #[gpui::test]
    fn a_new_thread_starts_with_the_role_an_agent_names(cx: &mut TestAppContext) {
        use bruhs_models::agent::PermissionMode;

        let (workspace, cx) = fixture(cx);
        let (sender, _sender_id, recipient, recipient_id) = two_threads(&workspace, cx);
        workspace.update(cx, |workspace, _| {
            let writer = workspace.roles.add("Writer").unwrap();
            workspace.roles.update(&writer, |role| {
                role.use_for = "drafting prose".into();
                role.model = Some("claude-haiku-4-5".into());
                role.effort = Some("high".into());
                role.access = Some(PermissionMode::Plan);
                role.instructions = "Write plainly.".into();
            });
            // Held, so the new thread queues its message instead of starting
            // a real provider turn.
            workspace.permission_mode = PermissionMode::AcceptEdits;
        });

        // The list names each thread's model and role, and the roles on offer.
        let listed = now(ask(&workspace, &sender, ThreadOperation::List, cx));
        assert!(listed.text.contains(" · no role\n"));
        assert!(listed
            .text
            .contains("Writer · Claude Haiku 4.5 · drafting prose\n"));

        // A role only fits a new thread, and must exist.
        let unknown = now(ask(
            &workspace,
            &sender,
            ThreadOperation::Message {
                thread_id: None,
                title: Some("Draft".into()),
                message: "Write the intro.".into(),
                wait: false,
                role: Some("Reviewer".into()),
            },
            cx,
        ));
        assert!(!unknown.ok && unknown.text.contains("no role named \"Reviewer\""));
        assert!(unknown.text.contains("Writer · Claude Haiku 4.5"));
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.threads.len()),
            2
        );
        let _ = (recipient, recipient_id);
    }

    #[gpui::test]
    fn the_user_gives_their_own_thread_a_role(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, sender_id, _recipient, _recipient_id) = two_threads(&workspace, cx);
        let writer = workspace.update(cx, |workspace, cx| {
            let writer = workspace.roles.add("Writer").unwrap();
            workspace.roles.update(&writer, |role| {
                role.model = Some("claude-haiku-4-5".into());
                role.instructions = "Write plainly.".into();
            });
            workspace.active_thread_id = Some(sender_id.clone());
            cx.notify();
            writer
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-chat-header-role").is_none());

        // `/role writer`, as the composer sends it to the workspace.
        sender.update(cx, |_, cx| {
            cx.emit(ChatViewEvent::RoleCommand("writer".into()));
        });
        cx.run_until_parked();
        sender.read_with(cx, |chat, _| {
            assert_eq!(chat.role.as_deref(), Some(writer.as_str()));
            let profile = chat.role_profile.as_ref().expect("the role is resolved");
            assert_eq!(profile.instructions, "Write plainly.");
            assert_eq!(chat.model_id, "claude-haiku-4-5", "the role's model");
        });
        workspace.read_with(cx, |workspace, cx| {
            let saved = workspace.snapshot(cx);
            let thread = saved
                .threads
                .iter()
                .find(|thread| thread.id == sender_id)
                .expect("the thread is saved");
            assert_eq!(thread.role.as_deref(), Some(writer.as_str()));
            assert_eq!(thread.model_id, "claude-haiku-4-5");
        });

        // The header names the role right after the title, and the card
        // names it too.
        let title = cx
            .debug_bounds("desktop-chat-header-title")
            .expect("the title is drawn");
        let tag = cx
            .debug_bounds("desktop-chat-header-role")
            .expect("the role tag is drawn");
        let gap = f32::from(tag.left() - title.right());
        assert!((0.0..=16.0).contains(&gap), "tag follows the title: {gap}");
        let card: &'static str = Box::leak(
            format!(
                "desktop-thread-card-role-{}",
                crate::views::workspace::element_key(&sender_id)
            )
            .into_boxed_str(),
        );
        assert!(cx.debug_bounds(card).is_some(), "the card names the role");

        // A long title is cut short; the tag stays whole, left of the toggles.
        sender.update(cx, |chat, cx| {
            chat.thread_title = "A long title ".repeat(40);
            cx.notify();
        });
        cx.run_until_parked();
        let tag = cx
            .debug_bounds("desktop-chat-header-role")
            .expect("the role tag is drawn");
        let toggle = cx
            .debug_bounds("desktop-chat-diff-toggle")
            .expect("the Diff toggle is drawn");
        assert!(tag.right() <= toggle.left(), "{tag:?} {toggle:?}");
        assert!(f32::from(tag.size.width) > 30.0, "{tag:?}");

        // A name that matches no role changes nothing.
        workspace.update(cx, |workspace, cx| {
            workspace.apply_role_command(&sender, "Reviewer", cx);
        });
        assert_eq!(
            sender.read_with(cx, |chat, _| chat.role.clone()),
            Some(writer)
        );

        // `/role none` removes it. The thread keeps the model it is on.
        workspace.update(cx, |workspace, cx| {
            workspace.apply_role_command(&sender, "none", cx);
        });
        cx.run_until_parked();
        sender.read_with(cx, |chat, _| {
            assert!(chat.role.is_none() && chat.role_profile.is_none());
            assert_eq!(chat.model_id, "claude-haiku-4-5");
        });
        assert!(cx.debug_bounds("desktop-chat-header-role").is_none());
    }

    #[gpui::test]
    fn the_role_editor_follows_the_web_layout(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_settings(&crate::views::workspace::OpenSettings, window, cx);
            let id = workspace.roles.add("Writer");
            workspace.select_role(id, window, cx);
        });
        cx.run_until_parked();
        let drawn = |cx: &mut VisualTestContext, selector: &'static str| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is not drawn"))
        };
        let roles = drawn(cx, "desktop-settings-row-roles");
        let editor = drawn(cx, "desktop-settings-role-editor");
        let new_role = drawn(cx, "desktop-settings-role-new");
        let delete = drawn(cx, "desktop-settings-role-delete");
        let summary = drawn(cx, "desktop-settings-row-summary-model");
        let titles = drawn(cx, "desktop-settings-row-title-instructions");

        // Web order: the Roles row, the open role's fields, the task models,
        // then the title instructions.
        assert!(roles.bottom() <= editor.top(), "{roles:?} {editor:?}");
        assert!(editor.bottom() <= summary.top(), "{editor:?} {summary:?}");
        assert!(summary.bottom() <= titles.top(), "{summary:?} {titles:?}");
        // New role sits beside the dropdown in the Roles row; Delete role
        // closes the editor at the right. Both are the web's 32px buttons.
        assert!(new_role.top() >= roles.top() && new_role.bottom() <= roles.bottom());
        assert!(delete.bottom() <= editor.bottom() && delete.right() <= editor.right());
        assert!(delete.left() > editor.left() + editor.size.width / 2.0);
        assert_eq!(f32::from(new_role.size.height), 32.0);
        assert_eq!(f32::from(delete.size.height), 32.0);
        // No project is chosen, so its overrides are not shown.
        assert!(cx.debug_bounds("desktop-settings-role-override").is_none());
    }

    #[gpui::test]
    fn a_threads_role_sets_its_model_effort_access_and_instructions(cx: &mut TestAppContext) {
        use bruhs_models::agent::PermissionMode;

        let (workspace, cx) = fixture(cx);
        let (sender, _sender_id, _recipient, _recipient_id) = two_threads(&workspace, cx);
        let writer = workspace.update(cx, |workspace, cx| {
            let writer = workspace.roles.add("Writer").unwrap();
            workspace.roles.update(&writer, |role| {
                role.effort = Some("high".into());
                role.access = Some(PermissionMode::Plan);
                role.instructions = "Write plainly.".into();
            });
            workspace.permission_mode = PermissionMode::AcceptEdits;
            // An agent started this thread with the role.
            workspace.threads[0].chat.update(cx, |chat, _| {
                chat.role = Some(writer.clone());
            });
            workspace.stamp_run_prefs_on_threads(cx);
            writer
        });
        sender.read_with(cx, |chat, _| {
            let profile = chat.role_profile.as_ref().expect("the role is resolved");
            assert_eq!(profile.name, "Writer");
            assert_eq!(profile.effort.as_deref(), Some("high"));
            assert_eq!(profile.instructions, "Write plainly.");
            // The role narrows the user's Accept edits to Plan.
            assert_eq!(
                bruhs_models::roles::narrow_access(chat.permission_mode, profile.access),
                PermissionMode::Plan
            );
        });

        // Editing the role reaches the thread; deleting it frees the thread.
        workspace.update(cx, |workspace, cx| {
            workspace
                .roles
                .update(&writer, |role| role.instructions = "Be brief.".into());
            workspace.stamp_run_prefs_on_threads(cx);
        });
        assert_eq!(
            sender.read_with(cx, |chat, _| chat
                .role_profile
                .as_ref()
                .map(|role| role.instructions.clone())),
            Some("Be brief.".to_string())
        );
        workspace.update(cx, |workspace, cx| {
            workspace.roles.remove(&writer);
            workspace.stamp_run_prefs_on_threads(cx);
        });
        assert!(sender.read_with(cx, |chat, _| chat.role_profile.is_none()));

        // Roles and task models are saved only once there is something to save.
        workspace.read_with(cx, |workspace, cx| {
            let saved = workspace.snapshot(cx);
            assert!(saved.roles.is_none() && saved.task_models.is_none());
        });
        workspace.update(cx, |workspace, _| {
            workspace.roles.add("Executor");
            workspace.task_models.summary = Some("claude-haiku-4-5".into());
        });
        workspace.read_with(cx, |workspace, cx| {
            let saved = workspace.snapshot(cx);
            assert_eq!(saved.roles.as_ref().map(|book| book.roles.len()), Some(1));
            assert_eq!(
                saved.task_models.and_then(|models| models.summary),
                Some("claude-haiku-4-5".to_string())
            );
        });
    }

    fn bo() -> bruhs_models::crew::BruhState {
        bruhs_models::crew::BruhState {
            agent_id: "b0b0b0b0-0000-4000-8000-000000000000".into(),
            name: "Bo".into(),
            look: bruhs_models::crew::BruhLook::default(),
            access: bruhs_models::crew::BruhAccess::default(),
            effort: None,
            status: bruhs_models::crew::BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
            computer: false,
            vault: Vec::new(),
        }
    }

    fn message_bo(text: &str, wait: bool) -> ThreadOperation {
        ThreadOperation::MessageBruh {
            name_or_id: "Bo".into(),
            message: text.into(),
            wait,
            caller_full_access: false,
        }
    }

    fn sent_commands(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> Vec<bruhs_models::reactive::ReactiveCommand> {
        workspace.read_with(cx, |workspace, cx| {
            workspace
                .reactive_automations
                .read(cx)
                .sent_for_test()
                .to_vec()
        })
    }

    #[gpui::test]
    fn message_bruh_sends_a_chat_with_the_thread_header(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, _, recipient, _) = two_threads(&workspace, cx);
        workspace.update(cx, |workspace, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_crew_for_test(vec![bo()], bruhs_models::crew::CREW_PROTOCOL, cx)
            });
        });
        let answer = ask(&workspace, &sender, message_bo("check", false), cx);
        assert_eq!(answer, ThreadAnswer::Pending);
        cx.run_until_parked();

        let sent = sent_commands(&workspace, cx);
        assert_eq!(sent.len(), 1, "{sent:?}");
        let bruhs_models::reactive::ReactiveCommand::Chat {
            agent_id,
            message,
            run_id,
        } = &sent[0]
        else {
            panic!("expected a chat, got {:?}", sent[0]);
        };
        assert_eq!(agent_id, &bo().agent_id);
        assert!(run_id.is_some());
        let delivery = parse_delivery(message).expect("a thread header");
        assert_eq!(delivery.body, "check");
        assert_eq!(delivery.hops, 1);

        let replies = take_resolved_for_test();
        assert_eq!(replies.len(), 1, "{replies:?}");
        assert_eq!(replies[0].0, "ask-1");
        assert!(replies[0].1.ok);
        assert!(
            replies[0].1.text.starts_with("Sent to Bo."),
            "{}",
            replies[0].1.text
        );
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.bruh_waits.waits.is_empty())
        });
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_full_access_bruh_takes_messages_only_from_a_full_access_turn(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, _, recipient, _) = two_threads(&workspace, cx);
        let mut full = bo();
        full.access.level = bruhs_models::crew::AccessLevel::Full;
        workspace.update(cx, |workspace, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_crew_for_test(vec![full], bruhs_models::crew::CREW_PROTOCOL, cx)
            });
        });
        let refused = now(ask(&workspace, &sender, message_bo("deploy", false), cx));
        assert!(!refused.ok);
        assert_eq!(
            refused.text,
            "Not sent: Bo has Full access, and this thread doesn't. Ask the user to message Bo themselves, or to give this thread Full access."
        );
        cx.run_until_parked();
        assert!(sent_commands(&workspace, cx).is_empty());

        let from_full = ThreadOperation::MessageBruh {
            name_or_id: "Bo".into(),
            message: "deploy".into(),
            wait: false,
            caller_full_access: true,
        };
        assert_eq!(
            ask(&workspace, &sender, from_full, cx),
            ThreadAnswer::Pending
        );
        cx.run_until_parked();
        assert_eq!(sent_commands(&workspace, cx).len(), 1);
        let replies = take_resolved_for_test();
        assert!(replies.iter().any(|(_, reply)| reply.ok), "{replies:?}");
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn message_bruh_before_the_service_answers_says_so(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, _, recipient, _) = two_threads(&workspace, cx);
        let refused = now(ask(&workspace, &sender, message_bo("check", false), cx));
        assert!(!refused.ok);
        assert_eq!(
            refused.text,
            "Not sent: the app hasn't heard from the background service yet. Try again in a moment, and tell the user if it keeps happening."
        );
        cx.run_until_parked();
        assert!(sent_commands(&workspace, cx).is_empty());
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn message_bruh_on_an_older_service_is_refused(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, _, recipient, _) = two_threads(&workspace, cx);
        workspace.update(cx, |workspace, cx| {
            workspace
                .reactive_automations
                .update(cx, |panel, cx| panel.show_crew_for_test(vec![bo()], 1, cx));
        });
        let refused = now(ask(&workspace, &sender, message_bo("check", true), cx));
        assert!(!refused.ok);
        assert_eq!(
            refused.text,
            "Not sent: the background service is older and can't take messages for bruhs. Tell the user to update it."
        );
        cx.run_until_parked();
        assert!(sent_commands(&workspace, cx).is_empty());
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_waited_bruh_message_is_held_and_ends_with_its_callers_turn(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, sender_id, recipient, _) = two_threads(&workspace, cx);
        workspace.update(cx, |workspace, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_crew_for_test(vec![bo()], bruhs_models::crew::CREW_PROTOCOL, cx)
            });
        });
        sender.update(cx, |chat, _| chat.is_streaming = true);
        let answer = ask(&workspace, &sender, message_bo("check", true), cx);
        assert_eq!(answer, ThreadAnswer::Pending);
        cx.run_until_parked();
        assert!(take_resolved_for_test().is_empty(), "the call is held");
        let held = workspace.read_with(cx, |workspace, _| workspace.bruh_waits.waits.clone());
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].caller.as_str(), sender_id);
        assert_eq!(held[0].name, "Bo");
        let sent = sent_commands(&workspace, cx);
        let bruhs_models::reactive::ReactiveCommand::Chat {
            run_id, message, ..
        } = &sent[0]
        else {
            panic!("expected a chat");
        };
        assert_eq!(run_id.as_deref(), Some(held[0].run_id.as_str()));
        assert!(parse_delivery(message).is_some_and(|delivery| delivery.awaited));

        // The run ends: the held call gets the bruh's reply, once.
        let done: ReactiveRun = serde_json::from_value(serde_json::json!({
            "id": held[0].run_id, "agent_id": bo().agent_id, "source_id": null,
            "subscription_id": null, "event_id": null, "status": "completed",
            "prompt": "check", "thread_id": "t", "session_id": null, "created_at": 1,
            "updated_at": 1, "not_before": 1, "result": "All green.", "error": null,
        }))
        .unwrap();
        let page = vec![(bo().agent_id, vec![done])];
        workspace.update(cx, |workspace, cx| workspace.settle_bruh_waits(&page, cx));
        let replies = take_resolved_for_test();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].0, "ask-1");
        assert!(replies[0].1.ok && replies[0].1.text.starts_with("Bo answered:"));
        assert!(replies[0].1.text.contains("All green."));
        workspace.update(cx, |workspace, cx| workspace.settle_bruh_waits(&page, cx));
        assert!(take_resolved_for_test().is_empty());

        // Held again, then the caller's turn ends: the broker ended its call,
        // so the wait goes without an answer.
        let _ = ask(&workspace, &sender, message_bo("again", true), cx);
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.bruh_waits.waits.len()),
            1
        );
        sender.update(cx, |chat, _| chat.is_streaming = false);
        workspace.update(cx, |workspace, cx| {
            workspace.settle_thread_waits(&sender, cx)
        });
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.bruh_waits.waits.is_empty())
        });
        assert!(take_resolved_for_test().is_empty());
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_bruh_message_the_service_refuses_is_not_sent(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, _, recipient, _) = two_threads(&workspace, cx);
        workspace.update(cx, |workspace, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_crew_for_test(vec![bo()], bruhs_models::crew::CREW_PROTOCOL, cx);
                panel.fail_sends_for_test("Bo's queue is full.");
            });
        });
        sender.update(cx, |chat, _| chat.is_streaming = true);
        assert_eq!(
            ask(&workspace, &sender, message_bo("check", true), cx),
            ThreadAnswer::Pending
        );
        cx.run_until_parked();
        let replies = take_resolved_for_test();
        assert_eq!(replies.len(), 1, "{replies:?}");
        assert!(!replies[0].1.ok);
        assert_eq!(replies[0].1.text, "Not sent: Bo's queue is full.");
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.bruh_waits.waits.is_empty())
        });
        sender.update(cx, |chat, _| chat.is_streaming = false);
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }

    #[gpui::test]
    fn a_bruh_wait_is_not_held_for_a_turn_that_already_ended(cx: &mut TestAppContext) {
        let (workspace, cx) = fixture(cx);
        let (sender, _, recipient, _) = two_threads(&workspace, cx);
        workspace.update(cx, |workspace, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_crew_for_test(vec![bo()], bruhs_models::crew::CREW_PROTOCOL, cx)
            });
        });
        // The sender's turn is over by the time the send comes back.
        sender.update(cx, |chat, _| chat.is_streaming = false);
        assert_eq!(
            ask(&workspace, &sender, message_bo("check", true), cx),
            ThreadAnswer::Pending
        );
        cx.run_until_parked();
        assert_eq!(sent_commands(&workspace, cx).len(), 1);
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.bruh_waits.waits.is_empty())
        });
        assert!(take_resolved_for_test().is_empty());
        recipient.update(cx, |chat, _| chat.is_streaming = false);
    }
}
