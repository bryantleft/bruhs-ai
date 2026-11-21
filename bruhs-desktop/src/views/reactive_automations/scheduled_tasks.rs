//! Scheduled prompts are backed by the same saved sources and subscriptions as Automations.
use super::*;
mod task_rows;
use bruhs_ui::{
    scroll::ScrollableElement,
    zoom::{css_px, px},
};

#[derive(IntoElement)]
pub(crate) struct ScheduledTaskSettings {
    pub panel: Entity<ReactiveAutomations>,
    pub project: String,
    pub initial_project: String,
    pub initial_model: String,
    pub card_width: f32,
}

impl RenderOnce for ScheduledTaskSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.panel.update(cx, |panel, cx| {
            panel.last_scheduled_render = Some(Instant::now());
            if !panel.loaded {
                panel.refresh_snapshot(cx);
            }
            if let Some(id) = &panel.task_dialog.menu_task {
                let available = panel.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .subscriptions
                        .iter()
                        .find(|s| &s.id == id)
                        .and_then(|sub| AutomationSetup::from_subscription(snapshot, sub))
                        .is_some_and(|setup| {
                            scheduled(&setup.source.kind)
                                && (self.project.is_empty()
                                    || setup.agent.project_path == self.project)
                        })
                });
                if !available {
                    panel.task_dialog.menu_task = None;
                }
            }
            if panel.task_dialog.restore_focus && !panel.scheduled_editor_open() {
                panel.task_dialog.restore_focus = false;
                panel.task_dialog.return_focus.focus(window, cx);
            }
            panel.render_scheduled_tasks(
                &self.project,
                self.initial_project,
                self.initial_model,
                self.card_width,
                cx,
            )
        })
    }
}

#[derive(IntoElement)]
pub(crate) struct ScheduledTaskDialog {
    pub panel: Entity<ReactiveAutomations>,
    pub projects: Vec<bruhs_ui::dropdown::DropdownOption>,
    pub project_models: HashMap<String, String>,
    pub default_model: String,
}

pub(super) struct TaskDialogState {
    start: FocusHandle,
    end: FocusHandle,
    return_focus: FocusHandle,
    origin: Option<String>,
    pub(super) scroll: ScrollHandle,
    focus_on_open: bool,
    restore_focus: bool,
    pub(super) history: Option<super::task_deliveries::DeliveryHistory>,
    menu_task: Option<String>,
    menu_items: [FocusHandle; 3],
}

impl TaskDialogState {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            start: cx.focus_handle(),
            end: cx.focus_handle(),
            return_focus: cx.focus_handle().tab_stop(true),
            origin: None,
            scroll: ScrollHandle::new(),
            focus_on_open: false,
            restore_focus: false,
            history: None,
            menu_task: None,
            menu_items: std::array::from_fn(|_| cx.focus_handle()),
        }
    }
}

impl RenderOnce for ScheduledTaskDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.panel.update(cx, |panel, cx| {
            let editing = panel.task_dialog.history.is_none();
            let (title, description, form, footer) =
                if let Some(history) = &panel.task_dialog.history {
                    if panel.task_dialog.focus_on_open {
                        panel.task_dialog.focus_on_open = false;
                        history.initial_focus.focus(window, cx);
                    }
                    panel.render_task_deliveries(history, cx)
                } else {
                    let Some(Editor::Automation(editor)) = &panel.editor else {
                        return div().into_any_element();
                    };
                    let title = editor.task_title();
                    if panel.task_dialog.focus_on_open {
                        panel.task_dialog.focus_on_open = false;
                        editor.first_task_field(cx).focus(window, cx);
                    }
                    let choices = if editor.locked_agent().is_some() {
                        let model=editor.picker.chosen_thread.as_ref().and_then(|c|
                            panel.snapshot.as_ref()?.agents.iter().find(|a|a.id==c.agent_id)).map(|a|a.model.as_str());
                        editor.shared_task_choices(model)
                    } else {
                        panel.render_task_choices(
                            editor,
                            self.projects,
                            self.project_models,
                            self.default_model,
                            cx,
                        )
                    };
                    let form = panel.render_automation_form(editor, Some(choices), cx);
                    let footer = task_footer()
                        .child(
                            ghost_control("scheduled-task-cancel", "Cancel", panel.busy)
                                .on_activate(cx, |this, _, cx| this.dismiss_scheduled_task(cx)),
                        )
                        .child(
                            control_as(
                                "scheduled-task-save",
                                editor.task_save_label(),
                                panel.busy,
                                true,
                            )
                            .on_activate(cx, |this, _, cx| this.review_automation(cx)),
                        );
                    (
                        title.to_string(),
                        "Run a prompt on a schedule or when a webhook arrives.",
                        form.into_any_element(),
                        footer,
                    )
                };
            let viewport = window.viewport_size();
            let max_height = px((css_px(viewport.height) - 80.).clamp(1., 720.));
            let dialog = div()
                .id("scheduled-task-dialog")
                .debug_selector(|| "scheduled-task-dialog".into())
                .role(Role::Dialog)
                .aria_label(title.clone())
                .track_focus(&panel.task_dialog.start)
                .tab_group()
                .on_action(
                    cx.listener(|this, _: &bruhs_ui::actions::CloseOverlays, window, cx| {
                        cx.stop_propagation();
                        this.close_scheduled_overlay(window, cx);
                    }),
                )
                .on_click(|_, _, cx| cx.stop_propagation())
                .flex()
                .flex_col()
                .overflow_hidden()
                .w(px((css_px(viewport.width) - 32.).clamp(1., 640.)))
                .max_h(max_height)
                .when(editing, |el| el.h(max_height))
                .rounded(px(14.))
                .border_1()
                .border_color(theme::ring_modal())
                .bg(theme::longan_950())
                .text_color(theme::rambutan_100())
                .font_family(theme::UI_FONT_FAMILY)
                .child(
                    div()
                        .flex_shrink_0()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(theme::ring_modal())
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .child(
                                    heading(title)
                                        .text_size(px(17.))
                                        .font_weight(FontWeight::SEMIBOLD),
                                )
                                .child(muted(description)),
                        )
                        .child(
                            web_button(
                                "scheduled-task-close",
                                WebButtonVariant::Ghost,
                                WebButtonSize::IconSmall,
                                ButtonState::disabled_if(panel.busy),
                            )
                            .aria_label("Close dialog")
                            .debug_selector(|| "scheduled-task-close".into())
                            .child(
                                svg()
                                    .path(crate::assets::CLOSE_ICON_PATH)
                                    .size(px(theme::ICON_PX))
                                    .text_color(theme::rambutan_400()),
                            )
                            .on_activate(cx, |this, _, cx| this.dismiss_scheduled_task(cx)),
                        ),
                )
                .child(
                    div()
                        .id("scheduled-task-dialog-scroll")
                        .debug_selector(|| "scheduled-task-dialog-scroll".into())
                        .when(editing, |el| el.flex_1())
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&panel.task_dialog.scroll)
                        .child(div().p_4().child(form))
                        .vertical_scrollbar(&panel.task_dialog.scroll),
                )
                .when_some(
                    panel
                        .error
                        .as_ref()
                        .or(panel.load_error.as_ref())
                        .filter(|_| editing),
                    |el, error| {
                        el.child(
                            muted(error.clone())
                                .id("task-save-error")
                                .debug_selector(|| "task-save-error".into())
                                .role(Role::Alert)
                                .aria_label(error.clone())
                                .px_4()
                                .py_2()
                                .text_color(theme::lychee_300()),
                        )
                    },
                )
                .child(footer)
                .child(div().size(px(0.)).track_focus(&panel.task_dialog.end));
            div()
                .id("scheduled-task-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::longan_950().opacity(0.68))
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.dismiss_scheduled_task(cx);
                }))
                .child(dialog)
                .into_any_element()
        })
    }
}

pub(super) fn task_footer() -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .justify_end()
        .gap_2()
        .px_4()
        .py_3()
        .border_t_1()
        .border_color(theme::ring_modal())
}

fn scheduled(kind: &SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::Once { .. }
            | SourceKind::Schedule { .. }
            | SourceKind::Calendar { .. }
            | SourceKind::Webhook
    )
}

fn shared_agent(snapshot: &ReactiveSnapshot, task: &bruhs_models::reactive::Subscription) -> bool {
    snapshot
        .subscriptions
        .iter()
        .any(|other| other.agent_id == task.agent_id && other.id != task.id)
}

fn schedule_summary(kind: &SourceKind) -> String {
    let SourceKind::Schedule { every_ms } = kind else {
        return setup::source_summary(kind, None);
    };
    for (unit, label) in [(86_400_000, "day"), (3_600_000, "hour"), (60_000, "minute")] {
        if every_ms.is_multiple_of(unit) {
            let count = every_ms / unit;
            return if count == 1 {
                format!("Every {label}")
            } else {
                format!("Every {count} {label}s")
            };
        }
    }
    setup::source_summary(kind, None)
}

fn relative_next_run(due: bruhs_models::EpochMs, now: bruhs_models::EpochMs) -> String {
    let remaining = due.as_millis() - now.as_millis();
    if remaining <= 0 {
        return bruhs_models::time::format_relative_ms(
            due,
            now,
            bruhs_models::time::RelativeTimeStyle::Ago,
        );
    }
    let minutes = (remaining as u64).div_ceil(60_000);
    match minutes {
        1 => "in under a minute".into(),
        2..60 => format!("in {minutes}m"),
        _ => {
            let hours = (minutes + 30) / 60;
            if hours < 24 {
                format!("in {hours}h")
            } else {
                format!("in {}d", (hours + 12) / 24)
            }
        }
    }
}

fn schedule_timing(setup: &AutomationSetup, snapshot: &ReactiveSnapshot) -> Stateful<Div> {
    let deadlines = snapshot.next_run_by_source.as_ref();
    let due = deadlines
        .and_then(|times| times.get(&setup.source.id))
        .copied();
    let (label, exact) = if !setup.enabled() {
        ("Paused".into(), None)
    } else if setup.source.kind == SourceKind::Webhook {
        ("Listening".into(), None)
    } else if let Some(due) = due {
        (
            format!(
                "Next run {}",
                relative_next_run(due, bruhs_models::EpochMs::now())
            ),
            Some(local_time(due)),
        )
    } else if deadlines.is_some() {
        ("Not scheduled".into(), None)
    } else {
        // Older hosts do not report deadlines.
        ("Enabled".into(), None)
    };
    let summary = format!("{} · {label}", schedule_summary(&setup.source.kind));
    muted(summary.clone())
        .id(SharedString::from(format!(
            "task-timing-{}",
            setup.subscription.id
        )))
        .role(Role::Label)
        .aria_label(summary)
        .when_some(exact, |line, time| {
            line.aria_description(time.clone())
                .tooltip(move |window, cx| {
                    bruhs_ui::tooltip::Tooltip::new(time.clone()).build(window, cx)
                })
        })
}

impl ReactiveAutomations {
    pub(crate) fn tasks_for_thread(&self, thread: &str) -> Vec<(String, String)> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let ids: std::collections::BTreeSet<_> = snapshot
            .runs
            .iter()
            .filter(|run| run.thread_id.as_str() == thread)
            .filter_map(|run| run.subscription_id.as_deref())
            .collect();
        snapshot
            .subscriptions
            .iter()
            .filter(|sub| {
                ids.contains(sub.id.as_str())
                    || snapshot
                        .task_threads
                        .get(&sub.id)
                        .is_some_and(|id| id.as_str() == thread)
            })
            .filter_map(|sub| AutomationSetup::from_subscription(snapshot, sub))
            .filter(|setup| scheduled(&setup.source.kind))
            .map(|setup| (setup.subscription.id.clone(), setup.title().to_owned()))
            .collect()
    }

    pub(crate) fn open_task_by_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let task = self.snapshot.as_ref().and_then(|snapshot| {
            let sub = snapshot.subscriptions.iter().find(|sub| sub.id == id)?;
            AutomationSetup::from_subscription(snapshot, sub)
        });
        if let Some(task) = task { self.edit_scheduled_task(Some(task), window, cx); }
    }

    pub(crate) fn automation_project_paths(&self) -> impl Iterator<Item = &str> {
        self.snapshot
            .iter()
            .flat_map(|snapshot| snapshot.agents.iter())
            .map(|agent| agent.project_path.as_str())
            .filter(|path| !path.is_empty())
    }

    pub(crate) fn scheduled_focus_bounds(&self) -> (FocusHandle, FocusHandle) {
        (self.task_dialog.start.clone(), self.task_dialog.end.clone())
    }

    pub(crate) fn scheduled_editor_open(&self) -> bool {
        self.task_dialog.history.is_some()
            || matches!(&self.editor, Some(Editor::Automation(editor)) if editor.scheduled_form)
    }

    pub(crate) fn close_scheduled_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.escape_schedule_picker(window, cx) {
            self.dismiss_scheduled_task(cx);
        }
    }

    pub(crate) fn dismiss_scheduled_task(&mut self, cx: &mut Context<'_, Self>) {
        if self.task_dialog.history.take().is_some() {
            cx.notify();
        } else if self.scheduled_editor_open() && !self.busy {
            self.editor = None;
            self.error = None;
            cx.notify();
        }
    }

    pub(super) fn edit_scheduled_task(
        &mut self,
        setup: Option<AutomationSetup>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.snapshot.is_none() {
            return;
        }
        let origin = setup.as_ref().map(|s| s.subscription.id.clone());
        let chosen = origin
            .as_ref()
            .and_then(|id| {
                self.snapshot
                    .as_ref()?
                    .selected_task_threads
                    .as_ref()?
                    .get(id)
            })
            .cloned();
        let original = setup
            .as_ref()
            .filter(|_| {
                self.snapshot
                    .as_ref()
                    .is_some_and(|s| s.selected_task_threads.is_some())
            })
            .and_then(|s| {
                Some(TaskThreadChoice {
                    thread_id: self
                        .snapshot
                        .as_ref()?
                        .task_threads
                        .get(&s.subscription.id)?
                        .clone(),
                    title: s.title().to_owned(),
                    agent_id: s.agent.id.clone(),
                    checkout: s.subscription.task_checkout.clone(),
                })
            });
        let webhook_task = setup.as_ref()
            .filter(|setup| setup.source.kind == SourceKind::Webhook)
            .filter(|_| self.snapshot.as_ref().is_some_and(|s| s.can_address_task_webhooks))
            .map(|setup| setup.subscription.id.clone());
        let new = setup.is_none();
        if let Some(setup) = setup {
            if !scheduled(&setup.source.kind) {
                return;
            }
            if chosen.is_some()
                || self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| shared_agent(snapshot, &setup.subscription))
            {
                let agent = setup.agent.id.clone();
                self.open_routine(Some(setup), &agent, window, cx);
            } else {
                self.open_automation(Some(setup), window, cx);
            }
        } else {
            self.open_automation(None, window, cx);
        }
        if let Some(Editor::Automation(editor)) = &mut self.editor {
            self.task_dialog.menu_task = None;
            self.task_dialog.origin = origin;
            editor.webhook_url = webhook_task.map(|task| {
                cx.new(|cx| task_webhook_url::TaskWebhookUrl::new(self.client.clone(), task, window, cx))
            });
            self.task_dialog.scroll = ScrollHandle::new();
            self.task_dialog.restore_focus = true;
            self.task_dialog.focus_on_open = true;
            editor.picker.original_thread = chosen.clone().or(original);
            editor.picker.chosen_thread = chosen;
            editor.prepare_task_schedule(window, cx);
            if new {
                editor.subscription_mut().subscription.task_conversation =
                    TaskConversation::NewThread;
                editor.checkout.select("worktree");
            }
        }
        cx.notify();
    }

    pub(super) fn render_task_conversation(
        &self,
        editor: &setup::AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        use bruhs_ui::dropdown::{dropdown, DropdownGroup, DropdownOption, Listbox};
        let toggle = cx.weak_entity();
        let select = toggle.clone();
        let options = TaskConversation::ALL
            .into_iter()
            .map(|mode| DropdownOption {
                value: mode.key(),
                label: mode.label().into(),
                description: None,
            })
            .collect();
        let picker = dropdown(
            "schedule-conversation",
            "Task thread",
            editor.task_conversation().key(),
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if editor.task_thread_open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = toggle.update(cx, |this, cx| {
                    if let Some(Editor::Automation(e)) = &mut this.editor {
                        e.task_thread_open = !e.task_thread_open;
                        e.picker.open = None;
                        e.open_filter = None;
                        e.security.encoding_open = false;
                        e.checkout.close();
                    }
                    cx.notify();
                });
            }),
            Rc::new(move |value, _, cx| {
                let Some(mode) = TaskConversation::ALL
                    .into_iter()
                    .find(|mode| mode.key() == value)
                else {
                    return;
                };
                let _ = select.update(cx, |this, cx| {
                    if let Some(Editor::Automation(e)) = &mut this.editor {
                        e.subscription_mut().subscription.task_conversation = mode;
                        e.task_thread_open = false;
                        if !mode.has_destination() {
                            e.picker.clear_thread();
                        }
                    }
                    cx.notify();
                });
            }),
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(muted("Destination"))
            .child(picker)
            .when(
                editor.task_conversation().has_destination()
                    && self
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| s.selected_task_threads.is_some()),
                |el| el.child(self.task_thread_picker(editor, cx)),
            )
            .child(muted(match editor.task_conversation() {
                TaskConversation::NewThread => {
                    "Each run starts a new thread with the selected workspace and model."
                }
                TaskConversation::Continue if editor.picker.displayed_thread().is_some() => {
                    "Each run continues this conversation in its workspace with its current model."
                }
                TaskConversation::Continue => {
                    "The first run starts a thread. Later runs continue its conversation."
                }
                TaskConversation::FreshSession => {
                    "Runs post to the same thread. Each starts a fresh provider session."
                }
                TaskConversation::Agent => "Runs use this agent’s own conversation.",
            }))
            .when(editor.task_conversation().has_destination() && self.snapshot.as_ref().is_some_and(|s|s.can_delete_task_threads),
                |el|el.child(muted("Deleting the thread also deletes this task.")))
    }

    fn confirm_delete_scheduled_task(
        &self,
        id: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Delete the schedule for “{name}”?"),
            Some("This removes the schedule. The agent and its past runs stay available."),
            &[
                gpui::PromptButton::Cancel("Cancel".into()),
                gpui::PromptButton::Ok("Delete".into()),
            ],
            cx,
        );
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| {
                    this.send_all(
                        vec![apply(ReactiveChange::DeleteSubscription { id })],
                        Scope::General,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn render_scheduled_tasks(
        &self,
        project: &str,
        initial_project: String,
        initial_model: String,
        card_width: f32,
        cx: &Context<'_, Self>,
    ) -> Div {
        // T3's task rows stack controls below 32rem of usable row width.
        let stacked = card_width - 2.0 * 12.0 - 2.0 < theme::SETTINGS_INLINE_CONTENT_WIDTH_PX;
        let mut body = div().flex().flex_col().gap_3().p_3();
        if let Some(error) = self.error.as_ref().or(self.load_error.as_ref()) {
            body = body.child(
                muted(error.clone())
                    .id("scheduled-task-error")
                    .role(Role::Alert)
                    .text_color(theme::lychee_300()),
            );
        }
        let Some(snapshot) = &self.snapshot else {
            return body
                .child(muted(if self.reading {
                    "Loading scheduled tasks…"
                } else {
                    "Connect the local automation server to view scheduled tasks."
                }))
                .child(
                    ghost_control(
                        "scheduled-tasks-retry",
                        "Refresh",
                        self.reading || self.busy,
                    )
                    .on_activate(cx, |this, _, cx| this.refresh_snapshot(cx)),
                );
        };
        let tasks: Vec<_> = snapshot
            .subscriptions
            .iter()
            .filter_map(|sub| {
                let setup = AutomationSetup::from_subscription(snapshot, sub)?;
                (scheduled(&setup.source.kind)
                    && (project.is_empty() || setup.agent.project_path == project))
                    .then_some((sub, setup))
            })
            .collect();
        body = body.child(
            div().flex().justify_end().child(
                control("settings-new-scheduled-task", "New task", self.busy)
                    .when(
                        self.task_dialog
                            .origin
                            .as_ref()
                            .is_none_or(|id| !tasks.iter().any(|(sub, _)| &sub.id == id)),
                        |b| b.track_focus(&self.task_dialog.return_focus),
                    )
                    .on_activate(cx, move |this, window, cx| {
                        this.edit_scheduled_task(None, window, cx);
                        if let Some(Editor::Automation(editor)) = &this.editor {
                            editor.set_project_path(&initial_project, window, cx);
                            editor.set_task_model(&initial_model, window, cx);
                        }
                    }),
            ),
        );
        if tasks.is_empty() {
            let message = if project.is_empty() {
                "No scheduled tasks"
            } else {
                "No scheduled tasks in this project"
            };
            return body.child(muted(message).debug_selector(|| "scheduled-tasks-empty".into()));
        }
        for (sub, setup) in tasks {
            let title_id = format!("scheduled-task-name-{}", sub.id);
            let mut info = div()
                .min_w_0()
                .when(stacked, |info| info.w_full())
                .when(!stacked, |info| info.flex_1())
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    heading(setup.title().to_owned())
                        .id(SharedString::from(title_id.clone()))
                        .debug_selector(move || title_id.clone())
                        .role(Role::Heading)
                        .aria_level(3)
                        .aria_label(setup.title().to_owned()),
                )
                .child(
                    muted(if let Some(prompt) = &sub.webhook_prompt {
                        prompt.clone()
                    } else if shared_agent(snapshot, sub) || snapshot.selected_task_threads.as_ref().is_some_and(|all|all.contains_key(&sub.id)) {
                        sub.instructions.clone()
                    } else {
                        setup.agent.instructions.clone()
                    })
                    .line_clamp(2),
                )
                .child(schedule_timing(&setup, snapshot));
            if let Some(run) = snapshot
                .runs
                .iter()
                .find(|run| run.subscription_id.as_ref() == Some(&sub.id))
            {
                info = info.child(muted(format!("Last run · {}", run_label(run.status))));
            }
            if let Some(thread_id) = snapshot.task_threads.get(&sub.id) {
                let selected = snapshot
                    .selected_task_threads
                    .as_ref()
                    .and_then(|all| all.get(&sub.id));
                let title = selected.map_or_else(|| setup.title().to_owned(), |t| t.title.clone());
                let label = selected
                    .map_or("Open thread", |t| t.title.as_str())
                    .to_owned();
                let location = TaskThreadLocation {
                    task_id: sub.id.clone(),
                    agent_id: sub.agent_id.clone(),
                    thread_id: thread_id.clone(),
                    title,
                };
                let selector = format!("task-open-thread-{}", sub.id);
                info = info.child(bruhs_ui::press::on_press_view(
                    div()
                        .id(SharedString::from(selector.clone()))
                        .debug_selector(move || selector.clone())
                        .role(Role::Link)
                        .tab_index(0)
                        .aria_label(format!("Open thread {}", location.title))
                        .cursor_pointer()
                        .underline()
                        .text_size(px(12.))
                        .text_color(theme::rambutan_300())
                        .hover(|e| e.text_color(theme::rambutan_100()))
                        .focus_visible(|e| e.text_color(theme::persimmon_300()))
                        .child(label),
                    cx,
                    move |_, _, cx| cx.emit(location.clone()),
                ));
            }
            if setup.source.kind == SourceKind::Webhook {
                if let Some(delivery) = snapshot
                    .events
                    .iter()
                    .filter(|event| event.source_id == sub.source_id)
                    .max_by_key(|event| event.created_at)
                {
                    if delivery.filtered_subscriptions.contains(&sub.id) {
                        info = info.child(muted(format!(
                            "Last delivery filtered · {}",
                            local_time(delivery.created_at)
                        )));
                    }
                }
            }
            let row = div()
                .id(SharedString::from(format!("scheduled-task-{}", sub.id)))
                .role(Role::Group)
                .aria_label(format!("Scheduled task {}", setup.title()))
                .flex()
                .when(stacked, |row| row.flex_col().items_start().gap_3())
                .when(!stacked, |row| row.items_center().justify_between().gap_8())
                .min_w_0()
                .py_3()
                .border_b_1()
                .border_color(theme::ring_modal())
                .child(info)
                .child(self.scheduled_row_actions(setup, cx));
            body = body.child(row);
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::relative_next_run;
    use ::core::prelude::v1::test;

    #[test]
    fn next_run_rounds_up_to_the_due_minute_then_to_hours_and_days() {
        use bruhs_models::EpochMs;
        let now = EpochMs::new(86_400_000).unwrap();
        for (offset, expected) in [
            (-60_000, "1m ago"),
            (0, "just now"),
            (1, "in under a minute"),
            (60_000, "in under a minute"),
            (60_001, "in 2m"),
            (59 * 60_000, "in 59m"),
            (60 * 60_000, "in 1h"),
            (90 * 60_000, "in 2h"),
            (24 * 3_600_000, "in 1d"),
        ] {
            let due = EpochMs::new(now.as_millis() + offset).unwrap();
            assert_eq!(relative_next_run(due, now), expected);
        }
    }
}
