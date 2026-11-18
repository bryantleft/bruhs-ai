//! Turn-scoped MCP tools for the local workbench. Permission creation stays in the UI.
use crate::agent::ProviderQueryOptions;
use serde_json::{json, Value};
use std::{collections::HashMap, io, sync::OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const FLAG: &str = "--workbench-mcp";
const SERVER: &str = "bruhs_workbench";
/// Set on a bruh's helper: the agent id whose own browser it drives. The
/// helper then offers only that browser (see [`BruhScope`]).
pub(crate) const BRUH_ENV: &str = "BRUHS_WORKBENCH_BRUH";
const ENDPOINT_ENV: &str = "BRUHS_WORKBENCH_URL";
static ENDPOINT: OnceLock<String> = OnceLock::new();
pub fn set_endpoint(url: String) -> io::Result<()> {
    validate_endpoint(&url)?;
    ENDPOINT
        .set(url)
        .map_err(|_| io::Error::other("Workbench endpoint was already configured."))
}
fn validate_endpoint(url: &str) -> io::Result<()> {
    let parsed = url::Url::parse(url).map_err(io::Error::other)?;
    if parsed.scheme() != "http"
        || !matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(io::Error::other(
            "Agent workbench tools require an HTTP loopback endpoint without credentials or a path.",
        ));
    }
    Ok(())
}
pub fn is_mcp_helper() -> bool {
    std::env::args_os().any(|a| a == FLAG)
}

/// The loopback service the helper calls.
fn endpoint() -> io::Result<String> {
    let endpoint = ENDPOINT
        .get()
        .cloned()
        .or_else(|| std::env::var("BRUHS_SERVER_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:3001".into());
    validate_endpoint(&endpoint)?;
    Ok(endpoint)
}

/// Give a bruh's unattended Claude turn its own browser: the helper as the
/// turn's only MCP server, bound to `agent_id`, with no thread, project or
/// grant scope. `tools` stays as it is, so nothing pre-approves the browser
/// and every call reaches the run's permission handler and gate.
pub(crate) fn configure_bruh(options: &mut ProviderQueryOptions, agent_id: &str) -> io::Result<()> {
    let ProviderQueryOptions::Claude(o) = options else {
        return Err(io::Error::other(bruhs_models::crew::NEEDS_CLAUDE));
    };
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let env = HashMap::from([
        (ENDPOINT_ENV, endpoint()?),
        (BRUH_ENV, agent_id.to_string()),
    ]);
    // Replace, never merge: this is the turn's one server.
    o.mcp_config = Some(
        json!({"mcpServers":{SERVER:{"command":executable,"args":[FLAG],"env":env}}}).to_string(),
    );
    o.strict_mcp_config = true;
    Ok(())
}

pub(crate) fn configure(
    options: &mut ProviderQueryOptions,
    project: &str,
    thread: &str,
    read_only: bool,
    mode: bruhs_models::PermissionMode,
) -> io::Result<()> {
    let endpoint = endpoint()?;
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let env = HashMap::from([
        (ENDPOINT_ENV.into(), endpoint),
        ("BRUHS_WORKBENCH_PROJECT".into(), project.into()),
        ("BRUHS_WORKBENCH_THREAD".into(), thread.into()),
        ("BRUHS_WORKBENCH_MODE".into(), mode.as_str().into()),
        (
            "BRUHS_WORKBENCH_READ_ONLY".into(),
            if read_only { "1" } else { "0" }.into(),
        ),
    ]);
    match options {
        ProviderQueryOptions::Claude(o) => {
            let mut config = o
                .mcp_config
                .as_deref()
                .map(serde_json::from_str::<Value>)
                .transpose()
                .map_err(io::Error::other)?
                .unwrap_or_else(|| json!({}));
            if config.get("mcpServers").is_none() {
                config["mcpServers"] = json!({});
            }
            let servers = config["mcpServers"]
                .as_object_mut()
                .ok_or_else(|| io::Error::other("MCP servers must be an object."))?;
            servers.insert(
                SERVER.into(),
                json!({"command":executable,"args":[FLAG],"env":env}),
            );
            o.mcp_config = Some(config.to_string());
            if let claude_sdk_rs::Tools::Allow(tools) = &mut o.tools {
                for tool in names(read_only) {
                    tools.push(format!("mcp__{SERVER}__{tool}"));
                }
            }
        }
        ProviderQueryOptions::Codex(o) => {
            let vars = env
                .iter()
                .map(|(k, v)| format!("{k} = {}", json!(v)))
                .collect::<Vec<_>>()
                .join(", ");
            let config = format!(
                "{{ command = {}, args = [\"{FLAG}\"], env = {{ {vars} }}, required = true }}",
                json!(executable)
            );
            o.config.push(
                codex_sdk_rs::ConfigOverride::new(format!("mcp_servers.{SERVER}"), config)
                    .map_err(io::Error::other)?,
            );
        }
        ProviderQueryOptions::External(o) => {
            o.add_mcp(SERVER, &executable, &[FLAG.into()], &env)?
        }
    }
    Ok(())
}

fn names(read_only: bool) -> Vec<&'static str> {
    let mut names = vec!["workspace_status", "task_read", "delivery_read"];
    if !read_only {
        names.extend([
            "prepare_delivery",
            "task_work",
            "local_browser",
            "local_device",
        ]);
    }
    names
}
fn input_schema(name: &str) -> Value {
    let string = json!({"type":"string"});
    let number = json!({"type":"number"});
    let forge = json!({"enum":["auto","git_hub","git_lab"],"default":"auto"});
    let isolation = json!({"type":"object","properties":{"kind":{"enum":["shared","worktree"]},"base_branch":string},"required":["kind"]});
    let task = json!({"type":"object","properties":{
        "title":string,"project_path":string,"prompt":string,"model":string,
        "permission_mode":{"enum":["plan","default","acceptEdits","bypassPermissions"]},
        "isolation":isolation,"spec_id":{"type":["string","null"]}
    },"required":["title","project_path","prompt","model","isolation"]});
    let (properties, required) = match name {
        "workspace_status" => (json!({}), vec![]),
        "task_read" => (
            json!({"id":string,"wait":{"type":"boolean","default":false}}),
            vec!["id"],
        ),
        "task_work" => (
            json!({"command":{"type":"object","properties":{
            "action":{"enum":["create","fork","fanout","bring_back","send","cancel"]},
            "id":string,"task":task,"title":string,"prompt":string,"model":string,
            "models":{"type":"array","items":string},"relationship":{"enum":["delegate","fork","side_chat","verify"]},"isolation":isolation
        },"required":["action"]}}),
            vec!["command"],
        ),
        "delivery_read" => (
            json!({"forge":forge,"query":{"type":"object","properties":{"type":{"enum":["pulls","pull","diff","line_comments","issue","tracker_issue","scripts"]},"number":{"type":"integer","minimum":1},"provider":{"enum":["linear","jira"]},"reference":string},"required":["type"]}}),
            vec!["query"],
        ),
        "prepare_delivery" => (
            json!({"forge":forge,"action":{"type":"object","description":"Prepare a typed delivery review. Read current PR head_oid first. Supply only fields for the chosen action.","properties":{"type":{"enum":["create_pull","commit_push_pull","review","comment","request_reviewers","merge","rebase_stack","merge_stack","merge_native_stack","lifecycle","start_issue","start_tracker_issue"]},"title":string,"body":string,"base":string,"head":string,"draft":{"type":"boolean"},"files":{"type":"array","items":string},"message":string,"number":{"type":"integer","minimum":1},"head_oid":string,"event":{"enum":["comment","approve","request_changes"]},"comments":{"type":"array","items":{"type":"object","properties":{"path":string,"line":{"type":"integer","minimum":1},"side":{"enum":["LEFT","RIGHT"]},"body":string},"required":["path","line","side","body"]}},"reviewers":{"type":"array","items":string},"method":{"enum":["merge","squash","rebase"]},"auto":{"type":"boolean"},"phase":{"enum":["setup","run","teardown"]},"branch":string,"run_setup":{"type":"boolean"},"provider":{"enum":["linear","jira"]},"reference":string},"required":["type"]},"timeout_seconds":{"type":"integer","minimum":1,"description":"Owner's execution time budget in seconds; this prepares a review only."}}),
            vec!["action", "timeout_seconds"],
        ),
        "local_browser" => (
            json!({"operation":{"enum":["start","command"]},"url":string,"session_id":string,"command":{"type":"object","properties":{"action":{"enum":["navigate","snapshot","click","click_at","type","key","scroll","fill","evaluate","screenshot","console","network"]},"url":string,"selector":string,"text":string,"expression":string,"x":number,"y":number,"key":string,"dx":number,"dy":number},"required":["action"]}}),
            vec!["operation"],
        ),
        "local_device" => (
            json!({"operation":{"enum":["list","command"]},"device_id":string,"platform":{"enum":["android","ios"]},"command":{"type":"object","properties":{"action":{"enum":["boot","shutdown","screenshot","tap","type","home","back","open_url"]},"x":{"type":"integer","minimum":0},"y":{"type":"integer","minimum":0},"text":string,"url":string},"required":["action"]}}),
            vec!["operation"],
        ),
        _ => (json!({}), vec![]),
    };
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn catalogue(read_only: bool) -> Value {
    let definitions = [
        (
            "workspace_status",
            "Read this project's host-owned task summaries, each with its board lane (building, validating, in_review, ready, done), status, branch and pull requests, and this thread's local-tool grants. To use a browser/device, ask the user to grant access in Workbench → Verification using this thread ID as principal.",
        ),
        (
            "task_read",
            "Read a host-owned task's messages, live output, errors and approvals. arguments.id is a task ID; wait=true waits for its next completion or permission request using host events, up to the existing 60 second request budget. A running result means call again to await more work. Host-owned tasks can read only themselves and descendants in the same project; ordinary chats can read tasks in their project. Never wait on yourself.",
        ),
        (
            "delivery_read",
            "Read GitHub PRs, one PR, its diff/line comments, an issue or project scripts. arguments.query is a typed DeliveryQuery, e.g. {type:'pull',number:1}. The project path is fixed by this turn.",
        ),
        (
            "prepare_delivery",
            "Prepare a delivery action for the owner to review in Workbench → Delivery. arguments.action is a typed DeliveryAction, timeout_seconds is the user's execution budget. This never executes it; only the UI can run the reviewed action.",
        ),
        (
            "task_work",
            "Fork/delegate, fanout, bring_back, send or cancel host-owned work within this project. arguments.command is a typed CoordinationCommand. Cannot approve permissions, accept guest messages, grant access, or delete work. This thread must be a host-owned task for parent/child operations.",
        ),
        (
            "local_browser",
            "Operate the dedicated local browser using an existing grant for this thread. arguments.operation='start' with url, or operation='command' with session_id and command (navigate/snapshot/click/click_at/type/key/scroll/fill/evaluate/screenshot/console/network). click presses the element's centre (via: input) or clicks it by script when covered (via: script); click_at takes x and y in CSS px; type inserts text into selector or the focused element; key presses one named key (Enter, Tab, Escape, Backspace, Delete, Space, arrows, Home, End, PageUp, PageDown); scroll wheels by dx and dy px. Cannot create a grant.",
        ),
        (
            "local_device",
            "Inspect/control installed mobile devices with an existing grant for this thread. arguments.operation='list', or 'command' with device_id, platform, and command. Cannot create a grant.",
        ),
    ];
    json!({"tools":definitions.into_iter().filter(|(name,_)|names(read_only).contains(name)).map(|(name,description)|json!({"name":name,"description":description,"inputSchema":input_schema(name),"annotations":{"readOnlyHint":matches!(name,"workspace_status"|"task_read"|"delivery_read")}})).collect::<Vec<_>>()})
}

/// The model-facing description of a bruh's browser.
const BRUH_BROWSER_DESCRIPTION: &str = "Use your own browser. It keeps its logins between runs and starts on first use. command.action is one of navigate, snapshot, screenshot, click, click_at, type, key, scroll, fill. Take a snapshot to read the page; screenshots are 1280 by 800 CSS pixels, the coordinates click_at uses. If the owner has taken control, your step waits until they hand it back.";

/// What a bruh's helper offers: its own browser, and nothing else. No
/// evaluate, console or network action, no session or grant.
fn bruh_catalogue() -> Value {
    let string = json!({"type":"string"});
    let number = json!({"type":"number"});
    let command = json!({"type":"object","properties":{
        "action":{"type":"string","enum":bruhs_models::local_tools::BRUH_BROWSER_ACTIONS},
        "url":string,"selector":string,"text":string,"x":number,"y":number,
        "key":string,"dx":number,"dy":number
    },"required":["action"],"additionalProperties":false});
    json!({"tools":[{
        "name":"local_browser",
        "description":BRUH_BROWSER_DESCRIPTION,
        "inputSchema":{"type":"object","properties":{"command":command},"required":["command"],"additionalProperties":false},
        "annotations":{"readOnlyHint":false}
    }]})
}

/// A bruh's helper: bound to one agent id, it reaches only that bruh's
/// browser route. The host checks the Computer and the action again.
struct BruhScope {
    endpoint: String,
    agent_id: String,
    client: reqwest::Client,
}
impl BruhScope {
    /// `/api/reactive/agents/{id}/computer/command`, the id escaped as one
    /// path segment.
    fn route(&self) -> io::Result<String> {
        if matches!(self.agent_id.as_str(), "" | "." | "..") {
            return Err(io::Error::other("This bruh has no computer."));
        }
        let mut url = url::Url::parse(&self.endpoint).map_err(io::Error::other)?;
        url.path_segments_mut()
            .map_err(|()| io::Error::other("The workbench endpoint has no path."))?
            .pop_if_empty()
            .extend([
                "api",
                "reactive",
                "agents",
                &self.agent_id,
                "computer",
                "command",
            ]);
        Ok(url.into())
    }
    async fn call(&self, name: &str, args: &Value) -> io::Result<Value> {
        if name != "local_browser" {
            return Err(io::Error::other("Unknown workbench tool."));
        }
        // Only the command: a model-supplied session, grant or page URL
        // never reaches the host.
        let command = args.get("command").cloned().unwrap_or(Value::Null);
        send(
            &self.client,
            &self.route()?,
            Some(json!({ "command": command })),
        )
        .await
    }
}

/// Send one request to the host; a failure carries the host's `error` text.
async fn send(client: &reqwest::Client, url: &str, body: Option<Value>) -> io::Result<Value> {
    let request = match body {
        Some(body) => client.post(url).json(&body),
        None => client.get(url),
    };
    let response = request.send().await.map_err(io::Error::other)?;
    let status = response.status();
    let body: Value = response.json().await.map_err(io::Error::other)?;
    if !status.is_success() {
        return Err(io::Error::other(body["error"].as_str().unwrap_or(
            "Workbench request failed. Open Workbench to inspect the service status.",
        )));
    }
    Ok(body)
}

struct Scope {
    endpoint: String,
    project: String,
    thread: String,
    read_only: bool,
    mode: bruhs_models::PermissionMode,
    client: reqwest::Client,
    wait_client: reqwest::Client,
}
fn descendant(tasks: &[Value], root: &str, id: &str) -> bool {
    let mut current = id;
    let mut visited = std::collections::BTreeSet::new();
    loop {
        if current == root {
            return true;
        }
        if !visited.insert(current) {
            return false;
        }
        let Some(parent) = tasks
            .iter()
            .find(|task| task["id"] == current)
            .and_then(|task| task["parent_id"].as_str())
        else {
            return false;
        };
        current = parent;
    }
}
/// Adds a task's board placement to its summary: lane, status label, branch
/// and compact pull requests. Verification runs and side chats have no card
/// of their own and stay plain.
fn with_board(mut summary: Value, board: &Value) -> Value {
    let Some(card) = board["cards"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|card| card["id"] == summary["id"])
    else {
        return summary;
    };
    summary["lane"] = card["lane"].clone();
    summary["status"] = card["status"]["label"].clone();
    if !card["branch"].is_null() {
        summary["branch"] = card["branch"].clone();
    }
    let pulls: Vec<_> = card["pulls"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|pull| {
            json!({"number":pull["number"],"state":pull["state"],"checks":pull["checks"],
                "review":pull["review_decision"],"mergeable":pull["mergeable"],"url":pull["url"]})
        })
        .collect();
    if !pulls.is_empty() {
        summary["pulls"] = Value::Array(pulls);
    }
    if card["archived"] == true {
        summary["archived"] = Value::Bool(true);
    }
    summary
}
fn task_running(task: &Value) -> bool {
    matches!(
        task["state"].as_str(),
        Some("ready" | "running" | "waiting_child")
    )
}
impl Scope {
    async fn request(&self, path: &str, body: Option<Value>) -> io::Result<Value> {
        self.request_with(&self.client, path, body).await
    }
    async fn request_with(
        &self,
        client: &reqwest::Client,
        path: &str,
        body: Option<Value>,
    ) -> io::Result<Value> {
        let url = format!("{}{path}", self.endpoint.trim_end_matches('/'));
        send(client, &url, body).await
    }
    fn project_matches(&self, path: &str) -> bool {
        std::path::Path::new(path)
            .canonicalize()
            .ok()
            .zip(std::path::Path::new(&self.project).canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
    }
    fn scoped_task(&self, snapshot: &Value, id: &str) -> io::Result<Value> {
        let tasks = snapshot["tasks"]
            .as_array()
            .ok_or_else(|| io::Error::other("The task snapshot is unavailable."))?;
        let task = tasks
            .iter()
            .find(|t| t["id"] == id)
            .ok_or_else(|| io::Error::other("Task not found."))?;
        let current = tasks.iter().find(|t| t["id"] == self.thread);
        let project_ok = if let Some(current) = current {
            let current_scope = current["working_path"]
                .as_str()
                .is_some_and(|p| self.project_matches(p))
                || current["project_path"]
                    .as_str()
                    .is_some_and(|p| self.project_matches(p));
            current_scope
                && task["project_path"] == current["project_path"]
                && descendant(tasks, &self.thread, id)
        } else {
            task["project_path"]
                .as_str()
                .is_some_and(|p| self.project_matches(p))
                || task["working_path"]
                    .as_str()
                    .is_some_and(|p| self.project_matches(p))
        };
        if !project_ok {
            return Err(io::Error::other(
                "Task access is limited to this project's task and its descendants.",
            ));
        }
        Ok(task.clone())
    }
    async fn read_task(&self, args: &Value) -> io::Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("A task id is required."))?;
        let wait = args["wait"].as_bool().unwrap_or(false);
        if wait && id == self.thread {
            return Err(io::Error::other(
                "A task cannot wait for itself. Read your child task instead.",
            ));
        }
        let task = self.scoped_task(&self.request("/api/coordination", None).await?, id)?;
        if !wait || !task_running(&task) {
            return Ok(task);
        }
        // The host parks this parent, observes the child, and restores its
        // worker permit before returning. No client total deadline may release
        // the parent early; a dropped request cancels the parked task tree.
        self.request_with(
            &self.wait_client,
            "/api/coordination/wait",
            Some(json!({"parent_id":self.thread,"id":id})),
        )
        .await
    }
    async fn grant(&self, scope: &str) -> io::Result<Value> {
        let snapshot = self.request("/api/local-tools", None).await?;
        snapshot["grants"].as_array().and_then(|grants|grants.iter().find(|g|g["principal"]==self.thread&&g["scope"]==scope)).cloned().ok_or_else(||io::Error::other(format!("No {scope} grant for thread {}. Ask the user to allow it in Workbench → Verification for this thread; do not call the grant endpoint yourself.",self.thread)))
    }
    async fn call(&self, name: &str, args: &Value) -> io::Result<Value> {
        if !names(self.read_only).contains(&name) {
            return Err(io::Error::other(
                "This tool is unavailable in the current access mode.",
            ));
        }
        match name {
            "workspace_status"=>{
                let tasks=self.request("/api/coordination",None).await?;
                // An older or busy host without a board still answers with plain summaries.
                let board=self.request("/api/coordination/board",None).await.unwrap_or(Value::Null);
                let visible:Vec<_>=tasks["tasks"].as_array().into_iter().flatten().filter(|t|t["id"].as_str().is_some_and(|id|self.scoped_task(&tasks,id).is_ok())).map(|t|with_board(json!({"id":t["id"],"title":t["title"],"state":t["state"],"parent_id":t["parent_id"],"model":t["model"],"error":t["error"]}),&board)).collect();
                let tools=self.request("/api/local-tools",None).await?;
                let grants:Vec<_>=tools["grants"].as_array().into_iter().flatten().filter(|g|g["principal"]==self.thread).cloned().collect();
                Ok(json!({"thread_id":self.thread,"project":self.project,"tasks":visible,"grants":grants}))
            },
            "delivery_read"=>self.request("/api/delivery/read",Some(json!({"path":self.project,"forge":args.get("forge").cloned().unwrap_or_else(||json!("auto")),"query":args["query"]}))).await,
            "task_read"=>self.read_task(args).await,
            "prepare_delivery"=>self.request("/api/delivery/prepare",Some(json!({"path":self.project,"forge":args.get("forge").cloned().unwrap_or_else(||json!("auto")),"action":args["action"],"timeout_seconds":args["timeout_seconds"]}))).await,
            "task_work"=>{
                let mut command=args.get("command").cloned().ok_or_else(||io::Error::other("A typed task command is required."))?;
                let action=command["action"].as_str().unwrap_or("");
                if !matches!(action,"create"|"fork"|"fanout"|"bring_back"|"send"|"cancel"){return Err(io::Error::other("Agents cannot approve, delete, change grants, or accept guest proposals through this tool."));}
                if matches!(action,"create"|"fanout") {
                    let requested:bruhs_models::PermissionMode=serde_json::from_value(command["task"]["permission_mode"].clone()).unwrap_or_default();
                    if !bruhs_models::coordination::narrower(self.mode,requested){return Err(io::Error::other("A delegated task cannot exceed this turn's access mode."));}
                    if !command["task"]["project_path"].as_str().is_some_and(|p|self.project_matches(p)){return Err(io::Error::other("New tasks must use this turn's project path."));}
                    let snapshot=self.request("/api/coordination",None).await?;
                    let parent=self.scoped_task(&snapshot,&self.thread).map_err(|_|io::Error::other("Continue this chat in Workbench before delegating host-owned tasks."))?;
                    command["parent_id"]=json!(self.thread);
                    command["task"]["project_path"]=parent["project_path"].clone();
                }
                if let Some(id)=command["id"].as_str(){
                    let snapshot=self.request("/api/coordination",None).await?;
                    let task=self.scoped_task(&snapshot,id)?;
                    if !snapshot["tasks"].as_array().into_iter().flatten().any(|t|t["id"]==self.thread){return Err(io::Error::other("Start a host-owned task before controlling a task tree."));}
                    let access:bruhs_models::PermissionMode=serde_json::from_value(task["permission_mode"].clone()).map_err(io::Error::other)?;
                    if !bruhs_models::coordination::narrower(self.mode,access){return Err(io::Error::other("Task access is broader than this turn's mode."));}
                }
                let reply=self.request("/api/coordination/commands",Some(command)).await?;
                Ok(json!({"tasks":reply["tasks"].as_array().into_iter().flatten().filter(|t|t["id"].as_str().is_some_and(|id|self.scoped_task(&reply,id).is_ok())).map(|t|json!({"id":t["id"],"title":t["title"],"state":t["state"],"parent_id":t["parent_id"]})).collect::<Vec<_>>()}))
            },
            "local_browser"=>{
                let grant=self.grant("browser").await?;
                match args["operation"].as_str(){
                    Some("start")=>self.request("/api/local-tools/browser/start",Some(json!({"grant_id":grant["id"],"url":args["url"]}))).await,
                    Some("command")=>{
                        let snapshot=self.request("/api/local-tools",None).await?;
                        if !snapshot["browsers"].as_array().into_iter().flatten().any(|s|s["id"]==args["session_id"]&&s["grant_id"]==grant["id"]){return Err(io::Error::other("Browser session is outside this thread's grant."));}
                        self.request("/api/local-tools/browser/command",Some(json!({"grant_id":grant["id"],"session_id":args["session_id"],"command":args["command"]}))).await
                    },
                    _=>Err(io::Error::other("Choose operation start or command.")),
                }
            },
            "local_device"=>{
                let grant=self.grant("devices").await?;
                match args["operation"].as_str(){
                    Some("list")=>self.request(&format!("/api/local-tools/devices?grant_id={}",grant["id"].as_str().unwrap_or("")),None).await,
                    Some("command")=>self.request("/api/local-tools/devices/command",Some(json!({"grant_id":grant["id"],"device_id":args["device_id"],"platform":args["platform"],"command":args["command"]}))).await,
                    _=>Err(io::Error::other("Choose operation list or command.")),
                }
            },
            _=>Err(io::Error::other("Unknown workbench tool.")),
        }
    }
}

/// Who a helper process serves: a thread's turn, or one bruh's browser.
enum Helper {
    Thread(Scope),
    Bruh(BruhScope),
}
impl Helper {
    fn catalogue(&self) -> Value {
        match self {
            Helper::Thread(scope) => catalogue(scope.read_only),
            Helper::Bruh(_) => bruh_catalogue(),
        }
    }
    async fn call(&self, name: &str, args: &Value) -> io::Result<Value> {
        match self {
            Helper::Thread(scope) => scope.call(name, args).await,
            Helper::Bruh(scope) => scope.call(name, args).await,
        }
    }
}

/// Who a helper serves, read from its environment through `get`: one bruh's
/// browser when [`BRUH_ENV`] is set, else a thread's turn. A bruh's helper
/// has no thread scope, so its id is checked first.
fn helper(get: impl Fn(&str) -> Option<String>, client: reqwest::Client) -> io::Result<Helper> {
    let env = |key: &str| {
        get(key).ok_or_else(|| io::Error::other(format!("Missing workbench scope {key}.")))
    };
    let endpoint = env(ENDPOINT_ENV)?;
    validate_endpoint(&endpoint)?;
    if let Some(agent_id) = get(BRUH_ENV) {
        return Ok(Helper::Bruh(BruhScope {
            endpoint,
            agent_id,
            client,
        }));
    }
    Ok(Helper::Thread(Scope {
        endpoint,
        project: env("BRUHS_WORKBENCH_PROJECT")?,
        thread: env("BRUHS_WORKBENCH_THREAD")?,
        mode: bruhs_models::PermissionMode::parse_lossy(&env("BRUHS_WORKBENCH_MODE")?),
        read_only: env("BRUHS_WORKBENCH_READ_ONLY")? != "0",
        wait_client: reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(crate::capture_client::CONNECT_TIMEOUT)
            .build()
            .map_err(io::Error::other)?,
        client,
    }))
}

pub async fn run_mcp_helper() -> io::Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(crate::capture_client::CONNECT_TIMEOUT)
        .timeout(crate::capture_client::REQUEST_TIMEOUT)
        .build()
        .map_err(io::Error::other)?;
    let helper = helper(|key| std::env::var(key).ok(), client)?;
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut writer = tokio::io::stdout();
    loop {
        let mut frame = Vec::new();
        // Match the existing thread/artifact MCP message budget: IDs and commands, never media bytes.
        let n = (&mut reader)
            .take(64 * 1024 + 1)
            .read_until(b'\n', &mut frame)
            .await?;
        if n == 0 {
            return Ok(());
        }
        if n > 64 * 1024 {
            return Err(io::Error::other("Workbench MCP message exceeded 64 KiB."));
        }
        let request: Value = serde_json::from_slice(&frame).map_err(io::Error::other)?;
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion":request["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),"capabilities":{"tools":{}},"serverInfo":{"name":SERVER,"version":env!("CARGO_PKG_VERSION")}})
            }
            Some("tools/list") => helper.catalogue(),
            Some("tools/call") => match helper
                .call(
                    request["params"]["name"].as_str().unwrap_or(""),
                    &request["params"]["arguments"],
                )
                .await
            {
                Ok(mut value) => {
                    let image = value
                        .get_mut("image")
                        .map(Value::take)
                        .filter(|v| !v.is_null());
                    let mut content = vec![
                        json!({"type":"text","text":serde_json::to_string(&value).map_err(io::Error::other)?}),
                    ];
                    if let Some(image) = image {
                        content.push(json!({"type":"image","mimeType":image["mime_type"],"data":image["base64"]}));
                    }
                    json!({"content":content,"isError":false})
                }
                Err(e) => json!({"content":[{"type":"text","text":e.to_string()}],"isError":true}),
            },
            _ => json!({}),
        };
        writer
            .write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":id,"result":result})).as_bytes())
            .await?;
        writer.flush().await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_only_catalogue_cannot_launch_tools_or_task_work() {
        let tools = catalogue(true).to_string();
        assert!(tools.contains("workspace_status"));
        assert!(!tools.contains("local_browser"));
        assert!(!tools.contains("task_work"));
    }
    #[test]
    fn the_thread_browser_offers_real_input_actions() {
        let schema = input_schema("local_browser");
        let command = &schema["properties"]["command"]["properties"];
        for action in ["click_at", "type", "key", "scroll"] {
            assert!(command["action"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(action)));
        }
        for field in ["x", "y", "dx", "dy"] {
            assert_eq!(command[field]["type"], "number", "{field}");
        }
        assert_eq!(command["key"]["type"], "string");
        assert!(catalogue(false)
            .to_string()
            .contains("navigate/snapshot/click/click_at/type/key/scroll/fill"));
    }
    #[test]
    fn forge_and_tracker_tools_prepare_without_an_owner_execution_capability() {
        let tools = catalogue(false).to_string();
        for capability in [
            "git_lab",
            "tracker_issue",
            "start_tracker_issue",
            "merge_native_stack",
        ] {
            assert!(tools.contains(capability));
        }
        for forbidden in [
            "execute-native-stack",
            "acknowledge_server_heads",
            "configure_tracker",
            "token",
        ] {
            assert!(
                !tools.contains(forbidden),
                "Agent catalogue exposed {forbidden}"
            );
        }
    }
    #[test]
    fn task_summaries_carry_their_board_lane_and_pull_requests() {
        let board = json!({"cards":[{"id":"a","lane":"in_review","status":{"label":"Awaiting review","tone":"warning","busy":false},
            "branch":"bruhs/task-a","archived":false,"pulls":[{"number":7,"title":"Fix","url":"https://github.com/o/r/pull/7",
            "state":"open","checks":"passing","review_decision":"REVIEW_REQUIRED","mergeable":"MERGEABLE","comments":2}]}]});
        let summary = with_board(json!({"id":"a","title":"Fix","state":"completed"}), &board);
        assert_eq!(summary["lane"], "in_review");
        assert_eq!(summary["status"], "Awaiting review");
        assert_eq!(summary["branch"], "bruhs/task-a");
        assert_eq!(summary["pulls"][0]["number"], 7);
        assert_eq!(summary["pulls"][0]["review"], "REVIEW_REQUIRED");
        assert!(summary.get("archived").is_none());
        // A folded verification run, or a host without a board, stays plain.
        let plain = json!({"id":"v","title":"Verify","state":"completed"});
        assert_eq!(with_board(plain.clone(), &board), plain);
        assert_eq!(with_board(plain.clone(), &Value::Null), plain);
    }
    #[test]
    fn helper_endpoint_cannot_be_redirected_to_external_hosts() {
        assert!(validate_endpoint("http://127.0.0.1:3001").is_ok());
        for url in [
            "https://example.com",
            "http://127.0.0.1.evil.test",
            "http://user@127.0.0.1:3001",
            "http://127.0.0.1:3001/evil",
        ] {
            assert!(validate_endpoint(url).is_err(), "{url}");
        }
    }
    #[test]
    fn the_bruh_catalogue_offers_only_the_browser() {
        let tools = bruh_catalogue();
        let names: Vec<_> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["local_browser"]);
        let schema = &tools["tools"][0]["inputSchema"];
        let command = &schema["properties"]["command"];
        let action = &command["properties"]["action"];
        assert_eq!(
            action["enum"],
            json!(bruhs_models::local_tools::BRUH_BROWSER_ACTIONS)
        );
        assert_eq!(action["type"], "string");
        assert_eq!(command["additionalProperties"], false);
        assert_eq!(schema["additionalProperties"], false);
        // No session, grant or operation: the helper is bound to one bruh.
        let fields: Vec<_> = schema["properties"].as_object().unwrap().keys().collect();
        assert_eq!(fields, ["command"]);
        let text = tools.to_string();
        for hidden in ["evaluate", "expression", "console", "network", "session_id"] {
            assert!(!text.contains(hidden), "{hidden}");
        }
    }

    fn bruh_scope(endpoint: String) -> BruhScope {
        BruhScope {
            endpoint,
            agent_id: "a".into(),
            client: reqwest::Client::new(),
        }
    }

    #[tokio::test]
    async fn the_bruh_helper_calls_its_own_computer_route() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/reactive/agents/a/computer/command"))
            .and(body_json(json!({"command":{"action":"snapshot"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text":"page"})))
            .expect(1)
            .mount(&server)
            .await;
        let scope = bruh_scope(server.uri());
        // Only `command` is sent, whatever else the model adds.
        let reply = scope
            .call(
                "local_browser",
                &json!({"command":{"action":"snapshot"},"session_id":"other","page_url":"x"}),
            )
            .await
            .unwrap();
        assert_eq!(reply, json!({"text":"page"}));
        for name in ["workspace_status", "task_work", "local_device", ""] {
            let error = scope.call(name, &json!({})).await.unwrap_err();
            assert_eq!(error.to_string(), "Unknown workbench tool.", "{name}");
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn the_bruh_helper_returns_the_route_refusal() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let off = bruhs_models::crew::COMPUTER_OFF_DENIAL;
        Mock::given(method("POST"))
            .and(path("/api/reactive/agents/a/computer/command"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": off})))
            .mount(&server)
            .await;
        let error = bruh_scope(server.uri())
            .call("local_browser", &json!({"command":{"action":"snapshot"}}))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), off);
    }

    /// A helper environment holding exactly `pairs`.
    fn helper_env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| env.get(key).cloned()
    }

    #[test]
    fn the_helper_serves_a_bruh_when_its_id_is_set() {
        let url = (ENDPOINT_ENV, "http://127.0.0.1:3001");
        let thread = [
            url,
            ("BRUHS_WORKBENCH_PROJECT", "/p"),
            ("BRUHS_WORKBENCH_THREAD", "t"),
            ("BRUHS_WORKBENCH_MODE", "default"),
            ("BRUHS_WORKBENCH_READ_ONLY", "1"),
        ];
        // The endpoint and the bruh's id alone are enough.
        let bruh = helper(helper_env(&[url, (BRUH_ENV, "a")]), reqwest::Client::new());
        assert!(matches!(bruh, Ok(Helper::Bruh(scope)) if scope.agent_id == "a"));
        // The bruh's id wins over any thread scope beside it.
        let both = helper(
            helper_env(&[thread.as_slice(), &[(BRUH_ENV, "a")]].concat()),
            reqwest::Client::new(),
        );
        assert!(matches!(both, Ok(Helper::Bruh(_))));
        let turn = helper(helper_env(&thread), reqwest::Client::new());
        assert!(
            matches!(turn, Ok(Helper::Thread(scope)) if scope.read_only && scope.thread == "t")
        );
        // A thread helper still needs its whole scope.
        let Err(missing) = helper(helper_env(&[url]), reqwest::Client::new()) else {
            panic!("a thread helper without its scope starts");
        };
        assert_eq!(
            missing.to_string(),
            "Missing workbench scope BRUHS_WORKBENCH_PROJECT."
        );
        let Err(no_url) = helper(helper_env(&[(BRUH_ENV, "a")]), reqwest::Client::new()) else {
            panic!("a helper without an endpoint starts");
        };
        assert_eq!(
            no_url.to_string(),
            "Missing workbench scope BRUHS_WORKBENCH_URL."
        );
    }

    #[test]
    fn the_bruh_route_escapes_the_agent_id() {
        let scope = BruhScope {
            agent_id: "../x?y".into(),
            ..bruh_scope("http://127.0.0.1:3001".into())
        };
        assert_eq!(
            scope.route().unwrap(),
            "http://127.0.0.1:3001/api/reactive/agents/..%2Fx%3Fy/computer/command"
        );
    }

    #[test]
    fn delegation_scope_follows_lineage_across_worktrees_and_rejects_siblings() {
        let root = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let child = tempfile::tempdir().unwrap();
        let scope = Scope {
            endpoint: "http://127.0.0.1:3001".into(),
            project: worktree.path().to_string_lossy().into(),
            thread: "parent".into(),
            read_only: false,
            mode: bruhs_models::PermissionMode::Default,
            client: reqwest::Client::new(),
            wait_client: reqwest::Client::new(),
        };
        let snapshot = json!({"tasks":[{"id":"parent","project_path":root.path(),"working_path":worktree.path()},{"id":"child","parent_id":"parent","project_path":root.path(),"working_path":child.path()},{"id":"sibling","project_path":root.path(),"working_path":root.path()}]});
        assert!(scope.scoped_task(&snapshot, "child").is_ok());
        assert!(scope.scoped_task(&snapshot, "sibling").is_err());
        assert!(!descendant(
            &[
                json!({"id":"a","parent_id":"b"}),
                json!({"id":"b","parent_id":"a"})
            ],
            "parent",
            "a"
        ));
    }
}
