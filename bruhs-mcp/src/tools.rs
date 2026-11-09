//! The tool registry: one table drives `tools/list` and dispatch in both
//! modes, so a tool is never advertised in a mode that cannot dispatch it.
//!
//! The standalone mode serves this process's own in-memory state; the bridge
//! mode forwards to the running native app (see `workspace_bridge`). Tools
//! that only the app can answer are not listed, and are unknown, in
//! standalone mode, and the other way around.
use crate::args::{self, ReadPage, ToolId, WriteRequest};
use crate::protocol::{ToolInfo, ToolResult};
use bruhs_core::AppState;
use bruhs_models::{
    agent_tool_definitions, AgentAccess, AgentDomain, AutomationState, WorkflowOperation,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
mod life_schema;
mod workflow_schema;

/// Which backend answers `tools/call`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(unix),
    expect(dead_code, reason = "the workspace bridge is Unix-only")
)]
pub enum Mode {
    /// This process's own in-memory state (no `BRUHS_WORKSPACE_BRIDGE`).
    Standalone,
    /// The running native app, over its private socket.
    Bridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Standalone,
    Bridge,
    Both,
}

impl Availability {
    fn includes(self, mode: Mode) -> bool {
        matches!(
            (self, mode),
            (Self::Both, _) | (Self::Standalone, Mode::Standalone) | (Self::Bridge, Mode::Bridge)
        )
    }
}

/// Every tool this server can dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    ReadDomain(AgentDomain),
    WriteDomain(AgentDomain),
    ReadMeetingNotes,
    ReadMeetingNote,
    ReadMeeting,
    ReadPerson,
    ReadChatMessages,
    ProposeWorkflow,
    WorkflowStatus,
    RequestWorkflowRollback,
    ListThreads,
    ListTasks,
    AgentAudit,
}

struct ToolSpec {
    tool: Tool,
    availability: Availability,
    info: ToolInfo,
}

fn registry() -> &'static [ToolSpec] {
    static REGISTRY: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    REGISTRY.get_or_init(build_registry)
}

/// The tools `tools/list` advertises in `mode`.
pub fn list_tools(mode: Mode) -> Vec<ToolInfo> {
    registry()
        .iter()
        .filter(|spec| spec.availability.includes(mode))
        .map(|spec| spec.info.clone())
        .collect()
}

/// The tool named `name`, if `mode` can dispatch it.
pub fn lookup(mode: Mode, name: &str) -> Option<Tool> {
    registry()
        .iter()
        .find(|spec| spec.info.name == name && spec.availability.includes(mode))
        .map(|spec| spec.tool)
}

fn spec(tool: Tool, availability: Availability, info: ToolInfo) -> ToolSpec {
    ToolSpec {
        tool,
        availability,
        info,
    }
}

fn info(name: &str, description: &str, input_schema: Value) -> ToolInfo {
    ToolInfo {
        name: name.into(),
        description: description.into(),
        input_schema,
    }
}

fn build_registry() -> Vec<ToolSpec> {
    use Availability::{Both, Bridge, Standalone};
    let mut specs = vec![
        spec(Tool::ProposeWorkflow, Bridge, workflow_schema::tool()),
        spec(Tool::ReadMeetingNotes, Bridge, info(
            "bruhs_read_meeting_notes",
            "List local text meeting-note identities, separate from recordings. Requires Meetings Read independently of Write. Filters ID/title only, at most 50 per page within 512 KiB; follow next_offset. Use bruhs_read_meeting_note for the exact body. Live workspace data, not a save receipt. No provider request or mutation.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "query":{"type":"string","maxLength":512,"default":""},
                "offset":{"type":"integer","minimum":0,"maximum":100000,"default":0}
            }}),
        )),
        spec(Tool::ReadMeetingNote, Bridge, info(
            "bruhs_read_meeting_note",
            "Read an exact local text meeting note by ID, including its body. Requires current Meetings Read independently of Write. Separate from recorded meetings and generated summaries. Live workspace data, not a save receipt. No provider request or mutation.",
            json!({"type":"object","additionalProperties":false,"required":["note_id"],"properties":{
                "note_id":{"type":"string","minLength":1,"maxLength":128}
            }}),
        )),
        spec(Tool::ReadMeeting, Bridge, info(
            "bruhs_read_meeting",
            "Read one loaded native meeting by exact ID from bruhs_read_meetings. Requires current Meetings Read independently of Write. summary returns metadata, speakers and optional generated summary (offset 0); transcript pages up to 50 cached segments within 512 KiB, including explicitly marked partial segments. Follow next_offset. Local audio paths are never returned. Live workspace cache, not a frozen export or durable receipt; missing transcript cache returns an error, not an empty transcript. No provider request or mutation. Open the meeting in the app to load uncached content.",
            json!({"type":"object","additionalProperties":false,"required":["meeting_id"],"properties":{
                "meeting_id":{"type":"string","minLength":1,"maxLength":128},
                "section":{"type":"string","enum":["summary","transcript"],"default":"summary"},
                "offset":{"type":"integer","minimum":0,"maximum":100000,"default":0}
            }}),
        )),
        spec(Tool::ReadPerson, Bridge, info(
            "bruhs_read_person",
            "Read native People context by exact ID from bruhs_read_people. Requires current People Read independently of Write. section=profile returns identity, aliases and relationship metadata/notes (offset 0 only); observations and relationship_items return paginated full records, at most 50 within 512 KiB. Follow next_offset, which can advance by less than 50. Voice fingerprints are never exposed. Live local state, not a frozen export or save receipt. No provider request or mutation. Oversized individual records fail explicitly.",
            json!({"type":"object","additionalProperties":false,"required":["person_id"],"properties":{
                "person_id":{"type":"string","minLength":1,"maxLength":128},
                "section":{"type":"string","enum":["profile","observations","relationship_items"],"default":"profile"},
                "offset":{"type":"integer","minimum":0,"maximum":100000,"default":0,"description":"Profile requires 0. History sections accept next_offset from their previous page."}
            }}),
        )),
        spec(Tool::ReadChatMessages, Bridge, info(
            "bruhs_read_chat_messages",
            "Read a page of completed local messages from one native workspace thread. Requires current Chat Read, independently of Write. Use IDs from bruhs_read_chat. Excludes streaming messages, unsent input and stashed drafts. Returns at most 50 records with next_offset; pages are live, not frozen snapshots or save receipts. No provider request or mutation. Oversized individual records return an explicit error.",
            json!({"type":"object","additionalProperties":false,"required":["thread_id"],"properties":{
                "thread_id":{"type":"string","minLength":1,"maxLength":128},
                "offset":{"type":"integer","minimum":0,"maximum":100000,"default":0}
            }}),
        )),
        spec(Tool::WorkflowStatus, Bridge, info(
            "bruhs_workflow_status",
            "Check a proposal and ordered step progress from this workspace connection session. Counts are provisional unless durable=true; durable counts are null until the app save is acknowledged. Completion is never inferred from submission.",
            json!({"type":"object","additionalProperties":false,"required":["workflow_id"],"properties":{
                "workflow_id":{"type":"string","minLength":1,"maxLength":128}
            }}),
        )),
        spec(Tool::RequestWorkflowRollback, Bridge, info(
            "bruhs_request_workflow_rollback",
            "Preview rollback eligibility for a workflow proposed in this native connection session. With dry_run=false, request user review in the app; never delete records or grant approval. Current Write grants, a saved checkpoint, and unchanged created records are required. The user must pause any running background job first. Repeats do not override a declined request. Use bruhs_workflow_status for durable rollback progress.",
            json!({"type":"object","additionalProperties":false,"required":["workflow_id"],"properties":{
                "workflow_id":{"type":"string","minLength":1,"maxLength":128},
                "dry_run":{"type":"boolean","default":true}
            }}),
        )),
        spec(Tool::ListThreads, Standalone, info(
            "bruhs_list_threads",
            "List all chat threads",
            json!({ "type": "object", "properties": {} }),
        )),
        spec(Tool::ListTasks, Standalone, info(
            "bruhs_list_tasks",
            "List background tasks for a thread",
            json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string", "minLength": 1, "maxLength": 128, "description": "The thread ID to list tasks for" }
                },
                "required": ["thread_id"]
            }),
        )),
        spec(Tool::AgentAudit, Standalone, info(
            "bruhs_agent_audit",
            "Read the human-readable cross-domain workflow audit timeline",
            json!({ "type": "object", "properties": {} }),
        )),
    ];
    specs.extend(agent_tool_definitions().into_iter().map(|definition| {
        let (tool, availability) = match definition.access {
            // Only Chat has a store in this process; the other domains' data
            // lives in the app.
            AgentAccess::Read if definition.domain == AgentDomain::Chat => {
                (Tool::ReadDomain(definition.domain), Both)
            }
            AgentAccess::Read => (Tool::ReadDomain(definition.domain), Bridge),
            AgentAccess::Write => (Tool::WriteDomain(definition.domain), Both),
        };
        let input_schema = match definition.access {
            AgentAccess::Read if matches!(definition.domain, AgentDomain::Finance | AgentDomain::Health) => life_schema::read_schema(definition.domain),
            AgentAccess::Read if definition.domain == AgentDomain::People => json!({
                "type":"object", "additionalProperties":false,
                "properties": {
                    "query":{"type":"string","maxLength":512,"description":"Filter ID, display name or aliases, never private notes/history"},
                    "offset":{"type":"integer","minimum":0,"maximum":100000,"default":0}
                }
            }),
            AgentAccess::Read => json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "maxLength": 512, "description": "Optional local filter" },
                    "offset": { "type": "integer", "minimum": 0, "maximum": 100000, "description": "Pagination offset returned by the connected workspace" }
                }
            }),
            AgentAccess::Write => json!({
                "type": "object",
                "properties": {
                    "operation": workflow_schema::operation_schema(definition.domain).unwrap_or_else(|| json!({
                        "type": "object",
                        "description": "A tagged WorkflowOperation whose domain matches this tool"
                    })),
                    "dry_run": {
                        "type": "boolean",
                        "default": true,
                        "description": "Preview without applying. With a connected workspace, false submits for in-app user review; it does not execute. Requires a Write grant and request_id."
                    },
                    "request_id": { "type": "string", "minLength": 1, "maxLength": 128, "description": "Stable unique identifier for a submission. Reuse only with exactly the same operation when retrying." }
                },
                "required": ["operation"]
            }),
        };
        let description = if definition.domain == AgentDomain::Chat && definition.access == AgentAccess::Read {
            "Read paginated native Chat thread summaries, not message bodies. Requires current Chat Read. query filters ID, title and project name; offset pages matching threads. Use bruhs_read_chat_messages with a returned thread ID for completed history. Live workspace state, not a save receipt; no mutation or provider request.".into()
        } else if definition.domain == AgentDomain::Meetings && definition.access == AgentAccess::Read {
            "Read paginated native meeting identity summaries from the current workspace cache with Meetings Read. query filters ID/title only, never transcript text, summaries or local audio paths. Use bruhs_read_meeting with a returned ID for loaded summary/transcript context. At most 50 records within 512 KiB; follow next_offset. Not a durable receipt. No provider request or mutation.".into()
        } else if definition.domain == AgentDomain::People && definition.access == AgentAccess::Read {
            "Read paginated native People identity summaries by current People Read grant. query filters ID, display name and aliases. Summaries include tracked status, timestamps and alias/observation counts, not histories, relationship notes or voice fingerprints. Use bruhs_read_person with an exact returned ID for context. At most 50 records within 512 KiB; follow next_offset. Live local state, no mutation or provider request.".into()
        } else if definition.access == AgentAccess::Read && matches!(definition.domain, AgentDomain::Finance | AgentDomain::Health) {
            life_schema::read_description(definition.domain).into()
        } else {
            definition.description
        };
        spec(
            tool,
            availability,
            ToolInfo {
                name: definition.name,
                description,
                input_schema,
            },
        )
    }));
    specs
}

fn automation_state() -> &'static Mutex<AutomationState> {
    static STATE: OnceLock<Mutex<AutomationState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(AutomationState::default()))
}

fn granted(domain: AgentDomain, access: AgentAccess) -> bool {
    automation_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .permissions
        .allows(domain, access)
}

/// Answers a standalone-mode tool call. Argument problems and execution
/// failures are both tool results with `isError: true`.
pub async fn call_standalone(tool: Tool, args: &Value, state: &Arc<AppState>) -> ToolResult {
    match tool {
        Tool::ListThreads => {
            with_grant(AgentDomain::Chat, AgentAccess::Read, || list_threads(state)).await
        }
        Tool::ListTasks => {
            let args: ListTasksArgs = match args::parse("bruhs_list_tasks", args) {
                Ok(args) => args,
                Err(error) => return ToolResult::err(error.to_string()),
            };
            with_grant(AgentDomain::Chat, AgentAccess::Read, || async {
                list_tasks(&args, state)
            })
            .await
        }
        Tool::ReadDomain(domain) => read_domain(domain, args, state).await,
        Tool::WriteDomain(domain) => write_domain(domain, args),
        Tool::AgentAudit => agent_audit(),
        // The registry never offers these in standalone mode; answer rather
        // than panic if a future edit lets one through.
        Tool::ReadMeetingNotes
        | Tool::ReadMeetingNote
        | Tool::ReadMeeting
        | Tool::ReadPerson
        | Tool::ReadChatMessages
        | Tool::ProposeWorkflow
        | Tool::WorkflowStatus
        | Tool::RequestWorkflowRollback => {
            ToolResult::err("This tool requires the native workspace connection.")
        }
    }
}

async fn with_grant<F, Fut>(domain: AgentDomain, access: AgentAccess, run: F) -> ToolResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ToolResult>,
{
    if granted(domain, access) {
        run().await
    } else {
        ToolResult::err(format!(
            "{} {access:?} access is not granted",
            domain.label()
        ))
    }
}

async fn read_domain(domain: AgentDomain, args: &Value, state: &Arc<AppState>) -> ToolResult {
    // Same bounds as the app-backed readers, though this process ignores them.
    if let Err(error) = args::parse::<ReadPage>("bruhs_read", args) {
        return ToolResult::err(error.to_string());
    }
    with_grant(domain, AgentAccess::Read, || async {
        if domain != AgentDomain::Chat {
            return ToolResult::err(format!(
                "{} storage is not attached to this MCP process; no records were read.",
                domain.label()
            ));
        }
        let threads = state.get_threads().await;
        match serde_json::to_string_pretty(&threads) {
            Ok(text) => ToolResult::ok(text),
            Err(error) => ToolResult::err(format!("Cannot encode threads: {error}")),
        }
    })
    .await
}

fn write_domain(domain: AgentDomain, args: &Value) -> ToolResult {
    let tool = format!("bruhs_write_{}", domain.key());
    let request = match WriteRequest::parse(&tool, domain, args) {
        Ok(request) => request,
        Err(error) => return ToolResult::err(error.to_string()),
    };
    if !request.dry_run {
        return ToolResult::err("No durable domain-write adapter is attached to this MCP process. Nothing was changed. Use the app workspace; MCP execution is unavailable until a trusted workspace bridge is connected.");
    }
    preview(domain, request.operation)
}

fn preview(domain: AgentDomain, operation: WorkflowOperation) -> ToolResult {
    let now_ms = bruhs_models::EpochMs::now().as_millis();
    let mut state = automation_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let workflow_id = state.create_workflow(
        format!("MCP {} write", domain.label()),
        vec![operation],
        now_ms,
    );
    let Some(preview) = state.preview(&workflow_id, now_ms) else {
        return ToolResult::err("Workflow could not be previewed");
    };
    drop(state);
    ToolResult::ok(
        json!({
            "dry_run": true,
            "workflow_id": workflow_id,
            "allowed": preview.allowed,
            "changes": preview.summaries,
            "blocked_domains": preview.blocked_domains.iter().map(|domain| domain.key()).collect::<Vec<_>>()
        })
        .to_string(),
    )
}

fn agent_audit() -> ToolResult {
    let state = automation_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let encoded = serde_json::to_string_pretty(&json!({
        "permissions": state.permissions,
        "workflows": state.workflows,
        "jobs": state.jobs,
        "audit": state.audit,
    }));
    drop(state);
    match encoded {
        Ok(text) => ToolResult::ok(text),
        Err(error) => ToolResult::err(format!("Cannot encode the audit timeline: {error}")),
    }
}

async fn list_threads(state: &Arc<AppState>) -> ToolResult {
    let items: Vec<Value> = state
        .get_threads()
        .await
        .iter()
        .map(|thread| json!({ "id": thread.id, "title": thread.title }))
        .collect();
    json_result(&items)
}

#[derive(Debug, Deserialize)]
struct ListTasksArgs {
    thread_id: ToolId,
}

fn list_tasks(args: &ListTasksArgs, state: &Arc<AppState>) -> ToolResult {
    let items: Vec<Value> = state
        .registry()
        .get_by_thread(args.thread_id.as_str())
        .iter()
        .map(|task| {
            json!({
                "id": task.id,
                "label": task.label,
                "status": format!("{:?}", task.status),
                "started_at": task.started_at,
                "ended_at": task.ended_at,
                "exit_code": task.exit_code,
            })
        })
        .collect();
    json_result(&items)
}

fn json_result(value: &impl serde::Serialize) -> ToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(text) => ToolResult::ok(text),
        Err(error) => ToolResult::err(format!("Cannot encode the result: {error}")),
    }
}

#[cfg(test)]
mod tests;
