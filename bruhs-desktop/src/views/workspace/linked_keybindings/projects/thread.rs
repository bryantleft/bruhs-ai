//! Open a task's conversation through the grant belonging to its host.
use super::*;
use bruhs_models::{
    ids::ThreadId,
    reactive::{LinkedThreadMessage, LinkedThreadPage, LinkedThreadReply, ScheduledTaskSummary},
};
use bruhs_ui::press::on_press_view;
use std::sync::Arc;

pub(super) struct OpenThread {
    route: AppView,
    host: TaskEditorHost,
    panel: Entity<ThreadPanel>,
}

struct ThreadPanel {
    host: TaskEditorHost,
    task: String,
    thread: ThreadId,
    title: String,
    messages: Vec<LinkedThreadMessage>,
    before: Option<String>,
    input: Entity<TextareaState>,
    pending: Option<LinkedThreadReply>,
    scroll: ScrollHandle,
    reading: bool,
    sending: bool,
    loaded: bool,
    read_error: Option<String>,
    send_error: Option<String>,
    _poll: Task<()>,
    _input_events: gpui::Subscription,
}

impl Workspace {
    pub(super) fn linked_thread_link(
        &self,
        host: &ProjectHost,
        task: &ScheduledTaskSummary,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let thread = task.thread.as_ref().filter(|_| host.data.read_threads)?;
        let title = if thread.title.trim().is_empty() {
            "Open thread"
        } else {
            &thread.title
        };
        let route = AppView::LinkedTaskThread {
            host: host.credential,
            task: task.id.clone(),
            thread: thread.id.clone(),
        };
        let id = SharedString::from(format!("linked-thread-{}-{}", host.credential, task.id));
        Some(
            on_press_view(
                div()
                    .id(id.clone())
                    .debug_selector(move || id.to_string())
                    .role(Role::Link)
                    .max_w(px(256.0))
                    .min_w_0()
                    .truncate()
                    .tab_index(0)
                    .cursor_pointer()
                    .text_sm()
                    .text_color(theme::rambutan_300())
                    .underline()
                    .aria_label(format!("Open {title} on {}", host.name()))
                    .child(title.to_owned()),
                cx,
                move |ws, w, cx| ws.activate_sidebar_view(route.clone(), w, cx),
            )
            .into_any_element(),
        )
    }

    pub(in crate::views::workspace) fn reconcile_linked_thread(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let AppView::LinkedTaskThread { host, task, thread } = &self.active_view else {
            self.linked_projects.thread = None;
            return;
        };
        let Some(link) = self.linked_projects.hosts.iter().find(|h| {
            h.credential == *host
                && h.data.read_threads
                && h.problem.is_none()
                && !h.token.is_empty()
        }) else {
            self.linked_projects.thread = None;
            return;
        };
        if self.linked_projects.thread.as_ref().is_some_and(|open| {
            open.route == self.active_view
                && open.host.0.token == link.token
                && open.host.0.destination() == link.destination()
                && open.host.0.data.reply_threads == link.data.reply_threads
                && open.host.0.tunnel.as_ref().and_then(|t| t.socket.as_ref())
                    == link.tunnel.as_ref().and_then(|t| t.socket.as_ref())
        }) {
            return;
        }
        let host = TaskEditorHost(Arc::new(link.clone()));
        let panel =
            cx.new(|cx| ThreadPanel::new(host.clone(), task.clone(), thread.clone(), window, cx));
        self.linked_projects.thread = Some(OpenThread {
            route: self.active_view.clone(),
            host,
            panel,
        });
    }

    pub(in crate::views::workspace) fn render_linked_thread(&self, cx: &Context<'_, Self>) -> Div {
        let mut surface = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_3();
        let AppView::LinkedTaskThread { host, .. } = self.active_view else {
            return surface;
        };
        surface = surface.child(div().flex().child(on_press_view(
            button("task-thread-back", "Back to Scheduled tasks", false),
            cx,
            move |ws, w, cx| {
                ws.open_settings(&OpenSettings, w, cx);
                ws.linked_projects.scope = Some(ProjectView::OnHost { host, path: None });
                ws.set_settings_section(SettingsSection::ScheduledTasks, cx);
            },
        )));
        if let Some(open) = &self.linked_projects.thread {
            surface.child(open.panel.clone())
        } else {
            let reading = self.linked_projects.reading;
            surface
                .child(if reading {
                    "Connecting to the host…"
                } else {
                    "Conversation access is unavailable on this host."
                })
                .child(div().flex().child(on_press_view(
                    button("task-thread-reconnect", "Reconnect", reading),
                    cx,
                    |ws, _, cx| ws.refresh_linked_projects(true, cx),
                )))
        }
    }
}

impl ThreadPanel {
    fn new(
        host: TaskEditorHost,
        task: String,
        thread: ThreadId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Reply to this conversation"));
        let input_events = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if this.update(cx, |panel, cx| panel.read(None, cx)).is_err() {
                    break;
                }
            }
        });
        let mut panel = Self {
            host,
            task,
            thread,
            title: "Conversation".into(),
            messages: Vec::new(),
            before: None,
            input,
            pending: None,
            scroll: ScrollHandle::new(),
            reading: false,
            sending: false,
            loaded: false,
            read_error: None,
            send_error: None,
            _poll: poll,
            _input_events: input_events,
        };
        panel.read(None, cx);
        panel
    }

    fn endpoint(&self) -> Result<reqwest::Url, String> {
        self.host.url(&[
            "scheduled-tasks",
            &self.task,
            "thread",
            self.thread.as_str(),
        ])
    }

    fn read(&mut self, before: Option<String>, cx: &mut Context<'_, Self>) {
        if self.reading {
            return;
        }
        self.reading = true;
        let host = self.host.clone();
        let url = self.endpoint();
        let cursor = before.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let mut request = host.0.client()?.get(url?);
            if let Some(cursor) = cursor {
                request = request.query(&[("before", cursor)])
            }
            host.reply::<LinkedThreadPage>(request).await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |panel, cx| {
                panel.reading = false;
                match result.and_then(|p| {
                    if p.thread.id == panel.thread {
                        Ok(p)
                    } else {
                        Err("The host returned a different conversation.".into())
                    }
                }) {
                    Ok(mut page) => {
                        if before.is_none()
                            && (!panel.loaded
                                || panel.scroll.max_offset().y + panel.scroll.offset().y <= px(2.0))
                        {
                            panel.scroll.scroll_to_bottom();
                        }
                        if !page.thread.title.trim().is_empty() {
                            panel.title = page.thread.title;
                        }
                        if before.is_some() {
                            page.messages.append(&mut panel.messages);
                            panel.messages = page.messages;
                            panel.before = page.before;
                        } else if let Some(index) = page.messages.first().and_then(|first| {
                            panel.messages.iter().position(|m| m.key == first.key)
                        }) {
                            if index == 0 {
                                panel.before = page.before;
                            }
                            panel.messages.truncate(index);
                            panel.messages.extend(page.messages);
                        } else {
                            panel.messages = page.messages;
                            panel.before = page.before;
                        }
                        panel.loaded = true;
                        panel.read_error = None;
                    }
                    Err(error) => {
                        panel.messages.clear();
                        panel.title = "Conversation".into();
                        panel.before = None;
                        panel.loaded = false;
                        panel.read_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let message = self.input.read(cx).value().trim().to_owned();
        if !self.loaded || !self.host.0.data.reply_threads || self.sending || message.is_empty() {
            return;
        }
        let reply = self
            .pending
            .as_ref()
            .filter(|r| r.message == message)
            .cloned()
            .unwrap_or_else(|| LinkedThreadReply {
                request_id: uuid::Uuid::new_v4(),
                message: message.clone(),
            });
        let id = reply.request_id;
        self.pending = Some(reply.clone());
        self.sending = true;
        self.send_error = None;
        let host = self.host.clone();
        let url = self.endpoint();
        let job = crate::app_state::spawn_tokio(cx, async move {
            host.reply::<uuid::Uuid>(host.0.client()?.post(url?).json(&reply))
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |panel, window, cx| {
                panel.sending = false;
                match result {
                    Ok(receipt) if receipt == id => {
                        panel.pending = None;
                        if panel.input.read(cx).value().trim() == message {
                            panel
                                .input
                                .update(cx, |field, cx| field.set_value("", window, cx));
                        }
                        panel.read(None, cx);
                    }
                    Ok(_) => {
                        panel.send_error = Some("The host could not confirm this reply.".into())
                    }
                    Err(e) => {
                        panel.send_error = Some(e);
                        panel.read(None, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ThreadPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut history = div()
            .id("linked-thread-history")
            .debug_selector(|| "linked-thread-history".into())
            .flex_1()
            .min_h_0()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        if let Some(before) = self.before.clone() {
            history = history.child(div().flex().child(on_press_view(
                button("linked-thread-older", "Load earlier messages", self.reading),
                cx,
                move |panel, _, cx| panel.read(Some(before.clone()), cx),
            )));
        }
        for m in &self.messages {
            history = history.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::rambutan_300())
                            .child(if m.assistant { "Assistant" } else { "You" }),
                    )
                    .child(div().text_sm().child(m.text.clone())),
            );
        }
        if self.messages.is_empty() {
            history = history.child(self.read_error.clone().unwrap_or_else(|| {
                if self.loaded {
                    "No messages yet.".into()
                } else {
                    "Loading conversation…".into()
                }
            }));
        }
        if self.read_error.is_some() {
            history = history.child(div().flex().child(on_press_view(
                button("linked-thread-retry", "Retry", self.reading),
                cx,
                |panel, _, cx| panel.read(None, cx),
            )));
        }
        let mut panel = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_300())
                    .child(self.host.name()),
            )
            .child(div().text_lg().child(self.title.clone()))
            .child(history);
        if let Some(e) = &self.send_error {
            panel = panel.child(
                div()
                    .id("linked-thread-send-error")
                    .role(Role::Alert)
                    .child(e.clone()),
            );
        }
        if self.loaded {
            if self.host.0.data.reply_threads {
                panel = panel
                    .child(
                        Textarea::new(&self.input)
                            .aria_label("Reply to conversation")
                            .h(px(68.)),
                    )
                    .child(div().flex().child(on_press_view(
                        button(
                            "linked-thread-send",
                            if self.sending {
                                "Sending…"
                            } else {
                                "Send reply"
                            },
                            self.sending || self.input.read(cx).value().trim().is_empty(),
                        ),
                        cx,
                        |panel, w, cx| panel.send(w, cx),
                    )));
            } else {
                panel = panel.child(div().text_sm().child("Read-only conversation"));
            }
        }
        panel
    }
}
