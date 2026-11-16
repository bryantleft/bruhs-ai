//! What one connection session has asked the user to review: its proposals
//! (by `request_id`) and its undo requests. This state is ephemeral and never
//! saved; disabling or restarting the connection clears it, without touching
//! records or replaying anything.
use super::{
    validate_operation, validate_proposal, WorkflowProposal, MAX_ID_BYTES, PROPOSAL_LIMIT,
};
use crate::{
    AgentAccess, AgentDomain, AgentPermissionPolicy, AutomationState, WorkflowOperation,
    WorkflowStatus, WorkflowStepStatus,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Where a session's undo request stands. `Approved` follows the user's undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackReview {
    Pending,
    Declined,
    Approved,
}

/// The outcome of [`BridgeReviews::propose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposed {
    /// A retry of an earlier identical submission; nothing changed.
    Existing(String),
    /// A new previewed workflow awaiting review. The app saves it and shows it.
    Created(String),
}

/// An undo request that passed its ownership, grant and save checks. The app
/// computes the eligible targets in between (see [`BridgeReviews::begin_rollback_request`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollbackRequest {
    already_undone: bool,
    declined: bool,
}

impl RollbackRequest {
    /// Whether the app must compute the rollback targets: not for a workflow
    /// already undone or a request the user declined.
    pub fn needs_targets(self) -> bool {
        !self.already_undone && !self.declined
    }
}

/// One connection session's proposals and undo requests.
#[derive(Debug, Clone, Default)]
pub struct BridgeReviews {
    /// `request_id` → (workflow ID, the reviewed proposal).
    pub proposals: HashMap<String, (String, WorkflowProposal)>,
    pub rollback_reviews: HashMap<String, RollbackReview>,
}

/// `bruhs_write_<domain>` with `dry_run` (the default): the operation and
/// whether its Write grant currently allows it. Nothing is saved.
pub fn preview_operation(
    policy: &AgentPermissionPolicy,
    operation: &WorkflowOperation,
) -> Result<Value, String> {
    validate_operation(operation)?;
    Ok(
        json!({"dry_run":true,"operation":operation,"allowed":policy.allows(operation.domain(), AgentAccess::Write),"requires_user_approval":true,"executed":false}),
    )
}

/// `bruhs_propose_workflow` with `dry_run`: the proposal and the domains
/// whose Write grant blocks it. Nothing is saved.
pub fn preview_workflow(
    policy: &AgentPermissionPolicy,
    proposal: &WorkflowProposal,
) -> Result<Value, String> {
    validate_proposal(proposal)?;
    let blocked_domains: Vec<_> = AgentDomain::ALL
        .into_iter()
        .filter(|domain| {
            proposal
                .operations
                .iter()
                .any(|operation| operation.domain() == *domain)
                && !policy.allows(*domain, AgentAccess::Write)
        })
        .collect();
    Ok(
        json!({"dry_run":true,"proposal":proposal,"allowed":blocked_domains.is_empty(),"blocked_domains":blocked_domains,"requires_user_approval":true,"executed":false}),
    )
}

impl BridgeReviews {
    pub fn pending_review(&self, state: &AutomationState) -> bool {
        self.pending_review_id(state).is_some()
    }

    /// The workflow whose review the session is waiting on: an undo request
    /// first, then a proposal still in preview.
    pub fn pending_review_id<'a>(&'a self, state: &AutomationState) -> Option<&'a str> {
        if let Some((id, _)) = self.rollback_reviews.iter().find(|(id, review)| {
            **review == RollbackReview::Pending
                && state.workflows.iter().any(|workflow| {
                    workflow.id == **id && workflow.status != WorkflowStatus::RolledBack
                })
        }) {
            return Some(id.as_str());
        }
        self.pending_proposal_id(state)
    }

    /// A proposal waiting for its preview to be reviewed.
    pub fn pending_proposal_id<'a>(&'a self, state: &AutomationState) -> Option<&'a str> {
        // Zero or one owner already needs at most one history pass.
        if self.proposals.len() <= 1 {
            return self
                .proposals
                .values()
                .find(|(id, _)| {
                    state.workflows.iter().any(|workflow| {
                        workflow.id == *id && workflow.status == WorkflowStatus::Previewed
                    })
                })
                .map(|(id, _)| id.as_str());
        }
        // Inactive history needs one pass, not one per session proposal. Keep
        // the existing proposal iteration priority when several previews exist.
        let mut first: Option<(usize, &str)> = None;
        for workflow in state
            .workflows
            .iter()
            .filter(|workflow| workflow.status == WorkflowStatus::Previewed)
        {
            for (rank, (id, _)) in self.proposals.values().enumerate() {
                if first.is_some_and(|(best, _)| rank >= best) {
                    break;
                }
                if workflow.id == *id {
                    if rank == 0 {
                        return Some(id);
                    }
                    first = Some((rank, id.as_str()));
                    break;
                }
            }
        }
        first.map(|(_, id)| id)
    }

    /// The user's undo of `id` was saved.
    pub fn finish_rollback_review(&mut self, id: &str) {
        if let Some(review) = self.rollback_reviews.get_mut(id) {
            *review = RollbackReview::Approved;
        }
    }

    pub fn rollback_pending(&self, id: &str) -> bool {
        self.rollback_reviews.get(id) == Some(&RollbackReview::Pending)
    }

    /// Keep changes: decline a pending undo request. Repeats do not reopen it.
    pub fn decline_rollback(&mut self, id: &str) -> bool {
        if !self.rollback_pending(id) {
            return false;
        }
        self.rollback_reviews
            .insert(id.to_owned(), RollbackReview::Declined);
        true
    }

    /// Admit a submission for review: every operation valid, every domain's
    /// Write grant current, a durable save (`saved`), and nothing else waiting
    /// on the user. A repeated `request_id` must carry the identical title and
    /// ordered operations.
    pub fn propose(
        &mut self,
        state: &mut AutomationState,
        saved: bool,
        request_id: String,
        proposal: WorkflowProposal,
        now_ms: i64,
    ) -> Result<Proposed, String> {
        validate_proposal(&proposal)?;
        if proposal.operations.iter().any(|operation| {
            !state
                .permissions
                .allows(operation.domain(), AgentAccess::Write)
        }) {
            return Err(
                "Write access is not granted for this domain. Review permissions in the app."
                    .into(),
            );
        }
        if request_id.is_empty() || request_id.len() > MAX_ID_BYTES {
            return Err("A request_id of 1 to 128 bytes is required.".into());
        }
        if let Some((id, existing)) = self.proposals.get(&request_id) {
            if existing != &proposal {
                return Err(
                    "This request_id was already used for a different title or ordered payload."
                        .into(),
                );
            }
            return Ok(Proposed::Existing(id.clone()));
        }
        if !saved {
            return Err(
                "Resolve the pending local save before submitting another proposal.".into(),
            );
        }
        if self.proposals.len() >= PROPOSAL_LIMIT {
            return Err(
                "Session proposal limit reached. Review existing workflows before reconnecting."
                    .into(),
            );
        }
        if self.pending_review_id(state).is_some() {
            return Err(
                "Review or decline the existing MCP request first. No records were changed.".into(),
            );
        }
        if state.workflows.iter().any(|workflow| {
            matches!(
                workflow.status,
                WorkflowStatus::Draft
                    | WorkflowStatus::Previewed
                    | WorkflowStatus::Running
                    | WorkflowStatus::Paused
            )
        }) {
            return Err("Review, finish, or decline the existing workflow before submitting another proposal.".into());
        }
        let id = state.create_workflow(proposal.title.clone(), proposal.operations.clone(), now_ms);
        state
            .preview(&id, now_ms)
            .ok_or("The proposal could not be previewed.")?;
        self.proposals.insert(request_id, (id.clone(), proposal));
        Ok(Proposed::Created(id))
    }

    /// `bruhs_workflow_status`: progress of a workflow this session proposed.
    /// `durable` is the app's acknowledged save of the records and checkpoint;
    /// until then counts are provisional and nothing is reported executed.
    pub fn status(
        &self,
        state: &AutomationState,
        durable: bool,
        id: &str,
    ) -> Result<Value, String> {
        let (_, proposal) = self
            .proposals
            .values()
            .find(|(workflow_id, _)| workflow_id == id)
            .ok_or("Unknown proposal for this connection session.")?;
        if proposal.operations.iter().any(|operation| {
            !state
                .permissions
                .allows(operation.domain(), AgentAccess::Write)
        }) {
            return Err("Write access was revoked. Review this workflow in the app.".into());
        }
        let workflow = state
            .workflows
            .iter()
            .find(|workflow| workflow.id == id)
            .ok_or("Workflow no longer exists.")?;
        let completed = workflow
            .steps
            .iter()
            .filter(|step| step.status == WorkflowStepStatus::Completed)
            .count();
        let reverted = workflow
            .steps
            .iter()
            .filter(|step| step.status == WorkflowStepStatus::Reverted)
            .count();
        let job = state.jobs.iter().find(|job| job.workflow_id == id);
        // Progress is provisional until the same save which contains the
        // records and checkpoint is acknowledged. Never serialize full target
        // snapshots or unrelated workflows into an agent response.
        Ok(json!({
            "workflow_id": id,
            "workflow_status": workflow.status,
            "durable": durable,
            "requires_user_approval": workflow.status == WorkflowStatus::Previewed || (self.rollback_pending(id) && workflow.status != WorkflowStatus::RolledBack),
            "executed": durable && workflow.status == WorkflowStatus::Completed,
            "rolled_back": durable && workflow.status == WorkflowStatus::RolledBack,
            "rollback_review": self.rollback_reviews.get(id),
            "progress": {
                "total_steps": workflow.steps.len(),
                "completed_steps": completed,
                "reverted_steps": reverted,
                "durable_completed_steps": durable.then_some(completed),
                "durable_reverted_steps": durable.then_some(reverted),
            },
            "job": job.map(|job| json!({
                "status": job.status,
                "next_step": job.next_step,
                "background_enabled": job.auto_run,
                "can_advance": durable && state.pending_operation(&job.id).is_some(),
            })),
        }))
    }

    /// First half of `bruhs_request_workflow_rollback`: the status boundary
    /// enforces session ownership and every current Write grant before any
    /// target state is inspected or revealed, then a durable save is required.
    pub fn begin_rollback_request(
        &self,
        state: &AutomationState,
        saved: bool,
        id: &str,
    ) -> Result<RollbackRequest, String> {
        let status = self.status(state, saved, id)?;
        if !saved {
            return Err("Wait for a durable local save before requesting undo.".into());
        }
        Ok(RollbackRequest {
            already_undone: status["rolled_back"] == true,
            declined: self.rollback_reviews.get(id) == Some(&RollbackReview::Declined),
        })
    }

    /// Second half: with the eligible `completed_steps` the app found (zero
    /// when [`RollbackRequest::needs_targets`] was false), a non-dry run opens
    /// an undo review. Returns the response and whether a review was opened.
    /// It never deletes a record or approves anything.
    pub fn finish_rollback_request(
        &mut self,
        state: &AutomationState,
        request: RollbackRequest,
        id: &str,
        dry_run: bool,
        completed_steps: usize,
    ) -> Result<(Value, bool), String> {
        let RollbackRequest {
            already_undone,
            declined,
        } = request;
        let mut opened = false;
        if !dry_run && !already_undone && !declined && !self.rollback_pending(id) {
            if self.pending_review_id(state).is_some() {
                return Err(
                    "Review or decline the existing MCP request first. No records were changed."
                        .into(),
                );
            }
            self.rollback_reviews
                .insert(id.into(), RollbackReview::Pending);
            opened = true;
        }
        Ok((
            json!({
                "workflow_id":id,
                "dry_run":dry_run,
                "eligible":completed_steps != 0,
                "completed_steps":completed_steps,
                "rollback_review":self.rollback_reviews.get(id),
                "requires_user_approval":!already_undone && !declined,
                "rolled_back":already_undone,
                "checkpoint_durable":true,
            }),
            opened,
        ))
    }
}
