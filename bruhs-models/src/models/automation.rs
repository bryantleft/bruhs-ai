//! Capability-scoped, auditable cross-domain agent workflows.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SUGGESTION_COOLDOWN_MS: i64 = 24 * 60 * 60 * 1_000;

/// Why a workflow cannot be rolled back; nothing was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum WorkflowRollbackError {
    WorkflowMissing,
    NotRollbackable,
    WriteGrantRevoked,
    IncompleteStep,
    MissingReceipt,
    MissingSnapshot,
    RecordChanged,
}

impl WorkflowRollbackError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::WorkflowMissing => "Workflow no longer exists.",
            Self::NotRollbackable => {
                "Only an unchanged, started workflow with completed steps can be rolled back."
            }
            Self::WriteGrantRevoked => {
                "A required Write grant was revoked. No records were changed."
            }
            Self::IncompleteStep => "The workflow has an incomplete step. No records were changed.",
            Self::MissingReceipt => "Missing record receipt. No records were changed.",
            Self::MissingSnapshot => {
                "This older receipt has no rollback snapshot. No records were changed."
            }
            Self::RecordChanged => {
                "A created record was edited, removed, or is unavailable. No records were changed."
            }
        }
    }
}

/// Use epoch instants on every surface; converting a naive local time to UTC
/// would shift the focus event by the user's timezone offset.
pub fn focus_workflow_operations(now_ms: i64) -> Vec<WorkflowOperation> {
    let start_at_ms = ((crate::EpochMs::saturating_from_i64(now_ms).as_f64() / 1_800_000.0).ceil()
        + 1.0)
        * 1_800_000.0;
    vec![
        WorkflowOperation::CreateTask {
            title: "Finish the highest-impact task".into(),
        },
        WorkflowOperation::CreateCalendarEvent {
            title: "Protected focus".into(),
            start_at_ms,
            end_at_ms: start_at_ms + 45.0 * 60_000.0,
        },
        WorkflowOperation::AddCapture {
            content: "Focus plan\nProtected 45 minutes for the highest-impact task.".into(),
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDomain {
    Chat,
    Tasks,
    Calendar,
    Meetings,
    Knowledge,
    Inbox,
    People,
    Finance,
    Health,
    Capture,
}

impl AgentDomain {
    pub const ALL: [Self; 10] = [
        Self::Chat,
        Self::Tasks,
        Self::Calendar,
        Self::Meetings,
        Self::Knowledge,
        Self::Inbox,
        Self::People,
        Self::Finance,
        Self::Health,
        Self::Capture,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Tasks => "tasks",
            Self::Calendar => "calendar",
            Self::Meetings => "meetings",
            Self::Knowledge => "knowledge",
            Self::Inbox => "inbox",
            Self::People => "people",
            Self::Finance => "finance",
            Self::Health => "health",
            Self::Capture => "capture",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Tasks => "Tasks",
            Self::Calendar => "Calendar",
            Self::Meetings => "Meetings",
            Self::Knowledge => "Knowledge",
            Self::Inbox => "Inbox",
            Self::People => "People",
            Self::Finance => "Finance",
            Self::Health => "Health",
            Self::Capture => "Capture",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentCapability {
    pub domain: AgentDomain,
    pub access: AgentAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPermissionPolicy {
    pub grants: Vec<AgentCapability>,
}

impl Default for AgentPermissionPolicy {
    fn default() -> Self {
        Self {
            grants: AgentDomain::ALL
                .into_iter()
                .filter(|domain| !matches!(domain, AgentDomain::Finance | AgentDomain::Health))
                .map(|domain| AgentCapability {
                    domain,
                    access: AgentAccess::Read,
                })
                .collect(),
        }
    }
}

impl AgentPermissionPolicy {
    pub fn allows(&self, domain: AgentDomain, access: AgentAccess) -> bool {
        self.grants.contains(&AgentCapability { domain, access })
    }

    pub fn set(&mut self, domain: AgentDomain, access: AgentAccess, allowed: bool) {
        let capability = AgentCapability { domain, access };
        if allowed {
            if !self.grants.contains(&capability) {
                self.grants.push(capability);
            }
        } else {
            self.grants.retain(|grant| *grant != capability);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolDefinition {
    pub name: String,
    pub description: String,
    pub domain: AgentDomain,
    pub access: AgentAccess,
}

/// Stable typed tool surface. Each domain has an independently grantable read
/// and write tool; concrete writes are carried by [`WorkflowOperation`].
pub fn agent_tool_definitions() -> Vec<AgentToolDefinition> {
    AgentDomain::ALL
        .into_iter()
        .flat_map(|domain| {
            [AgentAccess::Read, AgentAccess::Write]
                .into_iter()
                .map(move |access| {
                    let verb = match access {
                        AgentAccess::Read => "read",
                        AgentAccess::Write => "write",
                    };
                    AgentToolDefinition {
                        name: format!("bruhs_{verb}_{}", domain.key()),
                        description: format!(
                            "{verb} {} records with an explicit {} capability grant",
                            domain.label(),
                            domain.label()
                        ),
                        domain,
                        access,
                    }
                })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowOperation {
    CreateChatThread {
        title: String,
    },
    CreateTask {
        title: String,
    },
    CreateCalendarEvent {
        title: String,
        start_at_ms: f64,
        end_at_ms: f64,
    },
    CreateMeetingNote {
        title: String,
        #[serde(default)]
        body: String,
    },
    CreateKnowledgeNote {
        title: String,
        body: String,
    },
    DraftInboxMessage {
        recipient: String,
        body: String,
    },
    CreatePersonReminder {
        /// Legacy name-only requests remain readable, but fail validation.
        #[serde(default)]
        person_id: String,
        person: String,
        reminder: String,
    },
    AddFinanceTransaction {
        merchant: String,
        category: String,
        #[serde(with = "super::money::serde_minor::signed")]
        amount_minor: i64,
    },
    LogWorkout {
        activity: String,
        minutes: u16,
    },
    AddCapture {
        content: String,
    },
}

impl WorkflowOperation {
    /// Complete review data; summary() is only a navigation label, never an
    /// approval surface. Surfaces format UTC instants using their platform API.
    pub fn review_fields(
        &self,
        format_time: impl Fn(f64) -> String,
    ) -> Vec<(&'static str, String)> {
        match self {
            Self::CreateChatThread { title } | Self::CreateTask { title } => {
                vec![("Title", title.clone())]
            }
            Self::CreateCalendarEvent {
                title,
                start_at_ms,
                end_at_ms,
            } => vec![
                ("Title", title.clone()),
                ("Start (UTC)", format_time(*start_at_ms)),
                ("End (UTC)", format_time(*end_at_ms)),
            ],
            Self::CreateKnowledgeNote { title, body } | Self::CreateMeetingNote { title, body } => {
                vec![("Title", title.clone()), ("Body", body.clone())]
            }
            Self::DraftInboxMessage { recipient, body } => {
                vec![("Recipient", recipient.clone()), ("Body", body.clone())]
            }
            Self::CreatePersonReminder {
                person_id,
                person,
                reminder,
            } => vec![
                ("Person ID", person_id.clone()),
                ("Person", person.clone()),
                ("Reminder", reminder.clone()),
            ],
            Self::AddFinanceTransaction {
                merchant,
                category,
                amount_minor,
            } => vec![
                ("Merchant", merchant.clone()),
                ("Category", category.clone()),
                (
                    "Amount (USD minor units; negative is spend)",
                    amount_minor.to_string(),
                ),
            ],
            Self::LogWorkout { activity, minutes } => vec![
                ("Activity", activity.clone()),
                ("Minutes", minutes.to_string()),
            ],
            Self::AddCapture { content } => vec![("Content", content.clone())],
        }
    }

    /// Validate before preview and again before execution. Adapters must never
    /// silently replace malformed dates or empty content with defaults.
    pub fn is_valid(&self) -> bool {
        let text = |value: &str, limit: usize| !value.trim().is_empty() && value.len() <= limit;
        match self {
            Self::CreateChatThread { title } | Self::CreateTask { title } => text(title, 4_096),
            Self::CreateMeetingNote { title, body } => {
                super::MeetingNote::valid_content(title, body)
            }
            Self::CreateCalendarEvent {
                title,
                start_at_ms,
                end_at_ms,
            } => {
                let instant = |value: f64| {
                    value.is_finite()
                        && value.fract() == 0.0
                        && (-62_135_596_800_000.0..=253_402_300_799_999.0).contains(&value)
                };
                text(title, 4_096)
                    && instant(*start_at_ms)
                    && instant(*end_at_ms)
                    && end_at_ms > start_at_ms
            }
            Self::CreateKnowledgeNote { title, body } => text(title, 4_096) && text(body, 65_536),
            Self::DraftInboxMessage { recipient, body } => {
                text(recipient, 4_096) && text(body, 65_536)
            }
            Self::CreatePersonReminder {
                person_id,
                person,
                reminder,
            } => text(person_id, 128) && text(person, 4_096) && text(reminder, 65_536),
            Self::AddFinanceTransaction {
                merchant,
                category,
                amount_minor,
            } => text(merchant, 4_096) && text(category, 4_096) && *amount_minor != i64::MIN,
            Self::LogWorkout { activity, minutes } => {
                text(activity, 4_096) && (1..=1_440).contains(minutes)
            }
            Self::AddCapture { content } => text(content, 65_536),
        }
    }

    pub const fn domain(&self) -> AgentDomain {
        match self {
            Self::CreateChatThread { .. } => AgentDomain::Chat,
            Self::CreateTask { .. } => AgentDomain::Tasks,
            Self::CreateCalendarEvent { .. } => AgentDomain::Calendar,
            Self::CreateMeetingNote { .. } => AgentDomain::Meetings,
            Self::CreateKnowledgeNote { .. } => AgentDomain::Knowledge,
            Self::DraftInboxMessage { .. } => AgentDomain::Inbox,
            Self::CreatePersonReminder { .. } => AgentDomain::People,
            Self::AddFinanceTransaction { .. } => AgentDomain::Finance,
            Self::LogWorkout { .. } => AgentDomain::Health,
            Self::AddCapture { .. } => AgentDomain::Capture,
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::CreateChatThread { title } => format!("Create chat “{title}”"),
            Self::CreateTask { title } => format!("Create task “{title}”"),
            Self::CreateCalendarEvent { title, .. } => format!("Create event “{title}”"),
            Self::CreateMeetingNote { title, .. } => format!("Create meeting note “{title}”"),
            Self::CreateKnowledgeNote { title, .. } => format!("Save note “{title}”"),
            Self::DraftInboxMessage { recipient, .. } => {
                format!("Draft message to {recipient}")
            }
            Self::CreatePersonReminder { person, .. } => {
                format!("Add follow-up for {person}")
            }
            Self::AddFinanceTransaction { merchant, .. } => {
                format!("Add transaction from {merchant}")
            }
            Self::LogWorkout { activity, minutes } => {
                format!("Log {minutes}-minute {activity}")
            }
            Self::AddCapture { content } => {
                let preview = content.chars().take(48).collect::<String>();
                format!("Capture “{preview}”")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Draft,
    Previewed,
    Declined,
    Running,
    Paused,
    Completed,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepStatus {
    Pending,
    Completed,
    Failed,
    Reverted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub operation: WorkflowOperation,
    pub status: WorkflowStepStatus,
    pub target_id: Option<String>,
    pub error: Option<String>,
    /// Adapter-owned exact serialized record at creation. Old receipts lack
    /// this evidence and cannot be automatically rolled back.
    #[serde(default)]
    pub target_snapshot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub title: String,
    pub status: WorkflowStatus,
    pub steps: Vec<WorkflowStep>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Bind execution to the exact reviewed operations, not just their domains.
    /// Missing legacy approvals fail closed and require a new preview.
    #[serde(default)]
    previewed_operations: Option<Vec<WorkflowOperation>>,
}

impl Workflow {
    pub fn can_rollback(&self) -> bool {
        matches!(
            self.status,
            WorkflowStatus::Running
                | WorkflowStatus::Paused
                | WorkflowStatus::Completed
                | WorkflowStatus::Failed
        ) && self.matches_preview()
            && self
                .steps
                .iter()
                .any(|step| step.status == WorkflowStepStatus::Completed)
    }

    pub fn draft(
        title: impl Into<String>,
        operations: Vec<WorkflowOperation>,
        now_ms: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            status: WorkflowStatus::Draft,
            steps: operations
                .into_iter()
                .map(|operation| WorkflowStep {
                    operation,
                    status: WorkflowStepStatus::Pending,
                    target_id: None,
                    error: None,
                    target_snapshot: None,
                })
                .collect(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            previewed_operations: None,
        }
    }

    fn matches_preview(&self) -> bool {
        self.previewed_operations
            .as_ref()
            .is_some_and(|operations| {
                operations.len() == self.steps.len()
                    && operations.iter().zip(&self.steps).all(|(operation, step)| {
                        operation == &step.operation && operation.is_valid()
                    })
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPreview {
    pub allowed: bool,
    pub summaries: Vec<String>,
    pub blocked_domains: Vec<AgentDomain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundJobStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundJob {
    pub id: String,
    pub workflow_id: String,
    pub status: BackgroundJobStatus,
    pub next_step: usize,
    pub updated_at_ms: i64,
    /// Legacy manually stepped jobs must not start running on their own.
    #[serde(default)]
    pub auto_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    Previewed,
    Declined,
    Started,
    StepCompleted,
    Paused,
    Resumed,
    Failed,
    RolledBack,
    SuggestionDismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAuditEvent {
    pub id: String,
    pub workflow_id: Option<String>,
    pub kind: AuditKind,
    pub message: String,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveSuggestion {
    pub id: String,
    pub title: String,
    pub explanation: String,
    pub evidence: String,
    pub domain: AgentDomain,
    pub operations: Vec<WorkflowOperation>,
    pub dismissed: bool,
    pub shown_count: u8,
    pub last_shown_at_ms: Option<i64>,
}

impl ProactiveSuggestion {
    pub fn can_surface(&self, now_ms: i64) -> bool {
        !self.dismissed
            && self.shown_count < 3
            && self
                .last_shown_at_ms
                .is_none_or(|last| now_ms.saturating_sub(last) >= SUGGESTION_COOLDOWN_MS)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AutomationState {
    pub permissions: AgentPermissionPolicy,
    pub workflows: Vec<Workflow>,
    pub jobs: Vec<BackgroundJob>,
    pub audit: Vec<AgentAuditEvent>,
    pub suggestions: Vec<ProactiveSuggestion>,
}

impl AutomationState {
    pub fn create_workflow(
        &mut self,
        title: impl Into<String>,
        operations: Vec<WorkflowOperation>,
        now_ms: i64,
    ) -> String {
        let workflow = Workflow::draft(title, operations, now_ms);
        let id = workflow.id.clone();
        self.workflows.push(workflow);
        id
    }

    pub fn preview(&mut self, workflow_id: &str, now_ms: i64) -> Option<WorkflowPreview> {
        let workflow = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)?;
        if !matches!(
            workflow.status,
            WorkflowStatus::Draft | WorkflowStatus::Previewed
        ) || workflow.steps.is_empty()
            || workflow.steps.len() > 128
            || workflow.steps.iter().any(|step| !step.operation.is_valid())
            || workflow
                .steps
                .iter()
                .any(|step| step.status != WorkflowStepStatus::Pending)
            || self.jobs.iter().any(|job| job.workflow_id == workflow_id)
        {
            return None;
        }
        let mut blocked_domains = Vec::new();
        for step in &workflow.steps {
            let domain = step.operation.domain();
            if !self.permissions.allows(domain, AgentAccess::Write)
                && !blocked_domains.contains(&domain)
            {
                blocked_domains.push(domain);
            }
        }
        let summaries = workflow
            .steps
            .iter()
            .map(|step| step.operation.summary())
            .collect();
        workflow.status = WorkflowStatus::Previewed;
        workflow.updated_at_ms = now_ms;
        let allowed = blocked_domains.is_empty();
        workflow.previewed_operations = allowed.then(|| {
            workflow
                .steps
                .iter()
                .map(|step| step.operation.clone())
                .collect()
        });
        self.push_audit(
            Some(workflow_id.to_string()),
            AuditKind::Previewed,
            if allowed {
                "Dry run passed every capability check.".into()
            } else {
                format!("Dry run blocked {} domain grant(s).", blocked_domains.len())
            },
            now_ms,
        );
        Some(WorkflowPreview {
            allowed,
            summaries,
            blocked_domains,
        })
    }

    /// A user may decline an unstarted proposal without deleting its audit.
    pub fn decline_workflow(&mut self, workflow_id: &str, now_ms: i64) -> bool {
        let Some(workflow) = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
        else {
            return false;
        };
        if !matches!(
            workflow.status,
            WorkflowStatus::Draft | WorkflowStatus::Previewed
        ) || self.jobs.iter().any(|job| job.workflow_id == workflow_id)
        {
            return false;
        }
        workflow.status = WorkflowStatus::Declined;
        workflow.previewed_operations = None;
        workflow.updated_at_ms = now_ms;
        self.push_audit(
            Some(workflow_id.into()),
            AuditKind::Declined,
            "User declined this proposal. No records were changed.".into(),
            now_ms,
        );
        true
    }

    pub fn start_job(&mut self, workflow_id: &str, now_ms: i64) -> Option<String> {
        let workflow = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)?;
        if workflow.status != WorkflowStatus::Previewed
            || !workflow.matches_preview()
            || workflow.steps.is_empty()
            || self.jobs.iter().any(|job| job.workflow_id == workflow_id)
            || workflow
                .steps
                .iter()
                .any(|step| step.status != WorkflowStepStatus::Pending)
            || workflow.steps.iter().any(|step| {
                !self
                    .permissions
                    .allows(step.operation.domain(), AgentAccess::Write)
            })
        {
            return None;
        }
        workflow.status = WorkflowStatus::Running;
        workflow.updated_at_ms = now_ms;
        let job = BackgroundJob {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.to_string(),
            status: BackgroundJobStatus::Running,
            next_step: 0,
            updated_at_ms: now_ms,
            auto_run: false,
        };
        let id = job.id.clone();
        self.jobs.push(job);
        self.push_audit(
            Some(workflow_id.to_string()),
            AuditKind::Started,
            "Transaction started after preview.".into(),
            now_ms,
        );
        Some(id)
    }

    pub fn complete_step(
        &mut self,
        job_id: &str,
        target_id: impl Into<String>,
        now_ms: i64,
    ) -> bool {
        if self.pending_operation(job_id).is_none() {
            return false;
        }
        let target_id = target_id.into();
        if target_id.trim().is_empty() {
            return false;
        }
        let Some(job_index) = self.jobs.iter().position(|job| job.id == job_id) else {
            return false;
        };
        let workflow_id = self.jobs[job_index].workflow_id.clone();
        let Some(workflow_index) = self
            .workflows
            .iter()
            .position(|workflow| workflow.id == workflow_id)
        else {
            return false;
        };
        let step_index = self.jobs[job_index].next_step;
        let summary = {
            let Some(step) = self.workflows[workflow_index].steps.get_mut(step_index) else {
                return false;
            };
            step.status = WorkflowStepStatus::Completed;
            step.target_id = Some(target_id);
            step.operation.summary()
        };
        self.jobs[job_index].next_step += 1;
        self.jobs[job_index].updated_at_ms = now_ms;
        self.workflows[workflow_index].updated_at_ms = now_ms;
        if self.jobs[job_index].next_step == self.workflows[workflow_index].steps.len() {
            self.jobs[job_index].status = BackgroundJobStatus::Completed;
            self.workflows[workflow_index].status = WorkflowStatus::Completed;
        }
        self.push_audit(Some(workflow_id), AuditKind::StepCompleted, summary, now_ms);
        true
    }

    /// Recheck state and all remaining grants immediately before a record
    /// mutation. Executors must not use a previously cloned job as authority.
    pub fn pending_operation(&self, job_id: &str) -> Option<&WorkflowOperation> {
        let job = self.jobs.iter().find(|job| job.id == job_id)?;
        if job.status != BackgroundJobStatus::Running {
            return None;
        }
        let workflow = self
            .workflows
            .iter()
            .find(|workflow| workflow.id == job.workflow_id)?;
        if workflow.status != WorkflowStatus::Running
            || !workflow.matches_preview()
            || !self.remaining_steps_allowed(workflow, job.next_step)
        {
            return None;
        }
        let step = workflow.steps.get(job.next_step)?;
        (step.status == WorkflowStepStatus::Pending).then_some(&step.operation)
    }

    fn remaining_steps_allowed(&self, workflow: &Workflow, next_step: usize) -> bool {
        workflow.matches_preview()
            && next_step < workflow.steps.len()
            && workflow.steps[..next_step].iter().all(|step| {
                step.status == WorkflowStepStatus::Completed
                    && step
                        .target_id
                        .as_ref()
                        .is_some_and(|id| !id.trim().is_empty())
            })
            && workflow.steps[next_step..].iter().all(|step| {
                step.status == WorkflowStepStatus::Pending
                    && self
                        .permissions
                        .allows(step.operation.domain(), AgentAccess::Write)
            })
    }

    /// Checkpoint with rollback evidence only after an adapter has created the
    /// real record. Storage adapters must commit record and checkpoint together.
    pub fn complete_step_with_snapshot(
        &mut self,
        job_id: &str,
        target_id: impl Into<String>,
        snapshot: String,
        now_ms: i64,
    ) -> bool {
        if snapshot.is_empty() || self.pending_operation(job_id).is_none() {
            return false;
        }
        let Some(job) = self.jobs.iter().find(|job| job.id == job_id) else {
            return false;
        };
        let workflow_id = job.workflow_id.clone();
        let step_index = job.next_step;
        if !self.complete_step(job_id, target_id, now_ms) {
            return false;
        }
        if let Some(workflow) = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
        {
            workflow.steps[step_index].target_snapshot = Some(snapshot);
        }
        true
    }

    /// Preflight the entire rollback before deleting anything. Compare full
    /// adapter snapshots, so edits to any field preserve the user's record.
    /// This does not replace an adapter's atomic storage transaction.
    pub fn rollback_targets(
        &self,
        workflow_id: &str,
        current_snapshot: impl Fn(AgentDomain, &str) -> Option<String>,
    ) -> Result<Vec<(AgentDomain, String)>, WorkflowRollbackError> {
        let workflow = self
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .ok_or(WorkflowRollbackError::WorkflowMissing)?;
        if !workflow.can_rollback() {
            return Err(WorkflowRollbackError::NotRollbackable);
        }
        let mut targets = Vec::with_capacity(workflow.steps.len());
        for step in workflow.steps.iter().rev() {
            if matches!(
                step.status,
                WorkflowStepStatus::Pending | WorkflowStepStatus::Failed
            ) && step.target_id.is_none()
                && step.target_snapshot.is_none()
            {
                continue;
            }
            let domain = step.operation.domain();
            if !self.permissions.allows(domain, AgentAccess::Write) {
                return Err(WorkflowRollbackError::WriteGrantRevoked);
            }
            if step.status != WorkflowStepStatus::Completed {
                return Err(WorkflowRollbackError::IncompleteStep);
            }
            let id = step
                .target_id
                .as_deref()
                .ok_or(WorkflowRollbackError::MissingReceipt)?;
            let expected = step
                .target_snapshot
                .as_ref()
                .ok_or(WorkflowRollbackError::MissingSnapshot)?;
            if current_snapshot(domain, id).as_ref() != Some(expected) {
                return Err(WorkflowRollbackError::RecordChanged);
            }
            targets.push((domain, id.to_string()));
        }
        Ok(targets)
    }

    pub fn pause_job(&mut self, job_id: &str, now_ms: i64) -> bool {
        let Some(job) = self.jobs.iter_mut().find(|job| job.id == job_id) else {
            return false;
        };
        if job.status != BackgroundJobStatus::Running {
            return false;
        }
        job.status = BackgroundJobStatus::Paused;
        job.updated_at_ms = now_ms;
        let workflow_id = job.workflow_id.clone();
        if let Some(workflow) = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
        {
            workflow.status = WorkflowStatus::Paused;
            workflow.updated_at_ms = now_ms;
        }
        self.push_audit(
            Some(workflow_id),
            AuditKind::Paused,
            "Background job paused at a durable step boundary.".into(),
            now_ms,
        );
        true
    }

    pub fn enable_background(&mut self, job_id: &str) -> bool {
        if self.pending_operation(job_id).is_none() {
            return false;
        }
        if let Some(job) = self.jobs.iter_mut().find(|job| job.id == job_id) {
            job.auto_run = true;
            return true;
        }
        false
    }

    pub fn resume_job(&mut self, job_id: &str, now_ms: i64) -> bool {
        let Some(job_index) = self.jobs.iter().position(|job| job.id == job_id) else {
            return false;
        };
        let job = &self.jobs[job_index];
        let Some(workflow) = self
            .workflows
            .iter()
            .find(|workflow| workflow.id == job.workflow_id)
        else {
            return false;
        };
        if job.status != BackgroundJobStatus::Paused
            || workflow.status != WorkflowStatus::Paused
            || !self.remaining_steps_allowed(workflow, job.next_step)
        {
            return false;
        }
        let job = &mut self.jobs[job_index];
        job.status = BackgroundJobStatus::Running;
        job.updated_at_ms = now_ms;
        let workflow_id = job.workflow_id.clone();
        if let Some(workflow) = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
        {
            workflow.status = WorkflowStatus::Running;
            workflow.updated_at_ms = now_ms;
        }
        self.push_audit(
            Some(workflow_id),
            AuditKind::Resumed,
            "Background job resumed from its next unapplied step.".into(),
            now_ms,
        );
        true
    }

    pub fn mark_rolled_back(&mut self, workflow_id: &str, now_ms: i64) -> bool {
        let Some(workflow) = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.id == workflow_id)
        else {
            return false;
        };
        if !workflow.can_rollback() {
            return false;
        }
        for step in workflow.steps.iter_mut().rev() {
            if step.status == WorkflowStepStatus::Completed {
                step.status = WorkflowStepStatus::Reverted;
            }
        }
        workflow.status = WorkflowStatus::RolledBack;
        workflow.updated_at_ms = now_ms;
        for job in self
            .jobs
            .iter_mut()
            .filter(|job| job.workflow_id == workflow_id)
        {
            job.status = BackgroundJobStatus::RolledBack;
            job.auto_run = false;
            job.updated_at_ms = now_ms;
        }
        self.push_audit(
            Some(workflow_id.to_string()),
            AuditKind::RolledBack,
            "Every completed step was reverted in reverse order.".into(),
            now_ms,
        );
        true
    }

    pub fn surfaceable_suggestions(&self, now_ms: i64) -> Vec<&ProactiveSuggestion> {
        self.suggestions
            .iter()
            .filter(|suggestion| suggestion.can_surface(now_ms))
            .collect()
    }

    pub fn mark_suggestion_surfaced(&mut self, id: &str, now_ms: i64) -> bool {
        let Some(suggestion) = self
            .suggestions
            .iter_mut()
            .find(|suggestion| suggestion.id == id)
        else {
            return false;
        };
        if !suggestion.can_surface(now_ms) {
            return false;
        }
        suggestion.shown_count = suggestion.shown_count.saturating_add(1);
        suggestion.last_shown_at_ms = Some(now_ms);
        true
    }

    pub fn dismiss_suggestion(&mut self, id: &str, now_ms: i64) -> bool {
        let Some(suggestion) = self
            .suggestions
            .iter_mut()
            .find(|suggestion| suggestion.id == id)
        else {
            return false;
        };
        suggestion.dismissed = true;
        let title = suggestion.title.clone();
        self.push_audit(
            None,
            AuditKind::SuggestionDismissed,
            format!("Dismissed suggestion: {title}"),
            now_ms,
        );
        true
    }

    fn push_audit(
        &mut self,
        workflow_id: Option<String>,
        kind: AuditKind,
        message: String,
        occurred_at_ms: i64,
    ) {
        self.audit.push(AgentAuditEvent {
            id: Uuid::new_v4().to_string(),
            workflow_id,
            kind,
            message,
            occurred_at_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finance_workflow_amounts_are_exact_strings_and_accept_legacy_integers() {
        for amount_minor in [-9_007_199_254_740_993, i64::MAX, 123] {
            let operation = WorkflowOperation::AddFinanceTransaction {
                merchant: "Receipt".into(),
                category: "Other".into(),
                amount_minor,
            };
            let mut encoded = serde_json::to_value(&operation).unwrap();
            assert_eq!(encoded["amount_minor"], amount_minor.to_string());
            assert_eq!(
                serde_json::from_value::<WorkflowOperation>(encoded.clone()).unwrap(),
                operation
            );
            encoded["amount_minor"] = amount_minor.into();
            assert_eq!(
                serde_json::from_value::<WorkflowOperation>(encoded).unwrap(),
                operation
            );
        }
    }

    #[test]
    fn declined_proposal_is_terminal_and_retains_its_audit() {
        let mut state = AutomationState::default();
        let id = state.create_workflow("Decline", focus_workflow(), 1);
        state.preview(&id, 2).expect("preview");
        assert!(state.decline_workflow(&id, 3));
        assert_eq!(state.workflows[0].status, WorkflowStatus::Declined);
        assert!(state.workflows[0].previewed_operations.is_none());
        assert!(state.jobs.is_empty());
        assert!(state
            .audit
            .iter()
            .any(|entry| entry.kind == AuditKind::Declined));
        assert!(state.preview(&id, 4).is_none());
        assert!(state.start_job(&id, 5).is_none());
        assert!(!state.decline_workflow(&id, 6));
        let restored: AutomationState =
            serde_json::from_str(&serde_json::to_string(&state).expect("encode")).expect("decode");
        assert_eq!(restored.workflows[0].status, WorkflowStatus::Declined);
    }

    #[test]
    fn started_workflow_cannot_be_declined() {
        let mut state = AutomationState::default();
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let id = state.create_workflow(
            "Start",
            vec![WorkflowOperation::CreateTask {
                title: "Keep".into(),
            }],
            1,
        );
        state.preview(&id, 2).expect("preview");
        state.start_job(&id, 3).expect("job");
        assert!(!state.decline_workflow(&id, 4));
        assert_eq!(state.workflows[0].status, WorkflowStatus::Running);
    }

    fn focus_workflow() -> Vec<WorkflowOperation> {
        vec![
            WorkflowOperation::CreateTask {
                title: "Write launch brief".into(),
            },
            WorkflowOperation::CreateCalendarEvent {
                title: "Launch focus".into(),
                start_at_ms: 1_000.0,
                end_at_ms: 2_000.0,
            },
            WorkflowOperation::AddCapture {
                content: "Protected focus plan".into(),
            },
        ]
    }

    #[test]
    fn tool_registry_covers_read_and_write_for_every_domain() {
        let tools = agent_tool_definitions();
        assert_eq!(tools.len(), AgentDomain::ALL.len() * 2);
        for domain in AgentDomain::ALL {
            assert!(tools
                .iter()
                .any(|tool| tool.domain == domain && tool.access == AgentAccess::Read));
            assert!(tools
                .iter()
                .any(|tool| tool.domain == domain && tool.access == AgentAccess::Write));
        }
    }

    #[test]
    fn dry_run_blocks_writes_until_every_domain_is_granted() {
        let mut state = AutomationState::default();
        let id = state.create_workflow("Focus", focus_workflow(), 1);
        let preview = state.preview(&id, 2).expect("workflow");
        assert!(!preview.allowed);
        assert_eq!(preview.blocked_domains.len(), 3);
        for domain in [
            AgentDomain::Tasks,
            AgentDomain::Calendar,
            AgentDomain::Capture,
        ] {
            state.permissions.set(domain, AgentAccess::Write, true);
        }
        assert!(state.preview(&id, 3).expect("workflow").allowed);
    }

    #[test]
    fn background_job_resumes_and_rollback_reverts_in_reverse_safe_state() {
        let mut state = AutomationState::default();
        for domain in [
            AgentDomain::Tasks,
            AgentDomain::Calendar,
            AgentDomain::Capture,
        ] {
            state.permissions.set(domain, AgentAccess::Write, true);
        }
        let workflow_id = state.create_workflow("Focus", focus_workflow(), 1);
        assert!(state.preview(&workflow_id, 2).expect("workflow").allowed);
        let job_id = state.start_job(&workflow_id, 3).expect("job");
        assert!(state.complete_step(&job_id, "task-1", 4));
        assert!(state.pause_job(&job_id, 5));
        assert!(state.resume_job(&job_id, 6));
        assert!(state.complete_step(&job_id, "event-1", 7));
        assert!(state.complete_step(&job_id, "capture-1", 8));
        assert!(state.mark_rolled_back(&workflow_id, 9));
        let workflow = state
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .expect("workflow");
        assert_eq!(workflow.status, WorkflowStatus::RolledBack);
        assert!(workflow
            .steps
            .iter()
            .all(|step| step.status == WorkflowStepStatus::Reverted));
    }

    #[test]
    fn suggestions_are_dismissible_and_rate_limited() {
        let mut state = AutomationState::default();
        state.suggestions.push(ProactiveSuggestion {
            id: "focus".into(),
            title: "Protect focus".into(),
            explanation: "Two overdue tasks share a deadline.".into(),
            evidence: "2 overdue tasks".into(),
            domain: AgentDomain::Tasks,
            operations: focus_workflow(),
            dismissed: false,
            shown_count: 0,
            last_shown_at_ms: None,
        });
        assert!(state.mark_suggestion_surfaced("focus", 100));
        assert!(state.surfaceable_suggestions(101).is_empty());
        assert_eq!(
            state
                .surfaceable_suggestions(100 + SUGGESTION_COOLDOWN_MS + 1)
                .len(),
            1
        );
        assert!(state.dismiss_suggestion("focus", 200));
        assert!(state
            .surfaceable_suggestions(100 + 2 * SUGGESTION_COOLDOWN_MS)
            .is_empty());
    }
}
