//! Native capability grants, transactional workflows, durable jobs, and audit.

use super::*;
mod chat;
mod history_ui;
mod people;
#[cfg(test)]
mod review_tests;
mod rollback;
use bruhs_models::{
    agent_tool_definitions, AgentAccess, AgentDomain, BackgroundJobStatus, FinanceTransaction,
    WorkflowOperation, WorkflowStatus, WorkoutLog,
};

/// A record a workflow step creates, before it is stored.
enum AutomationRecord {
    WorkItem(DesktopWorkItem),
    CalendarEvent(DesktopCalendarEvent),
    KnowledgeNote(KnowledgeDocument),
    MeetingNote(bruhs_models::MeetingNote),
    InboxDraft(InboxConversation),
    Capture(CaptureRecord),
    FinanceTransaction(FinanceTransaction),
    Workout(WorkoutLog),
}

impl AutomationRecord {
    /// The record's id and the snapshot a rollback compares against.
    fn checkpoint(&self) -> Result<(String, String), serde_json::Error> {
        Ok(match self {
            Self::WorkItem(item) => (item.id.clone(), serde_json::to_string(item)?),
            Self::CalendarEvent(event) => (event.id.clone(), serde_json::to_string(event)?),
            Self::KnowledgeNote(document) => {
                (document.id.clone(), serde_json::to_string(document)?)
            }
            Self::MeetingNote(note) => (note.id.clone(), serde_json::to_string(note)?),
            Self::InboxDraft(conversation) => (
                conversation.id.clone(),
                serde_json::to_string(conversation)?,
            ),
            Self::Capture(capture) => (capture.id.clone(), serde_json::to_string(capture)?),
            Self::FinanceTransaction(transaction) => {
                (transaction.id.clone(), serde_json::to_string(transaction)?)
            }
            Self::Workout(workout) => (workout.id.clone(), serde_json::to_string(workout)?),
        })
    }
}

fn focus_operations() -> Vec<WorkflowOperation> {
    bruhs_models::focus_workflow_operations(EpochMs::now().as_millis())
}

impl Workspace {
    pub(super) fn drive_background_automations(&mut self, cx: &mut Context<'_, Self>) {
        if !self.save_status.is_saved()
            || self.automation_error.is_some()
            || self.automation_pending
        {
            return;
        }
        let next = self
            .automation_state
            .jobs
            .iter()
            .find(|job| job.auto_run && job.status == BackgroundJobStatus::Running)
            .map(|job| job.id.clone());
        if let Some(id) = next {
            self.run_automation_step(&id, cx);
        }
    }

    fn enable_background_automation(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if !self.require_saved_workflow_state(cx) {
            return;
        }
        if self.automation_state.enable_background(id) {
            self.automation_error = None;
            self.persist_and_notify(cx);
        } else {
            self.automation_error = Some(
                "Recheck the payload and Write grants before enabling background execution.".into(),
            );
            cx.notify();
        }
    }

    fn require_saved_workflow_state(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.save_status.is_saved() && !self.automation_pending {
            return true;
        }
        // The global save status explains this transient block. Do not turn a
        // double activation during Saving into a permanent executor error.
        cx.notify();
        false
    }

    pub(super) fn render_workflow_status(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let active = self.automation_state.jobs.iter().any(|job| {
            job.auto_run
                && matches!(
                    job.status,
                    BackgroundJobStatus::Running | BackgroundJobStatus::Paused
                )
        });
        let pending_mcp = self.mcp_bridge.pending_review(&self.automation_state);
        if !active && !pending_mcp && self.automation_error.is_none() {
            return None;
        }
        // This app-wide surface must not expose workflow payloads or domain data.
        let message = if self.automation_error.is_some() {
            "Workflow needs attention. Open workflows to review before continuing."
        } else if pending_mcp {
            "An MCP proposal is awaiting your review. Open workflows to inspect the full payload."
        } else if !self.save_status.is_saved() {
            "Background workflow is waiting for a durable local save."
        } else if self
            .automation_state
            .jobs
            .iter()
            .any(|job| job.auto_run && job.status == BackgroundJobStatus::Running)
        {
            "Background workflow is running while this app is open."
        } else {
            "Background workflow is paused."
        };
        Some(
            div()
                .id("workspace-workflow-status")
                .debug_selector(|| "desktop-workflow-status".into())
                .role(if self.automation_error.is_some() {
                    Role::Alert
                } else {
                    Role::Status
                })
                .aria_label(message)
                .w_full()
                .flex_shrink_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(theme::longan_900())
                .text_color(theme::rambutan_400())
                .text_size(px(12.0))
                .child(div().flex_1().min_w_0().child(message))
                .when(
                    self.automation_state
                        .jobs
                        .iter()
                        .any(|job| job.auto_run && job.status == BackgroundJobStatus::Running),
                    |el| {
                        el.child(
                            web_button(
                                "pause-background-workflows",
                                WebButtonVariant::Outline,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label("Pause background workflows")
                            .debug_selector(|| "desktop-pause-background-workflows".into())
                            .child("Pause")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let ids: Vec<_> = this
                                    .automation_state
                                    .jobs
                                    .iter()
                                    .filter(|job| {
                                        job.auto_run && job.status == BackgroundJobStatus::Running
                                    })
                                    .map(|job| job.id.clone())
                                    .collect();
                                for id in ids {
                                    // Only running jobs were collected, so
                                    // a refusal means it finished meanwhile.
                                    if !this
                                        .automation_state
                                        .pause_job(&id, EpochMs::now().as_millis())
                                    {
                                        tracing::debug!("job {id} was no longer running");
                                    }
                                }
                                this.persist_and_notify(cx);
                            })),
                        )
                    },
                )
                .child(
                    web_button(
                        "open-workflow-status",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .aria_label("Open workflows")
                    .debug_selector(|| "desktop-open-workflows".into())
                    .child("Open workflows")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reviewed_workflow_id = this
                            .mcp_bridge
                            .pending_review_id(&this.automation_state)
                            .map(str::to_owned)
                            .or_else(|| {
                                this.automation_state
                                    .jobs
                                    .iter()
                                    .find(|job| {
                                        job.auto_run
                                            && matches!(
                                                job.status,
                                                BackgroundJobStatus::Running
                                                    | BackgroundJobStatus::Paused
                                            )
                                    })
                                    .map(|job| job.workflow_id.clone())
                            })
                            .or_else(|| this.reviewed_workflow_id.clone());
                        this.active_view = AppView::Automations;
                        cx.notify();
                    })),
                )
                .into_any_element(),
        )
    }

    fn preview_focus_automation(&mut self, cx: &mut Context<'_, Self>) {
        let now = EpochMs::now().as_millis();
        // The suggestion is surfaced at most once per cooldown; a refusal
        // only means it was already counted.
        if !self
            .automation_state
            .mark_suggestion_surfaced("protect-focus", now)
        {
            tracing::debug!("focus suggestion was already surfaced");
        }
        let id = self
            .automation_state
            .create_workflow("Protect focus", focus_operations(), now);
        if self.automation_state.preview(&id, now).is_none() {
            self.automation_error =
                Some("The focus workflow could not be previewed. No records were changed.".into());
        }
        self.reviewed_workflow_id = Some(id);
        self.persist_and_notify(cx);
    }

    fn dismiss_focus_suggestion(&mut self, cx: &mut Context<'_, Self>) {
        if !self
            .automation_state
            .dismiss_suggestion("protect-focus", EpochMs::now().as_millis())
        {
            tracing::debug!("focus suggestion was already gone");
        }
        self.persist_and_notify(cx);
    }

    fn start_reviewed_automation(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if !self.require_saved_workflow_state(cx) {
            return;
        }
        self.automation_error = None;
        let now = EpochMs::now().as_millis();
        if self.automation_state.start_job(id, now).is_none() {
            self.automation_error = Some(
                "Recheck the preview and Write grants before starting. No records were changed."
                    .into(),
            );
        }
        self.persist_and_notify(cx);
    }

    #[cfg(test)]
    fn start_latest_automation(&mut self, cx: &mut Context<'_, Self>) {
        let Some(id) = self
            .automation_state
            .workflows
            .iter()
            .rev()
            .find(|workflow| workflow.status == WorkflowStatus::Previewed)
            .map(|workflow| workflow.id.clone())
        else {
            return;
        };
        self.start_reviewed_automation(&id, cx);
    }

    #[cfg(test)]
    pub(in crate::views::workspace) fn run_latest_automation_step(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let next = self
            .automation_state
            .jobs
            .iter()
            .rev()
            .find(|job| job.status == BackgroundJobStatus::Running)
            .map(|job| job.id.clone());
        if let Some(id) = next {
            self.run_automation_step(&id, cx);
        }
    }

    fn resume_automation(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if !self.require_saved_workflow_state(cx) {
            return;
        }
        if self
            .automation_state
            .resume_job(id, EpochMs::now().as_millis())
        {
            self.automation_error = None;
            self.persist_and_notify(cx);
        } else {
            self.automation_error = Some(
                "Recheck the reviewed payload and remaining Write grants before resuming.".into(),
            );
            cx.notify();
        }
    }

    fn run_automation_step(&mut self, job_id: &str, cx: &mut Context<'_, Self>) {
        if !self.require_saved_workflow_state(cx) {
            return;
        }
        self.automation_error = None;
        let Some(job) = self
            .automation_state
            .jobs
            .iter()
            .find(|job| job.id == job_id && job.status == BackgroundJobStatus::Running)
            .cloned()
        else {
            return;
        };
        let operation = self.automation_state.pending_operation(&job.id).cloned();
        let Some(operation) = operation else {
            self.automation_error = Some("This job is paused, finished, changed since preview, or missing a required Write grant. No records were changed.".into());
            cx.notify();
            return;
        };
        let now = EpochMs::now().as_millis();
        let record = match operation {
            WorkflowOperation::CreateChatThread { .. } => {
                self.schedule_chat_workflow_step(job.id, cx);
                return;
            }
            WorkflowOperation::CreatePersonReminder { .. } => {
                self.run_person_reminder_step(&job.id, &operation, now, cx);
                return;
            }
            WorkflowOperation::CreateTask { title } => {
                AutomationRecord::WorkItem(DesktopWorkItem {
                    id: uuid::Uuid::new_v4().to_string(),
                    project_id: None,
                    title,
                    description: Some("Created by an approved bruhs workflow.".into()),
                    status: DesktopWorkItemStatus::Todo,
                    priority: DesktopPriority::Medium,
                    due_date: None,
                    planning: WorkItemPlanning::default(),
                    updated_at: EpochMs::now(),
                })
            }
            WorkflowOperation::CreateCalendarEvent {
                title,
                start_at_ms,
                end_at_ms,
            } => {
                let start_at = chrono::DateTime::from_timestamp_millis(
                    EpochMs::saturating_from_f64(start_at_ms).as_millis(),
                )
                .map_or_else(
                    || Local::now().naive_local(),
                    |date| date.with_timezone(&Local).naive_local(),
                );
                let end_at = chrono::DateTime::from_timestamp_millis(
                    EpochMs::saturating_from_f64(end_at_ms).as_millis(),
                )
                .map_or_else(
                    || start_at + Duration::minutes(45),
                    |date| date.with_timezone(&Local).naive_local(),
                );
                AutomationRecord::CalendarEvent(DesktopCalendarEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    title,
                    start_at,
                    end_at,
                    all_day: false,
                    color: Some("persimmon".into()),
                    notes: Some("Created by an approved bruhs workflow.".into()),
                })
            }
            WorkflowOperation::CreateKnowledgeNote { title, body } => {
                AutomationRecord::KnowledgeNote(KnowledgeDocument::new(title, body, now))
            }
            WorkflowOperation::CreateMeetingNote { title, body } => {
                match bruhs_models::MeetingNote::new(title, body, now) {
                    Ok(note) => AutomationRecord::MeetingNote(note),
                    Err(error) => {
                        self.automation_error = Some(error.to_string());
                        cx.notify();
                        return;
                    }
                }
            }
            WorkflowOperation::DraftInboxMessage { recipient, body } => {
                AutomationRecord::InboxDraft(InboxConversation::local_draft(&recipient, &body, now))
            }
            WorkflowOperation::AddCapture { content } => {
                match CaptureRecord::new(CaptureKind::Note, &content, now) {
                    Ok(capture) => AutomationRecord::Capture(capture),
                    Err(error) => {
                        self.automation_error = Some(format!(
                            "The capture could not be created: {error}. No records were changed."
                        ));
                        cx.notify();
                        return;
                    }
                }
            }
            WorkflowOperation::AddFinanceTransaction {
                merchant,
                category,
                amount_minor,
            } => AutomationRecord::FinanceTransaction(FinanceTransaction::manual(
                merchant,
                category,
                amount_minor,
                now,
            )),
            WorkflowOperation::LogWorkout { activity, minutes } => {
                AutomationRecord::Workout(WorkoutLog::manual(activity, minutes, now))
            }
        };
        if let Err(message) = self.commit_automation_record(&job.id, record, now) {
            self.automation_error = Some(message.into());
            cx.notify();
            return;
        }
        self.persist_and_notify(cx);
    }

    /// Checkpoint the step, then store its record, both or neither: the
    /// record is only added once the job has accepted the checkpoint (with
    /// the record's rollback snapshot).
    fn commit_automation_record(
        &mut self,
        job_id: &str,
        record: AutomationRecord,
        now: i64,
    ) -> Result<(), &'static str> {
        let (target_id, snapshot) = record
            .checkpoint()
            .map_err(|_| "The workflow record could not be encoded. No records were changed.")?;
        if !self
            .automation_state
            .complete_step_with_snapshot(job_id, target_id, snapshot, now)
        {
            return Err("The reviewed workflow changed. No records were changed.");
        }
        match record {
            AutomationRecord::WorkItem(item) => self.work_items.insert(0, item),
            AutomationRecord::CalendarEvent(event) => self.calendar_events.push(event),
            AutomationRecord::KnowledgeNote(document) => {
                self.knowledge_documents.insert(0, document)
            }
            AutomationRecord::MeetingNote(note) => self.meeting_notes.insert(0, note),
            AutomationRecord::InboxDraft(conversation) => {
                self.inbox_conversations.insert(0, conversation)
            }
            AutomationRecord::Capture(capture) => self.captures.insert(0, capture),
            AutomationRecord::FinanceTransaction(transaction) => self
                .life_context
                .finance
                .transactions
                .insert(0, transaction),
            AutomationRecord::Workout(workout) => {
                self.life_context.health.workouts.insert(0, workout)
            }
        }
        Ok(())
    }

    fn rollback_reviewed_automation(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if !self.require_saved_workflow_state(cx) {
            return;
        }
        self.automation_error = None;
        let plan = match self.automation_rollback_plan(id, cx) {
            Ok(plan) => plan,
            Err(message) => {
                self.automation_error = Some(message.into());
                cx.notify();
                return;
            }
        };
        for (domain, id) in plan.targets {
            match domain {
                AgentDomain::Chat => {
                    self.threads.retain(|thread| thread.id != id);
                    self.unread_thread_ids.remove(&id);
                }
                AgentDomain::Tasks => self.work_items.retain(|item| item.id != id),
                AgentDomain::Calendar => self.calendar_events.retain(|event| event.id != id),
                AgentDomain::Capture => self.captures.retain(|capture| capture.id != id),
                AgentDomain::Knowledge => self.knowledge_documents.retain(|item| item.id != id),
                AgentDomain::Inbox => self.inbox_conversations.retain(|item| item.id != id),
                AgentDomain::Meetings => self.meeting_notes.retain(|note| note.id != id),
                AgentDomain::Finance => self
                    .life_context
                    .finance
                    .transactions
                    .retain(|item| item.id != id),
                AgentDomain::Health => self
                    .life_context
                    .health
                    .workouts
                    .retain(|item| item.id != id),
                _ => {}
            }
        }
        plan.people.apply(&mut self.relationships);
        if self
            .automation_state
            .mark_rolled_back(id, EpochMs::now().as_millis())
        {
            self.mcp_bridge.finish_rollback_review(id);
        }
        self.persist_and_notify(cx);
    }

    fn automation_panel(&self, title: &'static str, body: impl IntoElement) -> Div {
        div()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child(title),
            )
            .child(body)
    }

    pub(super) fn render_automations_page(&self, _window: &Window, cx: &Context<'_, Self>) -> Div {
        let grants = AgentDomain::ALL
            .into_iter()
            .enumerate()
            .map(|(index, domain)| {
                let read_on = self
                    .automation_state
                    .permissions
                    .allows(domain, AgentAccess::Read);
                let write_on = self
                    .automation_state
                    .permissions
                    .allows(domain, AgentAccess::Write);
                div()
                    .id(("automation-grant", index))
                    .mt_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(domain.label()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                web_button(
                                    ("automation-read", index),
                                    if read_on {
                                        WebButtonVariant::Outline
                                    } else {
                                        WebButtonVariant::Ghost
                                    },
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label(format!("Toggle {} Read access", domain.label()))
                                .child("Read")
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        let current = this
                                            .automation_state
                                            .permissions
                                            .allows(domain, AgentAccess::Read);
                                        this.automation_state.permissions.set(
                                            domain,
                                            AgentAccess::Read,
                                            !current,
                                        );
                                        this.persist_and_notify(cx);
                                    },
                                )),
                            )
                            .child(
                                web_button(
                                    ("automation-write", index),
                                    if write_on {
                                        WebButtonVariant::Outline
                                    } else {
                                        WebButtonVariant::Ghost
                                    },
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label(format!("Toggle {} Write access", domain.label()))
                                .child("Write")
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        let current = this
                                            .automation_state
                                            .permissions
                                            .allows(domain, AgentAccess::Write);
                                        this.automation_state.permissions.set(
                                            domain,
                                            AgentAccess::Write,
                                            !current,
                                        );
                                        this.persist_and_notify(cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let latest = self.reviewed_automation();
        let workflow_rows = latest
            .into_iter()
            .flat_map(|workflow| workflow.steps.iter())
            .enumerate()
            .map(|(index, step)| {
                let fields = step.operation.review_fields(|millis| {
                    if !millis.is_finite() || millis.fract() != 0.0 {
                        return format!("Invalid timestamp ({millis})");
                    }
                    chrono::DateTime::from_timestamp_millis(
                        EpochMs::saturating_from_f64(millis).as_millis(),
                    )
                    .map_or_else(
                        || format!("Invalid timestamp ({millis})"),
                        |date| {
                            format!(
                                "{} ({millis} ms since Unix epoch)",
                                date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                            )
                        },
                    )
                });
                div()
                    .id(("automation-step", index))
                    .mt_2()
                    .rounded(px(4.0))
                    .bg(theme::longan_950().opacity(0.45))
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w_0()
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_300())
                    .child(format!(
                        "{} · {} · {:?}{}",
                        index + 1,
                        step.operation.domain().label(),
                        step.status,
                        if self.save_status.is_saved() {
                            ""
                        } else {
                            " · not yet saved"
                        }
                    ))
                    .children(fields.into_iter().map(|(label, value)| {
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_color(theme::rambutan_500()).child(label))
                            .child(div().child(value))
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let blocked = latest
            .filter(|workflow| workflow.status == WorkflowStatus::Previewed)
            .map(|workflow| {
                workflow
                    .steps
                    .iter()
                    .map(|step| step.operation.domain())
                    .filter(|domain| {
                        !self
                            .automation_state
                            .permissions
                            .allows(*domain, AgentAccess::Write)
                    })
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let latest_running = self.automation_state.jobs.iter().rev().find(|job| {
            job.status == BackgroundJobStatus::Running
                && latest.is_some_and(|workflow| workflow.id == job.workflow_id)
        });
        let latest_paused = self.automation_state.jobs.iter().rev().find(|job| {
            job.status == BackgroundJobStatus::Paused
                && latest.is_some_and(|workflow| workflow.id == job.workflow_id)
        });
        let audit = self
            .automation_state
            .audit
            .iter()
            .rev()
            .take(20)
            .enumerate()
            .map(|(index, event)| {
                div()
                    .id(("automation-audit", index))
                    .mt_2()
                    .border_l_1()
                    .border_color(theme::blueberry_400().opacity(0.3))
                    .pl_3()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme::blueberry_400())
                            .child(format!("{:?}", event.kind)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(event.message.clone()),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let focus_suggestion = self
            .automation_state
            .surfaceable_suggestions(EpochMs::now().as_millis())
            .into_iter()
            .find(|suggestion| suggestion.id == "protect-focus")
            .cloned();

        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme::longan_950())
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .child(
                        div()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .text_size(px(18.0))
                            .text_color(theme::rambutan_100())
                            .child("Agent workflows"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_500())
                            .child("Preview changes, grant only what is needed, and keep every action reversible."),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_4()
                    .child(self.render_mcp_bridge(cx))
                    .when_some(self.automation_error.clone(), |panel, message| panel.child(
                        div().mb_3().text_size(px(12.0)).text_color(theme::lychee_300()).child(message)
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .items_start()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(self.automation_panel(
                                        "Local suggestion",
                                        div()
                                            .mt_2()
                                            .text_size(px(11.0))
                                            .text_color(theme::rambutan_400())
                                            .child(focus_suggestion.as_ref().map_or_else(
                                                || "No suggestion is due. Local suggestions appear at most three times, no more than daily.".to_string(),
                                                |suggestion| format!(
                                                    "{} — {} Evidence: {}",
                                                    suggestion.title,
                                                    suggestion.explanation,
                                                    suggestion.evidence
                                                ),
                                            ))
                                            .child(
                                                web_button("automation-preview", WebButtonVariant::Primary, WebButtonSize::Small, ButtonState::Enabled)
                                                    .aria_label("New dry run")
                                                    .mt_2()
                                                    .child("New dry run")
                                                    .on_click(cx.listener(|this, _event, _window, cx| this.preview_focus_automation(cx))),
                                            )
                                            .when(focus_suggestion.is_some(), |panel| panel.child(
                                                web_button("automation-dismiss-suggestion", WebButtonVariant::Ghost, WebButtonSize::Small, ButtonState::Enabled)
                                                    .aria_label("Dismiss focus suggestion")
                                                    .mt_2()
                                                    .child("Dismiss")
                                                    .on_click(cx.listener(|this, _event, _window, cx| this.dismiss_focus_suggestion(cx))),
                                            )),
                                    ))
                                    .child(self.automation_panel(
                                        "Workflow review",
                                        div()
                                            .child(self.render_automation_history(cx))
                                            .when_some(latest, |panel, workflow| panel.child(self.render_mcp_rollback_review(&workflow.id, cx)))
                                            .when(!blocked.is_empty(), |panel| panel.child(
                                                div().mt_2().text_size(px(11.0)).text_color(theme::durian_300()).child(format!(
                                                    "Grant Write for: {}.",
                                                    blocked.iter().map(|domain| domain.label()).collect::<Vec<_>>().join(", ")
                                                )),
                                            ))
                                            .when_some(latest.filter(|workflow| workflow.status == WorkflowStatus::Previewed).map(|workflow| workflow.id.clone()), |panel, reviewed_id| {
                                                let decline_id = reviewed_id.clone();
                                                let recheck_id = reviewed_id.clone();
                                                panel.child(
                                                div().flex().gap_2().child(
                                                web_button("automation-recheck", WebButtonVariant::Ghost, WebButtonSize::Small, ButtonState::Enabled)
                                                    .aria_label("Recheck grants")
                                                    .mt_2()
                                                    .child("Recheck grants")
                                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                                        if this.save_status.is_saved() {
                                                            this.automation_error = this
                                                                .automation_state
                                                                .preview(&recheck_id, EpochMs::now().as_millis())
                                                                .is_none()
                                                                .then(|| "This workflow can no longer be previewed. No records were changed.".into());
                                                            this.persist_and_notify(cx);
                                                        }
                                                    }))
                                                ).child(web_button("automation-start", WebButtonVariant::Primary, WebButtonSize::Small, ButtonState::disabled_if(!blocked.is_empty() || !self.save_status.is_saved()))
                                                    .debug_selector(|| "desktop-start-workflow".into())
                                                    .aria_label("Start transaction")
                                                    .mt_2()
                                                    .child("Start transaction")
                                                    .on_click(cx.listener(move |this, _event, _window, cx| this.start_reviewed_automation(&reviewed_id, cx))))
                                                .child(web_button("automation-decline", WebButtonVariant::Ghost, WebButtonSize::Small, ButtonState::disabled_if(!self.save_status.is_saved()))
                                                    .debug_selector(|| "desktop-decline-workflow".into())
                                                    .aria_label("Decline proposal")
                                                    .mt_2().child("Decline proposal")
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        if this.save_status.is_saved() && this.automation_state.decline_workflow(&decline_id, EpochMs::now().as_millis()) { this.persist_and_notify(cx); }
                                                    }))))
                                            }),
                                    ))
                                    .child(self.automation_panel(
                                        "Background job",
                                        div()
                                            .mt_2()
                                            .text_size(px(11.0))
                                            .text_color(theme::rambutan_400())
                                            .child("Background execution is opt-in and runs only while this app is open.")
                                            .child(if !self.save_status.is_saved() {
                                                "Changes are provisional · waiting for local save".to_string()
                                            } else if let Some(job) = latest_running {
                                                format!("Running · next checkpoint {}", job.next_step + 1)
                                            } else if let Some(job) = latest_paused {
                                                format!("Paused · resumes at checkpoint {}", job.next_step + 1)
                                            } else {
                                                "No active job.".to_string()
                                            })
                                            .when_some(latest_running.map(|job| job.id.clone()), |panel, job_id| panel.child(
                                                web_button("automation-run-next", WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::disabled_if(!self.save_status.is_saved()))
                                                    .debug_selector(|| "desktop-run-workflow-step".into())
                                                    .aria_label("Run next step")
                                                    .mt_2()
                                                    .child("Run next step")
                                                    .on_click(cx.listener(move |this, _event, _window, cx| this.run_automation_step(&job_id, cx))),
                                            ))
                                            .when_some(latest_running.map(|job| job.id.clone()), |panel, job_id| panel.child(
                                                web_button("automation-enable-background", WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::disabled_if(!self.save_status.is_saved() || latest_running.is_some_and(|job| job.auto_run)))
                                                    .aria_label("Run workflow in background")
                                                    .debug_selector(|| "desktop-enable-workflow-background".into())
                                                    .mt_2()
                                                    .child("Run in background")
                                                    .on_click(cx.listener(move |this, _, _, cx| this.enable_background_automation(&job_id, cx))),
                                            ))
                                            .when_some(latest_running.map(|job| job.id.clone()), |panel, job_id| panel.child(
                                                web_button("automation-pause", WebButtonVariant::Ghost, WebButtonSize::Small, ButtonState::Enabled)
                                                    .aria_label("Pause workflow")
                                                    .debug_selector(|| "desktop-pause-workflow".into())
                                                    .mt_2()
                                                    .child("Pause")
                                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                                        if this.automation_state.pause_job(&job_id, EpochMs::now().as_millis()) {
                                                            this.persist_and_notify(cx);
                                                        }
                                                    })),
                                            ))
                                            .when_some(latest_paused.map(|job| job.id.clone()), |panel, job_id| panel.child(
                                                web_button("automation-resume", WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::disabled_if(!self.save_status.is_saved()))
                                                    .aria_label("Resume workflow")
                                                    .debug_selector(|| "desktop-resume-workflow".into())
                                                    .mt_2()
                                                    .child("Resume")
                                                    .on_click(cx.listener(move |this, _, _, cx| this.resume_automation(&job_id, cx))),
                                            ))
                                            .when_some(latest.filter(|workflow| workflow.can_rollback()).map(|workflow| workflow.id.clone()), |panel, workflow_id| panel.child(
                                                web_button("automation-rollback", WebButtonVariant::Destructive, WebButtonSize::Small, ButtonState::disabled_if(!self.save_status.is_saved()))
                                                    .debug_selector(|| "desktop-undo-workflow".into())
                                                    .aria_label("Undo transaction")
                                                    .mt_2()
                                                    .child("Undo transaction")
                                                    .on_click(cx.listener(move |this, _event, _window, cx| this.rollback_reviewed_automation(&workflow_id, cx))),
                                            ))
                                            .children(workflow_rows),
                                    ))
                                    .child(self.automation_panel("Audit timeline", div()
                                        .when(!self.save_status.is_saved(), |el| el.child("Pending local save · entries below are provisional"))
                                        .children(audit))),
                            )
                            .child(
                                div()
                                    .w(px(320.0))
                                    .flex_none()
                                    .child(self.automation_panel(
                                        "Capability grants",
                                        div()
                                            .mt_1()
                                            .child(
                                                div()
                                                    .text_size(px(12.0))
                                                    .text_color(theme::rambutan_400())
                                                    .child("Read and Write are independent. Finance and Health start fully off."),
                                            )
                                            .children(grants)
                                            .child(
                                                div()
                                                    .mt_3()
                                                    .text_size(px(10.0))
                                                    .text_color(theme::pandan_300())
                                                    .child(format!(
                                                        "{} typed tools · {} workflows",
                                                        agent_tool_definitions().len(),
                                                        self.automation_state.workflows.len()
                                                    )),
                                            ),
                                    )),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod background_tests;

#[cfg(test)]
mod durability_tests {
    use super::{
        theme, AgentAccess, AgentDomain, AutomationState, DesktopState, SaveStatus, Workspace,
    };
    use gpui::AppContext;

    #[gpui::test]
    fn approval_starts_only_the_reviewed_workflow_not_a_newer_proposal(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.automation_state = AutomationState::default();
            for domain in [
                AgentDomain::Tasks,
                AgentDomain::Calendar,
                AgentDomain::Capture,
            ] {
                workspace
                    .automation_state
                    .permissions
                    .set(domain, AgentAccess::Write, true);
            }
            workspace.preview_focus_automation(cx);
            let reviewed = workspace.automation_state.workflows[0].id.clone();
            workspace.preview_focus_automation(cx);
            workspace.start_reviewed_automation(&reviewed, cx);
            assert_eq!(workspace.automation_state.jobs.len(), 1);
            assert_eq!(workspace.automation_state.jobs[0].workflow_id, reviewed);
            assert_eq!(
                workspace.automation_state.workflows[1].status,
                bruhs_models::WorkflowStatus::Previewed
            );
        });
    }

    #[gpui::test]
    fn unsaved_native_workflow_cannot_start_advance_or_undo(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.automation_state = AutomationState::default();
            workspace.work_items.clear();
            workspace.calendar_events.clear();
            workspace.captures.clear();
            for domain in [
                AgentDomain::Tasks,
                AgentDomain::Calendar,
                AgentDomain::Capture,
            ] {
                workspace
                    .automation_state
                    .permissions
                    .set(domain, AgentAccess::Write, true);
            }
            workspace.preview_focus_automation(cx);
            workspace.save_status = SaveStatus::Saving;
            workspace.start_latest_automation(cx);
            assert!(workspace.automation_state.jobs.is_empty());

            workspace.save_status.finish(Ok(()));
            workspace.start_latest_automation(cx);
            let job_id = workspace.automation_state.jobs[0].id.clone();
            for status in [
                SaveStatus::Saving,
                SaveStatus::Failed("disk full".into()),
                SaveStatus::RecoveryRequired("corrupt original".into()),
                SaveStatus::Conflict,
            ] {
                workspace.save_status = status;
                workspace.run_latest_automation_step(cx);
                assert!(workspace.work_items.is_empty());
                assert_eq!(workspace.automation_state.jobs[0].next_step, 0);
            }

            workspace.save_status.finish(Ok(()));
            for _ in 0..3 {
                workspace.run_latest_automation_step(cx);
            }
            assert_eq!(workspace.automation_state.jobs[0].id, job_id);
            assert_eq!(workspace.automation_state.jobs[0].next_step, 3);
            let before = workspace.snapshot(cx);
            workspace.save_status = SaveStatus::Failed("disk full".into());
            workspace.rollback_latest_automation(cx);
            assert_eq!(workspace.work_items.len(), 1);
            assert_eq!(workspace.calendar_events.len(), 1);
            assert_eq!(workspace.captures.len(), 1);
            assert_eq!(workspace.automation_state, before.automation_state);
        });
    }

    #[gpui::test]
    async fn stale_native_workspace_keeps_recovery_and_disables_retry(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::RefCell, fs, rc::Rc};
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let cell = Rc::new(RefCell::new(None));
        let capture = cell.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let first = cx.new(|cx| Workspace::new(window, cx));
            let stale = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some((first, stale.clone()));
            gpui_component::Root::new(stale, window, cx)
        });
        let (first, stale) = cell.borrow().clone().unwrap();
        let dir =
            std::env::temp_dir().join(format!("bruhs-native-stale-ui-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state.json");
        for workspace in [&first, &stale] {
            workspace.update_in(cx, |workspace, _, _| {
                workspace.persistence_test_path = Some(path.clone());
                workspace.automation_state = AutomationState::default();
            });
        }
        first.update_in(cx, |workspace, _, cx| {
            workspace.preview_focus_automation(cx)
        });
        cx.condition(&first, |workspace, _| workspace.save_status.is_saved())
            .await;
        let durable = fs::read(&path).unwrap();
        stale.update_in(cx, |workspace, _, cx| {
            workspace.preview_focus_automation(cx)
        });
        cx.condition(&stale, |workspace, _| {
            workspace.save_status == SaveStatus::Conflict
        })
        .await;
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-save-status").is_some());
        assert!(cx.debug_bounds("desktop-copy-recovery").is_some());
        assert!(cx.debug_bounds("desktop-retry-save").is_none());
        stale.update_in(cx, |workspace, _, cx| {
            assert_eq!(workspace.automation_state.workflows.len(), 1);
            workspace.persist_and_notify(cx);
            assert_eq!(workspace.save_status, SaveStatus::Conflict);
        });
        cx.run_until_parked();
        assert_eq!(fs::read(&path).unwrap(), durable);
        assert!(!path.with_file_name("state.json.bak").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    async fn native_workflow_write_failure_then_retry_commits_record_and_checkpoint_together(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::super::{desktop_state_backup_path, load_desktop_state_from_path};
        use std::{cell::RefCell, fs, rc::Rc};

        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let cell = Rc::new(RefCell::new(None));
        let capture = cell.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some(workspace.clone());
            gpui_component::Root::new(workspace, window, cx)
        });
        let workspace = cell.borrow().clone().unwrap();
        let dir =
            std::env::temp_dir().join(format!("bruhs-native-commit-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state.json");
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.persistence_test_path = Some(path.clone());
            workspace.automation_state = AutomationState::default();
            workspace.work_items.clear();
            workspace.calendar_events.clear();
            workspace.captures.clear();
            for domain in [
                AgentDomain::Tasks,
                AgentDomain::Calendar,
                AgentDomain::Capture,
            ] {
                workspace
                    .automation_state
                    .permissions
                    .set(domain, AgentAccess::Write, true);
            }
            workspace.preview_focus_automation(cx);
            assert_eq!(workspace.save_status, SaveStatus::Saving);
        });
        cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
            .await;
        workspace.update_in(cx, |workspace, _, cx| workspace.start_latest_automation(cx));
        cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
            .await;

        let durable_before = fs::read(&path).unwrap();
        let backup = desktop_state_backup_path(&path);
        let retained_backup = dir.join("retained-backup.json");
        fs::rename(&backup, &retained_backup).unwrap();
        fs::create_dir(&backup).unwrap();
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.run_latest_automation_step(cx);
            assert_eq!(workspace.work_items.len(), 1);
            assert_eq!(workspace.save_status, SaveStatus::Saving);
            // A double activation cannot create the next domain record.
            workspace.run_latest_automation_step(cx);
            assert_eq!(workspace.automation_state.jobs[0].next_step, 1);
        });
        cx.condition(&workspace, |workspace, _| {
            matches!(workspace.save_status, SaveStatus::Failed(_))
        })
        .await;
        assert_eq!(fs::read(&path).unwrap(), durable_before);
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.run_latest_automation_step(cx);
            assert_eq!(workspace.automation_state.jobs[0].next_step, 1);
            workspace.active_view = super::AppView::Today;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-save-status").is_some());
        let copy = cx.debug_bounds("desktop-copy-recovery").unwrap();
        cx.simulate_click(copy.center(), gpui::Modifiers::default());
        let recovery = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
        let recovery: super::PersistedWorkspaceState = serde_json::from_str(&recovery).unwrap();
        assert_eq!(recovery.work_items.len(), 1);
        assert_eq!(recovery.automation_state.jobs[0].next_step, 1);
        fs::remove_dir(&backup).unwrap();
        fs::rename(retained_backup, backup).unwrap();
        let retry = cx.debug_bounds("desktop-retry-save").unwrap();
        cx.simulate_click(retry.center(), gpui::Modifiers::default());
        cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
            .await;
        let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-save-status").is_none());
        assert_eq!(saved.work_items.len(), 1);
        assert_eq!(saved.automation_state.jobs[0].next_step, 1);
        workspace.update_in(cx, |workspace, _, _| {
            assert_eq!(saved.work_items[0].id, workspace.work_items[0].id);
            assert_eq!(saved.automation_state, workspace.automation_state);
        });
        fs::remove_dir_all(dir).unwrap();
    }
}
