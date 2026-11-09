use super::*;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};
use serde_json::{json, Value};

const RECORD_LIMIT: usize = 50;
const PROPOSAL_LIMIT: usize = 100;
mod chat_read;
mod finance_read;
mod health_read;
mod life_read;
mod meeting_note_read;
mod meeting_read;
mod people_read;
mod read_page;
mod rollback;
mod status;

fn supported(domain: AgentDomain) -> bool {
    matches!(
        domain,
        AgentDomain::Chat
            | AgentDomain::Meetings
            | AgentDomain::Tasks
            | AgentDomain::Calendar
            | AgentDomain::Capture
            | AgentDomain::Knowledge
            | AgentDomain::Inbox
            | AgentDomain::Finance
            | AgentDomain::Health
            | AgentDomain::People
    )
}

fn read_records<T: Serialize>(records: &[T], query: &str, offset: usize) -> Result<Value, String> {
    if query.len() > 512 || offset > 100_000 {
        return Err("Query or offset is too large.".into());
    }
    let query = query.to_lowercase();
    let mut matching = Vec::new();
    let mut skipped = 0;
    for record in records {
        // Unfiltered paging must not serialize all earlier pages on the UI thread.
        if query.is_empty() && skipped < offset {
            skipped += 1;
            continue;
        }
        let value =
            serde_json::to_value(record).map_err(|_| "A workspace record could not be encoded.")?;
        if !query.is_empty() && !value.to_string().to_lowercase().contains(&query) {
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        matching.push(value);
        if matching.len() > RECORD_LIMIT {
            break;
        }
    }
    let more = matching.len() > RECORD_LIMIT;
    matching.truncate(RECORD_LIMIT);
    let result = json!({"records":matching,"next_offset":more.then_some(offset + RECORD_LIMIT)});
    if result.to_string().len() > 512 * 1024 {
        return Err("Records are too large for one response. Use a more specific query.".into());
    }
    Ok(result)
}

impl Workspace {
    pub(super) fn handle_bridge_request(
        &mut self,
        request: BridgeRequest,
        cx: &mut Context<'_, Self>,
    ) -> BridgeResponse {
        match self.bridge_request(request, cx) {
            Ok(value) => BridgeResponse::Ok(value),
            Err(message) => BridgeResponse::Error(message),
        }
    }

    fn bridge_request(
        &mut self,
        request: BridgeRequest,
        cx: &mut Context<'_, Self>,
    ) -> Result<Value, String> {
        match request {
            BridgeRequest::ReadFinance {
                section,
                query,
                offset,
            } => self.read_bridge_finance(section, &query, offset),
            BridgeRequest::ReadHealth {
                section,
                query,
                offset,
            } => self.read_bridge_health(section, &query, offset),
            BridgeRequest::ReadMeetingNotes { query, offset } => {
                self.read_bridge_meeting_notes(&query, offset)
            }
            BridgeRequest::ReadMeetingNote { note_id } => self.read_bridge_meeting_note(&note_id),
            BridgeRequest::ReadMeeting {
                meeting_id,
                section,
                offset,
            } => self.read_bridge_meeting(&meeting_id, section, offset, cx),
            BridgeRequest::ReadPerson {
                person_id,
                section,
                offset,
            } => self.read_bridge_person(&person_id, section, offset, cx),
            BridgeRequest::ReadChatMessages { thread_id, offset } => {
                self.read_bridge_chat_messages(&thread_id, offset, cx)
            }
            BridgeRequest::Read {
                domain,
                query,
                offset,
            } => {
                if !self
                    .automation_state
                    .permissions
                    .allows(domain, AgentAccess::Read)
                {
                    return Err("Read access is not granted for this domain.".into());
                }
                match domain {
                    AgentDomain::Chat => self.read_bridge_chat(&query, offset, cx),
                    AgentDomain::Meetings => self.read_bridge_meetings(&query, offset, cx),
                    AgentDomain::People => self.read_bridge_people(&query, offset, cx),
                    AgentDomain::Finance => {
                        self.read_bridge_finance(Default::default(), &query, offset)
                    }
                    AgentDomain::Health => {
                        self.read_bridge_health(Default::default(), &query, offset)
                    }
                    AgentDomain::Tasks => read_records(&self.work_items, &query, offset),
                    AgentDomain::Calendar => read_records(&self.calendar_events, &query, offset),
                    AgentDomain::Capture => read_records(&self.captures, &query, offset),
                    AgentDomain::Knowledge => {
                        read_records(&self.knowledge_documents, &query, offset)
                    }
                    AgentDomain::Inbox => read_records(&self.inbox_conversations, &query, offset),
                }
            }
            BridgeRequest::Preview { operation } => {
                Self::validate_bridge_operation(&operation)?;
                Ok(
                    json!({"dry_run":true,"operation":operation,"allowed":self.automation_state.permissions.allows(operation.domain(), AgentAccess::Write),"requires_user_approval":true,"executed":false}),
                )
            }
            BridgeRequest::Propose {
                request_id,
                operation,
            } => self.propose_bridge_workflow(request_id, WorkflowProposal::single(operation), cx),
            BridgeRequest::PreviewWorkflow { proposal } => {
                Self::validate_bridge_proposal(&proposal)?;
                let blocked_domains: Vec<_> = AgentDomain::ALL
                    .into_iter()
                    .filter(|domain| {
                        proposal
                            .operations
                            .iter()
                            .any(|operation| operation.domain() == *domain)
                            && !self
                                .automation_state
                                .permissions
                                .allows(*domain, AgentAccess::Write)
                    })
                    .collect();
                Ok(
                    json!({"dry_run":true,"proposal":proposal,"allowed":blocked_domains.is_empty(),"blocked_domains":blocked_domains,"requires_user_approval":true,"executed":false}),
                )
            }
            BridgeRequest::ProposeWorkflow {
                request_id,
                proposal,
            } => self.propose_bridge_workflow(request_id, proposal, cx),
            BridgeRequest::Status { workflow_id } => self.bridge_workflow_status(&workflow_id),
            BridgeRequest::RequestRollback {
                workflow_id,
                dry_run,
            } => self.request_bridge_rollback(&workflow_id, dry_run, cx),
        }
    }

    fn validate_bridge_operation(operation: &WorkflowOperation) -> Result<(), String> {
        if !supported(operation.domain()) {
            return Err("This write adapter is not connected. Nothing was changed.".into());
        }
        if !operation.is_valid() {
            return Err("Invalid operation payload. Nothing was changed.".into());
        }
        Ok(())
    }

    fn validate_bridge_proposal(proposal: &WorkflowProposal) -> Result<(), String> {
        proposal.validate().map_err(|e| e.to_string())?;
        if proposal
            .operations
            .iter()
            .any(|operation| !supported(operation.domain()))
        {
            return Err(
                "This workflow includes an unconnected write adapter. Nothing was changed.".into(),
            );
        }
        Ok(())
    }

    fn propose_bridge_workflow(
        &mut self,
        request_id: String,
        proposal: WorkflowProposal,
        cx: &mut Context<'_, Self>,
    ) -> Result<Value, String> {
        Self::validate_bridge_proposal(&proposal)?;
        if proposal.operations.iter().any(|operation| {
            !self
                .automation_state
                .permissions
                .allows(operation.domain(), AgentAccess::Write)
        }) {
            return Err(
                "Write access is not granted for this domain. Review permissions in the app."
                    .into(),
            );
        }
        if request_id.is_empty() || request_id.len() > 128 {
            return Err("A request_id of 1–128 bytes is required.".into());
        }
        if let Some((id, existing)) = self.mcp_bridge.proposals.get(&request_id) {
            if existing != &proposal {
                return Err(
                    "This request_id was already used for a different title or ordered payload."
                        .into(),
                );
            }
            return self.bridge_workflow_status(id);
        }
        if !self.save_status.is_saved() {
            return Err(
                "Resolve the pending local save before submitting another proposal.".into(),
            );
        }
        if self.mcp_bridge.proposals.len() >= PROPOSAL_LIMIT {
            return Err(
                "Session proposal limit reached. Review existing workflows before reconnecting."
                    .into(),
            );
        }
        if self
            .mcp_bridge
            .pending_review_id(&self.automation_state)
            .is_some()
        {
            return Err(
                "Review or decline the existing MCP request first. No records were changed.".into(),
            );
        }
        if self.automation_state.workflows.iter().any(|workflow| {
            matches!(
                workflow.status,
                bruhs_models::WorkflowStatus::Draft
                    | bruhs_models::WorkflowStatus::Previewed
                    | bruhs_models::WorkflowStatus::Running
                    | bruhs_models::WorkflowStatus::Paused
            )
        }) {
            return Err("Review, finish, or decline the existing workflow before submitting another proposal.".into());
        }
        let id = self.automation_state.create_workflow(
            proposal.title.clone(),
            proposal.operations.clone(),
            EpochMs::now().as_millis(),
        );
        self.automation_state
            .preview(&id, EpochMs::now().as_millis())
            .ok_or("The proposal could not be previewed.")?;
        self.mcp_bridge
            .proposals
            .insert(request_id, (id.clone(), proposal));
        self.reviewed_workflow_id = Some(id.clone());
        self.persist_and_notify(cx);
        self.bridge_workflow_status(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::read_records;
    use serde_json::json;

    #[test]
    fn pages_are_bounded_and_offsets_apply_to_filtered_records() {
        let records: Vec<_> = (0..123)
            .map(|id| json!({"id":id,"title":if id % 2 == 0 {"Needle"} else {"other"}}))
            .collect();
        let page = read_records(&records, "", 50).expect("page");
        assert_eq!(page["records"].as_array().expect("records").len(), 50);
        assert_eq!(page["records"][0]["id"], 50);
        assert_eq!(page["next_offset"], 100);
        let filtered = read_records(&records, "NEEDLE", 50).expect("filtered page");
        assert_eq!(filtered["records"].as_array().expect("records").len(), 12);
        assert_eq!(filtered["records"][0]["id"], 100);
        assert!(filtered["next_offset"].is_null());
        assert!(read_records(&records, &"x".repeat(513), 0).is_err());
        assert!(read_records(&records, "", 100_001).is_err());
    }
}
