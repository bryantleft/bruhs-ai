//! Translates tool calls into requests for the running native app.
use crate::args::{self, ArgumentError, ReadPage, StrictReadPage, ToolId, WriteRequest};
use crate::protocol::ToolResult;
use crate::tools::Tool;
use bruhs_core::workspace_bridge::{
    BridgeClient, BridgeRequest, BridgeResponse, MeetingReadSection, PersonReadSection,
    WorkflowProposal,
};
use bruhs_models::{AgentDomain, WorkflowOperation};
use serde::Deserialize;
use serde_json::Value;
mod life_read;

/// Forwards one tool call to the app. Every failure (bad arguments, a
/// disconnected app, an app-side refusal) is a tool result with
/// `isError: true`; nothing is reported as done unless the app said so.
pub async fn call(tool: Tool, arguments: &Value, client: &BridgeClient) -> ToolResult {
    let request = match translate(tool, arguments) {
        Ok(request) => request,
        Err(error) => return ToolResult::err(error.to_string()),
    };
    match client.call(request).await {
        Ok(BridgeResponse::Ok(value)) => ToolResult::ok(value.to_string()),
        Ok(BridgeResponse::Error(message)) => ToolResult::err(message),
        Err(_) => ToolResult::err("Workspace disconnected, timed out, or rejected the response. Submission status may be unknown; reuse the same request_id when retrying a proposal. No completion is claimed."),
    }
}

fn translate(tool: Tool, args: &Value) -> Result<BridgeRequest, ArgumentError> {
    match tool {
        Tool::ReadDomain(domain) => translate_read(domain, args),
        Tool::WriteDomain(domain) => translate_write(domain, args),
        Tool::ReadMeetingNotes => {
            let page: StrictReadPage = args::parse("bruhs_read_meeting_notes", args)?;
            Ok(BridgeRequest::ReadMeetingNotes {
                query: page.query.into_string(),
                offset: page.offset.get(),
            })
        }
        Tool::ReadMeetingNote => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                note_id: ToolId,
            }
            let args: Arguments = args::parse("bruhs_read_meeting_note", args)?;
            Ok(BridgeRequest::ReadMeetingNote {
                note_id: args.note_id.into_string(),
            })
        }
        Tool::ReadMeeting => translate_meeting(args),
        Tool::ReadPerson => translate_person(args),
        Tool::ReadChatMessages => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                thread_id: ToolId,
                #[serde(default)]
                offset: args::Offset,
            }
            let args: Arguments = args::parse("bruhs_read_chat_messages", args)?;
            Ok(BridgeRequest::ReadChatMessages {
                thread_id: args.thread_id.into_string(),
                offset: args.offset.get(),
            })
        }
        Tool::ProposeWorkflow => translate_workflow(args),
        Tool::WorkflowStatus => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                workflow_id: ToolId,
            }
            let args: Arguments = args::parse("bruhs_workflow_status", args)?;
            Ok(BridgeRequest::Status {
                workflow_id: args.workflow_id.into_string(),
            })
        }
        Tool::RequestWorkflowRollback => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                workflow_id: ToolId,
                #[serde(default = "args::preview_by_default")]
                dry_run: bool,
            }
            let args: Arguments = args::parse("bruhs_request_workflow_rollback", args)?;
            Ok(BridgeRequest::RequestRollback {
                workflow_id: args.workflow_id.into_string(),
                dry_run: args.dry_run,
            })
        }
        Tool::ListThreads | Tool::ListTasks | Tool::AgentAudit => Err(ArgumentError::NotBridged),
    }
}

fn translate_read(domain: AgentDomain, args: &Value) -> Result<BridgeRequest, ArgumentError> {
    let tool = format!("bruhs_read_{}", domain.key());
    match domain {
        AgentDomain::Finance | AgentDomain::Health => life_read::translate(&tool, domain, args),
        AgentDomain::People => {
            let page: StrictReadPage = args::parse(&tool, args)?;
            Ok(BridgeRequest::Read {
                domain,
                query: page.query.into_string(),
                offset: page.offset.get(),
            })
        }
        _ => {
            let page: ReadPage = args::parse(&tool, args)?;
            Ok(BridgeRequest::Read {
                domain,
                query: page.query.into_string(),
                offset: page.offset.get(),
            })
        }
    }
}

fn translate_write(domain: AgentDomain, args: &Value) -> Result<BridgeRequest, ArgumentError> {
    let tool = format!("bruhs_write_{}", domain.key());
    let request = WriteRequest::parse(&tool, domain, args)?;
    if request.dry_run {
        return Ok(BridgeRequest::Preview {
            operation: request.operation,
        });
    }
    Ok(BridgeRequest::Propose {
        request_id: request
            .request_id
            .ok_or(ArgumentError::MissingRequestId)?
            .into_string(),
        operation: request.operation,
    })
}

fn translate_meeting(args: &Value) -> Result<BridgeRequest, ArgumentError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Arguments {
        meeting_id: ToolId,
        #[serde(default)]
        section: MeetingReadSection,
        #[serde(default)]
        offset: args::Offset,
    }
    let args: Arguments = args::parse("bruhs_read_meeting", args)?;
    if args.section == MeetingReadSection::Summary && args.offset.get() != 0 {
        return Err(ArgumentError::Rule("summary requires offset 0."));
    }
    Ok(BridgeRequest::ReadMeeting {
        meeting_id: args.meeting_id.into_string(),
        section: args.section,
        offset: args.offset.get(),
    })
}

fn translate_person(args: &Value) -> Result<BridgeRequest, ArgumentError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Arguments {
        person_id: ToolId,
        #[serde(default)]
        section: PersonReadSection,
        #[serde(default)]
        offset: args::Offset,
    }
    let args: Arguments = args::parse("bruhs_read_person", args)?;
    if args.section == PersonReadSection::Profile && args.offset.get() != 0 {
        return Err(ArgumentError::Rule("profile requires offset 0."));
    }
    Ok(BridgeRequest::ReadPerson {
        person_id: args.person_id.into_string(),
        section: args.section,
        offset: args.offset.get(),
    })
}

fn translate_workflow(args: &Value) -> Result<BridgeRequest, ArgumentError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Arguments {
        title: String,
        operations: Vec<WorkflowOperation>,
        #[serde(default = "args::preview_by_default")]
        dry_run: bool,
        #[serde(default)]
        request_id: Option<ToolId>,
    }
    let args: Arguments = args::parse("bruhs_propose_workflow", args)?;
    let proposal = WorkflowProposal {
        title: args.title,
        operations: args.operations,
    };
    proposal.validate()?;
    if args.dry_run {
        return Ok(BridgeRequest::PreviewWorkflow { proposal });
    }
    Ok(BridgeRequest::ProposeWorkflow {
        request_id: args
            .request_id
            .ok_or(ArgumentError::MissingRequestId)?
            .into_string(),
        proposal,
    })
}

#[cfg(test)]
mod tests;
