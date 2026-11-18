//! Answers each request from the live workspace with the shared logic in
//! `bruhs_models::workspace_bridge`, which the web answers with too.
use super::*;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, WorkflowProposal};
use bruhs_models::workspace_bridge::{self as bridge, Proposed};
use bruhs_models::AgentDomain;
use serde_json::Value;

mod chat_read;
mod meeting_read;
mod people_read;
mod rollback;

impl Workspace {
    pub(super) fn handle_bridge_request(
        &mut self,
        request: BridgeRequest,
        cx: &mut Context<'_, Self>,
    ) -> BridgeResponse {
        self.bridge_request(request, cx).into()
    }

    fn bridge_request(
        &mut self,
        request: BridgeRequest,
        cx: &mut Context<'_, Self>,
    ) -> Result<Value, String> {
        let permissions = &self.automation_state.permissions;
        match request {
            BridgeRequest::ReadFinance {
                section,
                query,
                offset,
            } => bridge::read_finance(permissions, &self.life_context, section, &query, offset),
            BridgeRequest::ReadHealth {
                section,
                query,
                offset,
            } => bridge::read_health(permissions, &self.life_context, section, &query, offset),
            BridgeRequest::ReadMeetingNotes { query, offset } => {
                bridge::read_meeting_notes(permissions, &self.meeting_notes, &query, offset)
            }
            BridgeRequest::ReadMeetingNote { note_id } => {
                bridge::read_meeting_note(permissions, &self.meeting_notes, &note_id)
            }
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
                bridge::require_read(permissions, domain)?;
                match domain {
                    AgentDomain::Chat => self.read_bridge_chat(&query, offset, cx),
                    AgentDomain::Meetings => self.read_bridge_meetings(&query, offset, cx),
                    AgentDomain::People => self.read_bridge_people(&query, offset, cx),
                    AgentDomain::Finance => bridge::read_finance(
                        permissions,
                        &self.life_context,
                        Default::default(),
                        &query,
                        offset,
                    ),
                    AgentDomain::Health => bridge::read_health(
                        permissions,
                        &self.life_context,
                        Default::default(),
                        &query,
                        offset,
                    ),
                    AgentDomain::Tasks => bridge::read_records(&self.work_items, &query, offset),
                    AgentDomain::Calendar => {
                        bridge::read_records(&self.calendar_events, &query, offset)
                    }
                    AgentDomain::Capture => bridge::read_records(&self.captures, &query, offset),
                    AgentDomain::Knowledge => {
                        bridge::read_records(&self.knowledge_documents, &query, offset)
                    }
                    AgentDomain::Inbox => {
                        bridge::read_records(&self.inbox_conversations, &query, offset)
                    }
                }
            }
            BridgeRequest::Preview { operation } => {
                bridge::preview_operation(permissions, &operation)
            }
            BridgeRequest::Propose {
                request_id,
                operation,
            } => self.propose_bridge_workflow(request_id, WorkflowProposal::single(operation), cx),
            BridgeRequest::PreviewWorkflow { proposal } => {
                bridge::preview_workflow(permissions, &proposal)
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

    pub(super) fn bridge_workflow_status(&self, id: &str) -> Result<Value, String> {
        self.mcp_bridge
            .reviews
            .status(&self.automation_state, self.save_status.is_saved(), id)
    }

    fn propose_bridge_workflow(
        &mut self,
        request_id: String,
        proposal: WorkflowProposal,
        cx: &mut Context<'_, Self>,
    ) -> Result<Value, String> {
        let saved = self.save_status.is_saved();
        let id = match self.mcp_bridge.reviews.propose(
            &mut self.automation_state,
            saved,
            request_id,
            proposal,
            EpochMs::now().as_millis(),
        )? {
            Proposed::Existing(id) => id,
            // As on the web: the proposal gets its own review card, and the
            // history stays where the user left it.
            Proposed::Created(id) => {
                self.persist_and_notify(cx);
                id
            }
        };
        self.bridge_workflow_status(&id)
    }
}
