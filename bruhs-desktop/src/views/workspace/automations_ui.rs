//! Native capability grants, transactional workflows, durable jobs, and audit.

use super::*;
use bruhs_ui::zoom::px;
mod chat;
mod draft_outcomes;
mod history_ui;
mod review_ui;
pub(in crate::views::workspace) use review_ui::{
    body_sm, card_title, label_sm, neutral_ring, web_card,
};
mod people;
#[cfg(test)]
mod reactive_tests;
#[cfg(test)]
mod review_tests;
mod rollback;
#[cfg(test)]
mod suggestion_tests;
use bruhs_models::logic::workflow_history;
use bruhs_models::{
    agent_tool_definitions, AgentAccess, AgentDomain, BackgroundJobStatus, FinanceTransaction,
    WorkflowOperation, WorkflowStatus, WorkoutLog,
};

/// Web `mx-auto max-w-6xl`: the page's content column.
fn page_column() -> Div {
    div().w_full().max_w(px(1152.0)).mx_auto()
}

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

/// Web `22rem`: the Capability grants column beside the workflows.
const AUTOMATION_GRANTS_WIDTH_PX: f32 = 352.0;

impl Workspace {
    pub(super) fn drive_background_automations(&mut self, cx: &mut Context<'_, Self>) {
        // Runs once after load and after every save: report draft decisions
        // back to their agents and keep agent update notices subscribed.
        self.start_reactive_awareness(cx);
        if self.save_status.is_saved() {
            self.reconcile_draft_outcomes(cx);
        }
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
            self.automation_error = Some(workflow_history::BACKGROUND_REFUSED.into());
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

    /// Web `WorkflowStatus`: the app-wide workflow notice, above every page
    /// but Agent workflows. Its words come from the shared rule, which never
    /// names a workflow or shows its payload.
    pub(super) fn render_workflow_status(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        use bruhs_models::logic::workflow_review as review;
        if self.active_view == AppView::Automations {
            return None;
        }
        let notice = self.workflow_notice()?;
        if notice.dismiss_key.is_some() && notice.dismiss_key == self.dismissed_workflow_notice {
            return None;
        }
        let text_button = |id: &'static str, label: &'static str, color: Hsla| {
            web_button(
                id,
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .px_2()
            .text_color(color)
            .aria_label(label)
        };
        Some(
            div()
                .id("workspace-workflow-status")
                .debug_selector(|| "desktop-workflow-status".into())
                .role(Role::Region)
                .aria_label(review::WORKFLOW_STATUS_LABEL)
                .w_full()
                .flex_shrink_0()
                .border_b_1()
                .border_color(theme::longan_800().opacity(0.5))
                .bg(theme::longan_900())
                .px_4()
                .py_2()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_x_3()
                        .gap_y_1()
                        .child(
                            review_ui::body_sm(
                                notice.message.clone(),
                                if notice.error {
                                    theme::lychee_300()
                                } else {
                                    theme::rambutan_300()
                                },
                            )
                            .id("workflow-status-message")
                            .role(if notice.error {
                                Role::Alert
                            } else {
                                Role::Status
                            })
                            .aria_label(notice.message.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .when(review::background_running(&self.automation_state), |row| {
                                    row.child(
                                        text_button(
                                            "pause-background-workflows",
                                            review::PAUSE_BACKGROUND_LABEL,
                                            theme::rambutan_300(),
                                        )
                                        .debug_selector(|| {
                                            "desktop-pause-background-workflows".into()
                                        })
                                        .child(review::PAUSE_BACKGROUND)
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                let ids: Vec<_> = this
                                                    .automation_state
                                                    .jobs
                                                    .iter()
                                                    .filter(|job| {
                                                        job.auto_run
                                                            && job.status
                                                                == BackgroundJobStatus::Running
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
                                                        tracing::debug!(
                                                            "job {id} was no longer running"
                                                        );
                                                    }
                                                }
                                                // Pause goes once its step settles and
                                                // takes the button with it: focus moves
                                                // to Open workflows, as on the web.
                                                this.workflow_notice_open_focus.focus(window, cx);
                                                this.persist_and_notify(cx);
                                            }),
                                        ),
                                    )
                                })
                                .child(
                                    text_button(
                                        "open-workflow-status",
                                        review::OPEN_WORKFLOWS,
                                        theme::persimmon_300(),
                                    )
                                    .track_focus(&self.workflow_notice_open_focus)
                                    .debug_selector(|| "desktop-open-workflows".into())
                                    .child(review::OPEN_WORKFLOWS)
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            // As on the web: the page, where each
                                            // waiting review has its own card.
                                            this.active_view = AppView::Automations;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .when_some(notice.dismiss_key, |row, key| {
                                    row.child(
                                        text_button(
                                            "dismiss-workflow-status",
                                            review::DISMISS_WORKFLOW_STATUS,
                                            theme::rambutan_400(),
                                        )
                                        .debug_selector(|| "desktop-dismiss-workflow-status".into())
                                        .child("Dismiss")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.dismissed_workflow_notice = Some(key.clone());
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// What the app-wide workflow notice says, by the shared rule.
    pub(super) fn workflow_notice(
        &self,
    ) -> Option<bruhs_models::logic::workflow_review::WorkflowNotice> {
        bruhs_models::logic::workflow_review::workflow_notice(
            &self.automation_state,
            // Like the web's failed store: a workflow error or a save that
            // failed, which also stops background steps.
            self.automation_error.is_some() || self.save_status.alert().is_some(),
            self.save_status.is_saved(),
            self.mcp_bridge.pending_review(&self.automation_state),
        )
    }

    /// Puts a new dry run on show, as the web's `make_preview` does: in
    /// Preview changes and in the history.
    pub(super) fn show_dry_run(&mut self, id: String) {
        self.dry_run_workflow_id = Some(id.clone());
        self.reviewed_workflow_id = Some(id);
    }

    /// Tests' shortcut to a previewed focus workflow that also counts the
    /// suggestion as shown (the card's Preview does that for any suggestion).
    #[cfg(test)]
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
        self.show_dry_run(id);
        self.persist_and_notify(cx);
    }

    /// Web "Dismiss suggestion".
    fn dismiss_suggestion(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if !self
            .automation_state
            .dismiss_suggestion(id, EpochMs::now().as_millis())
        {
            tracing::debug!("suggestion {id} was already gone");
        }
        self.persist_and_notify(cx);
    }

    /// Web "Preview N changes": counts the suggestion as shown, then
    /// previews its operations as a workflow to review.
    fn preview_suggestion(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let now = EpochMs::now().as_millis();
        let Some(suggestion) = self
            .automation_state
            .suggestions
            .iter()
            .find(|suggestion| suggestion.id == id)
            .cloned()
        else {
            return;
        };
        if !self.automation_state.mark_suggestion_surfaced(id, now) {
            tracing::debug!("suggestion {id} was already surfaced");
        }
        let workflow =
            self.automation_state
                .create_workflow(suggestion.title, suggestion.operations, now);
        if self.automation_state.preview(&workflow, now).is_none() {
            self.automation_error =
                Some("The workflow could not be previewed. No records were changed.".into());
        }
        self.show_dry_run(workflow);
        self.persist_and_notify(cx);
    }

    /// Web "New dry run": previews the focus workflow without counting a
    /// suggestion as shown.
    fn new_focus_dry_run(&mut self, cx: &mut Context<'_, Self>) {
        let now = EpochMs::now().as_millis();
        let workflow =
            self.automation_state
                .create_workflow("Protect focus", focus_operations(), now);
        if self.automation_state.preview(&workflow, now).is_none() {
            self.automation_error =
                Some("The focus workflow could not be previewed. No records were changed.".into());
        }
        self.show_dry_run(workflow);
        self.persist_and_notify(cx);
    }

    /// Web `Local suggestions`: the section's own dry run, then a card per
    /// suggestion that may surface, with its evidence, Dismiss and Preview.
    fn render_local_suggestions(&self, cx: &Context<'_, Self>) -> Div {
        use bruhs_models::suggestion_copy as copy;
        let now = EpochMs::now().as_millis();
        let cards = self
            .automation_state
            .surfaceable_suggestions(now)
            .into_iter()
            .enumerate()
            .map(|(index, suggestion)| {
                let preview_id = suggestion.id.clone();
                let dismiss_id = suggestion.id.clone();
                let preview = copy::preview(suggestion.operations.len());
                div()
                    .debug_selector(move || format!("desktop-suggestion-{index}"))
                    .rounded(px(review_ui::CARD_RADIUS_PX))
                    .bg(theme::longan_950().opacity(0.7))
                    .border_1()
                    .border_color(theme::longan_700().opacity(0.4))
                    .p(px(11.0))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(review_ui::label(
                                        suggestion.title.clone(),
                                        theme::rambutan_200(),
                                    ))
                                    .child(
                                        review_ui::body_sm(
                                            suggestion.explanation.clone(),
                                            theme::rambutan_400(),
                                        )
                                        .mt_1(),
                                    )
                                    .child(
                                        review_ui::label_sm(
                                            copy::evidence(&suggestion.evidence),
                                            theme::blueberry_300(),
                                        )
                                        .mt_2(),
                                    ),
                            )
                            .child(
                                web_button(
                                    ("automation-suggestion-dismiss", index),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(move || {
                                    format!("desktop-automation-suggestion-dismiss-{index}")
                                })
                                .aria_label(copy::DISMISS_LABEL)
                                // Web: plain `text-label-sm text-rambutan-500`.
                                .flex_none()
                                .min_h(px(0.0))
                                .p_0()
                                .text_color(theme::rambutan_500())
                                .child(copy::DISMISS)
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.dismiss_suggestion(&dismiss_id, cx);
                                    },
                                )),
                            ),
                    )
                    .child(
                        // A row, so the button keeps its own width as on the web.
                        div().mt_3().flex().child(
                            web_button(
                                ("automation-suggestion-preview", index),
                                WebButtonVariant::Primary,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(move || {
                                format!("desktop-automation-suggestion-preview-{index}")
                            })
                            .aria_label(preview.clone())
                            .child(preview)
                            .on_click(cx.listener(
                                move |this, _event, _window, cx| {
                                    this.preview_suggestion(&preview_id, cx);
                                },
                            )),
                        ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        review_ui::web_card(review_ui::neutral_ring())
            .debug_selector(|| "desktop-local-suggestions".into())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(review_ui::card_title(copy::TITLE))
                            .child(review_ui::body_sm(copy::DESCRIPTION, theme::rambutan_500())),
                    )
                    .child(
                        web_button(
                            "automation-new-dry-run",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-automation-new-dry-run".into())
                        .aria_label(copy::NEW_DRY_RUN)
                        .child(copy::NEW_DRY_RUN)
                        .on_click(
                            cx.listener(|this, _event, _window, cx| this.new_focus_dry_run(cx)),
                        ),
                    ),
            )
            .when(!cards.is_empty(), |panel| {
                panel.child(div().mt_3().flex().flex_col().gap_2().children(cards))
            })
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
        } else {
            // As on the web: it goes on show in the history, with its
            // Run, Pause and Undo controls.
            self.reviewed_workflow_id = Some(id.to_owned());
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
            self.automation_error = Some(workflow_history::RESUME_REFUSED.into());
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
                    source: None,
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
                    ..Default::default()
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
                .insert(0, transaction.into()),
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
                    self.cancel_quota_recovery(&id);
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

    /// A bruh's or a room's own page. The reactive panel draws it:
    /// `sync_bruh_page` has already told the panel which one is open.
    pub(super) fn render_bruh_page(&self) -> Div {
        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.reactive_automations.clone())
    }

    /// Tell the reactive panel which bruh or room page is open, before it
    /// draws, so it draws that page for `AppView::Bruh` or `AppView::Room`
    /// and its automations otherwise.
    pub(super) fn sync_bruh_page(&mut self, cx: &mut Context<'_, Self>) {
        let (bruh, room) = match &self.active_view {
            AppView::Bruh { agent_id } => (Some(agent_id.clone()), None),
            AppView::Room { room_id } => (None, Some(room_id.clone())),
            _ => (None, None),
        };
        let panel = self.reactive_automations.read(cx);
        let destination = self
            .task_thread_view
            .as_ref()
            .filter(|t| Some(t.agent_id.as_str()) == bruh.as_deref());
        if panel.bruh_page_agent() != bruh.as_deref()
            || panel.room_page_id() != room.as_deref()
            || panel.task_thread() != destination
        {
            self.reactive_automations.update(cx, |panel, cx| {
                if let Some(destination) = destination {
                    panel.show_task_thread(destination.clone(), cx);
                } else {
                    panel.show_bruh(bruh, cx);
                }
                panel.show_room(room, cx);
            });
        }
    }

    pub(super) fn render_automations_page(&self, window: &Window, cx: &Context<'_, Self>) -> Div {
        // Web `lg:grid-cols-[minmax(0,1fr)_22rem]`: the grants column sits
        // beside the workflows from `lg`, under them below.
        let stacked = Self::narrower_than(window, LG_BREAKPOINT_PX);
        // Web `px-4 py-4 sm:px-6 sm:py-5` around the page, `sm:px-6` in its
        // header.
        let compact = Self::narrower_than(window, SM_BREAKPOINT_PX);
        let grants = AgentDomain::ALL
            .into_iter()
            .enumerate()
            .map(|(index, domain)| {
                div()
                    .id(("automation-grant", index))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(review_ui::label_sm(domain.label(), theme::rambutan_300()).flex_1())
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.grant_toggle(index, domain, AgentAccess::Read, cx))
                            .child(self.grant_toggle(index, domain, AgentAccess::Write, cx)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
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
                    .flex_shrink_0()
                    .px(px(if compact { 16.0 } else { 24.0 }))
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .child(
                        div()
                            .font_family(theme::HEADING_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(18.0))
                            .text_color(theme::rambutan_100())
                            .child("Agent workflows"),
                    )
                    .child(
                        review_ui::body_sm(
                            "Preview changes, grant only what is needed, and keep every action reversible.",
                            theme::rambutan_500(),
                        )
                        .mt(px(2.0)),
                    ),
            )
            .child(
                div()
                    .id("automations-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.automations_scroll)
                    .px(px(if compact { 16.0 } else { 24.0 }))
                    .py(px(if compact { 16.0 } else { 20.0 }))
                    .child(self.reactive_automations.clone())
                    // Web `mx-auto mb-4 max-w-6xl`.
                    .child(page_column().mb_4().child(self.render_mcp_bridge(cx)))
                    .when_some(self.automation_error.clone(), |panel, message| {
                        panel.child(
                            page_column().mb_3().child(
                                div()
                                    .id("automation-error")
                                    .role(Role::Alert)
                                    .rounded(px(review_ui::CARD_RADIUS_PX))
                                    .bg(theme::longan_900())
                                    .px_3()
                                    .py_2()
                                    .child(review_ui::body_sm(message, theme::lychee_300())),
                            ),
                        )
                    })
                    .child(
                        page_column()
                            .flex()
                            .when(stacked, |content| content.flex_col())
                            .gap_4()
                            .items_start()
                            .child(
                                div()
                                    .debug_selector(|| "desktop-automation-workflows".into())
                                    .flex_1()
                                    .min_w_0()
                                    .when(stacked, |workflows| workflows.flex_none().w_full())
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .children(self.render_workflow_reviews(cx))
                                    .child(self.render_local_suggestions(cx))
                                    .children(self.render_preview_changes(cx))
                                    .child(self.render_background_jobs(cx))
                                    .child(self.render_audit_timeline()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "desktop-automation-grants".into())
                                    .w(px(AUTOMATION_GRANTS_WIDTH_PX))
                                    .when(stacked, |grants| grants.w_full())
                                    .flex_none()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        review_ui::web_card(review_ui::neutral_ring())
                                            .child(review_ui::card_title(
                                                bruhs_models::logic::workflow_review::CAPABILITY_GRANTS,
                                            ))
                                            .child(review_ui::body_sm(
                                                bruhs_models::logic::workflow_review::CAPABILITY_GRANTS_NOTE,
                                                theme::rambutan_500(),
                                            ))
                                            .child(
                                                div()
                                                    .mt_3()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_2()
                                                    .children(grants),
                                            ),
                                    )
                                    .child(self.render_safety_contract())
                                    .child(self.render_workflow_history(cx)),
                            ),
                    )
                    .when(!self.settings_visible(), |body| body.child(self.render_mobile(cx).mt_3()))
                    .vertical_scrollbar(&self.automations_scroll),
            )
    }

    /// Web's Read or Write toggle in Capability grants: a tinted pill when
    /// granted, a dark one when not.
    fn grant_toggle(
        &self,
        index: usize,
        domain: AgentDomain,
        access: AgentAccess,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let on = self.automation_state.permissions.allows(domain, access);
        let name = match access {
            AgentAccess::Read => "Read",
            AgentAccess::Write => "Write",
        };
        web_button(
            match access {
                AgentAccess::Read => ElementId::from(("automation-read", index)),
                AgentAccess::Write => ElementId::from(("automation-write", index)),
            },
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .aria_label(format!("Toggle {} {name} access", domain.label()))
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .min_h(px(0.0))
        // Web `px-2 py-1` with an inset ring; the border takes its 1px here.
        .px(px(7.0))
        .py(px(3.0))
        .border_1()
        .map(|toggle| {
            if on {
                // Web `bg-blueberry-500/15 text-blueberry-200
                // inset-ring-blueberry-400/20`.
                toggle
                    .bg(theme::blueberry_500().opacity(0.15))
                    .border_color(theme::blueberry_400().opacity(0.2))
                    .text_color(theme::blueberry_200())
            } else {
                toggle
                    .bg(theme::longan_950().opacity(0.6))
                    .border_color(theme::longan_700().opacity(0.4))
                    .text_color(theme::rambutan_400())
            }
        })
        .child(name)
        .on_click(cx.listener(move |this, _event, _window, cx| {
            let current = this.automation_state.permissions.allows(domain, access);
            this.automation_state
                .permissions
                .set(domain, access, !current);
            this.persist_and_notify(cx);
        }))
    }
}

impl Workspace {
    pub(super) fn task_thread_deletion(
        &mut self,
        event: &crate::views::reactive_automations::TaskThreadDeletion,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        use crate::views::reactive_automations::TaskThreadDeletion;
        match event {
            TaskThreadDeletion::Requested {
                thread_id,
                title,
                tasks,
            } => {
                let panel = self.reactive_automations.clone();
                let id = thread_id.clone();
                let linked = tasks.keys().cloned().collect();
                if !self.confirm_thread_delete {
                    panel.update(cx, |panel, cx| panel.delete_task_thread(id, linked, cx));
                    return;
                }
                let mut names = tasks.values();
                let detail = match (names.next(), names.next()) {
                    (None, _) => "This deletes the thread and its chat history.".into(),
                    (Some(name), None) => format!(
                        "This deletes the chat history and the linked scheduled task “{name}”."
                    ),
                    (Some(_), Some(_)) => format!(
                        "This deletes the chat history and {} linked scheduled tasks.\n\n{}",
                        tasks.len(),
                        tasks.values().cloned().collect::<Vec<_>>().join("\n")
                    ),
                };
                let response = window.prompt(
                    gpui::PromptLevel::Warning,
                    &format!("Delete “{title}”?"),
                    Some(&detail),
                    &[
                        gpui::PromptButton::Cancel("Cancel".into()),
                        gpui::PromptButton::Other("Delete".into()),
                    ],
                    cx,
                );
                cx.spawn(async move |_, cx| {
                    if response.await == Ok(1) {
                        panel.update(cx, |panel, cx| panel.delete_task_thread(id, linked, cx));
                    }
                })
                .detach();
            }
            TaskThreadDeletion::Failed(reason) => {
                drop(window.prompt(
                    gpui::PromptLevel::Warning,
                    "Couldn’t delete the conversation",
                    Some(reason),
                    &[gpui::PromptButton::Other("OK".into())],
                    cx,
                ));
            }
            TaskThreadDeletion::Deleted(id) => {
                if self.threads.iter().any(|t| t.id == id.as_str()) {
                    self.delete_thread_by_id(id.as_str(), cx);
                }
                if let Some(loc) = self
                    .task_thread_view
                    .as_ref()
                    .filter(|t| &t.thread_id == id)
                {
                    let here = matches!(&self.active_view, AppView::Bruh {agent_id} if agent_id==&loc.agent_id);
                    self.task_thread_view = None;
                    if here && self.overlay.is_none() && self.top_layer.is_none() {
                        self.open_settings(&OpenSettings, window, cx);
                        self.set_settings_section(SettingsSection::ScheduledTasks, cx);
                    }
                    self.persist_and_notify(cx);
                }
            }
        }
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
            bruhs_ui::init(cx);
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
            bruhs_ui::init(cx);
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
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let cell = Rc::new(RefCell::new(None));
        let capture = cell.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let first = cx.new(|cx| Workspace::new(window, cx));
            let stale = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some((first, stale.clone()));
            bruhs_ui::Root::new(stale, window, cx)
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
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let cell = Rc::new(RefCell::new(None));
        let capture = cell.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some(workspace.clone());
            bruhs_ui::Root::new(workspace, window, cx)
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
