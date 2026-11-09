//! Thread-to-thread requests for agent turns: list the app's threads, read
//! one, send one a message.
//!
//! Each turn gets a private MCP server, `bruhs_threads` (the host executable
//! in `--thread-mcp` mode). Its tool calls reach a per-turn broker on a
//! loopback port, guarded by a token that dies with the turn. The broker
//! answers nothing itself: it streams a [`ThreadRequest`] on the calling
//! turn and waits for the app that owns the threads (web or desktop) to
//! answer through [`ThreadLinks::resolve`]. See
//! [`bruhs_models::thread_link`] for what the app does with one.
//!
//! A `message_thread` call may wait for the recipient's reply where the app
//! runs threads side by side ([`ThreadLinks::with_waits`]). The app then
//! answers when that reply exists, up to [`WAIT`] later.

use crate::agent::ProviderQueryOptions;
use crate::{OrchestratorEvent, QueryEvent};
use bruhs_models::{
    ids::ThreadId,
    thread_link::{ThreadOperation, ThreadReply, ThreadRequest},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
};

const HELPER_FLAG: &str = "--thread-mcp";
const BROKER_ENV: &str = "BRUHS_THREAD_BROKER";
const SERVER: &str = "bruhs_threads";
/// A reply holds at most a clipped transcript; a request at most one message.
const MAX_FRAME_BYTES: usize = 256 * 1024;
/// How long the helper has to say what it wants after it connects.
const HANDSHAKE: Duration = Duration::from_secs(15);
/// How long the app has to answer. Every answer is local and immediate, so
/// a miss means no app is listening to this turn.
const ANSWER: Duration = Duration::from_secs(60);
/// How long a call may wait for another thread's reply.
const WAIT: Duration = Duration::from_secs(60 * 60);
/// The tools that change nothing; a turn never has to ask before using them.
const READ_TOOLS: [&str; 2] = ["list_threads", "read_thread"];
const WRITE_TOOL: &str = "message_thread";

/// Why an answer was not delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ThreadLinkError {
    #[error("This thread request is no longer pending in this thread.")]
    NotPending,
}

struct Pending {
    thread_id: ThreadId,
    scope: String,
    reply: oneshot::Sender<ThreadReply>,
}

/// Every pending thread request of this process.
#[derive(Clone, Default)]
pub struct ThreadLinks {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    /// The app can run the recipient while the sender's call is held.
    waits: bool,
}

impl ThreadLinks {
    /// Offer `message_thread`'s `wait`. Only an app that runs threads side
    /// by side can honor it.
    #[must_use]
    pub fn with_waits(mut self, waits: bool) -> Self {
        self.waits = waits;
        self
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Answer the request `id` raised by a turn of `thread_id`. Only the app
    /// UI calls this; neither MCP nor a provider tool can.
    ///
    /// # Errors
    ///
    /// [`ThreadLinkError::NotPending`] when no such request is waiting: it
    /// was answered already, its turn ended, or it belongs to another thread.
    pub fn resolve(
        &self,
        thread_id: &ThreadId,
        id: &str,
        reply: ThreadReply,
    ) -> Result<(), ThreadLinkError> {
        let item = {
            let mut pending = self.lock();
            if pending
                .get(id)
                .is_some_and(|item| &item.thread_id == thread_id)
            {
                pending.remove(id)
            } else {
                None
            }
        }
        .ok_or(ThreadLinkError::NotPending)?;
        item.reply
            .send(reply)
            .map_err(|_| ThreadLinkError::NotPending)
    }

    pub(crate) fn turn(
        &self,
        thread_id: ThreadId,
        tx: &mpsc::Sender<OrchestratorEvent>,
    ) -> Arc<ThreadScope> {
        let (cancel, _) = watch::channel(false);
        Arc::new(ThreadScope {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id,
            active: AtomicBool::new(true),
            cancel,
            tx: tx.downgrade(),
            links: self.clone(),
        })
    }
}

/// One turn's right to ask about threads. Revoked when the turn ends.
pub(crate) struct ThreadScope {
    id: String,
    thread_id: ThreadId,
    active: AtomicBool,
    cancel: watch::Sender<bool>,
    tx: mpsc::WeakSender<OrchestratorEvent>,
    links: ThreadLinks,
}

/// Revokes its scope when dropped.
pub(crate) struct ThreadScopeGuard(Arc<ThreadScope>);
impl Drop for ThreadScopeGuard {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

/// The turn's broker. Dropping it stops the listener and every call in it.
pub(crate) struct ThreadBroker {
    connection: Connection,
    task: JoinHandle<()>,
}
impl Drop for ThreadBroker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    port: u16,
    token: String,
    /// The turn may not write (a Codex turn that cannot ask first), so the
    /// helper offers only the read tools.
    read_only: bool,
    /// `message_thread` may wait for the reply.
    #[serde(default)]
    can_wait: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerRequest {
    token: String,
    operation: ThreadOperation,
}

struct PendingGuard {
    links: ThreadLinks,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.links.lock().remove(&self.id);
    }
}

impl ThreadScope {
    pub(crate) fn guard(self: &Arc<Self>) -> ThreadScopeGuard {
        ThreadScopeGuard(self.clone())
    }

    fn revoke(&self) {
        self.active.store(false, Ordering::Release);
        self.cancel.send_replace(true);
        // Dropping a reply sender ends its call as "the turn ended".
        self.links.lock().retain(|_, item| item.scope != self.id);
    }

    /// Start this turn's broker. `read_only` refuses `message_thread`.
    pub(crate) async fn start(self: &Arc<Self>, read_only: bool) -> std::io::Result<ThreadBroker> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let connection = Connection {
            port: listener.local_addr()?.port(),
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            read_only,
            can_wait: self.links.waits && !read_only,
        };
        let token = connection.token.clone();
        let scope = self.clone();
        let mut cancelled = self.cancel.subscribe();
        let task = tokio::spawn(async move {
            // A model may list, read and send in one batch, so calls overlap.
            // Dropping the set (this task ending or aborted) stops them all.
            let mut calls = JoinSet::new();
            while scope.active.load(Ordering::Acquire) {
                tokio::select! {
                    _ = cancelled.changed() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let (scope, token) = (scope.clone(), token.clone());
                        calls.spawn(async move {
                            // A helper that went away has nobody to tell.
                            let _ = scope.serve_connection(stream, &token, read_only).await;
                        });
                    }
                    Some(_) = calls.join_next() => {}
                }
            }
        });
        Ok(ThreadBroker { connection, task })
    }

    async fn serve_connection(
        self: &Arc<Self>,
        mut stream: TcpStream,
        token: &str,
        read_only: bool,
    ) -> std::io::Result<()> {
        let frame = tokio::time::timeout(HANDSHAKE, read_frame(&mut stream)).await??;
        let reply = match serde_json::from_slice::<BrokerRequest>(&frame) {
            Ok(request) if same_token(&request.token, token) => {
                if read_only && request.operation.writes() {
                    ThreadReply::refused(
                        "Not sent: this turn is read-only. The user can allow edits to let it message other threads.",
                    )
                } else {
                    let mut probe = [0u8; 1];
                    tokio::select! {
                        reply = self.request(request.operation) => reply,
                        // The helper hung up: its tool call was cancelled.
                        _ = stream.read(&mut probe) => return Ok(()),
                    }
                }
            }
            _ => ThreadReply::refused("Unauthorized."),
        };
        write_frame(&mut stream, &reply).await
    }

    /// Stream `operation` to the app and wait for its answer.
    async fn request(self: &Arc<Self>, operation: ThreadOperation) -> ThreadReply {
        const ENDED: &str = "The turn ended before the app answered.";
        if let Err(reason) = operation.validate() {
            return ThreadReply::refused(reason);
        }
        // A waiting call is answered when the recipient has replied.
        let waits = operation.waits() && self.links.waits;
        let limit = if waits { WAIT } else { ANSWER };
        let request = ThreadRequest {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id: self.thread_id.clone(),
            operation,
        };
        let (reply, receive) = oneshot::channel();
        {
            let mut pending = self.links.lock();
            // Checked under the lock `revoke` clears, so a revoked turn
            // leaves nothing behind.
            if !self.active.load(Ordering::Acquire) {
                return ThreadReply::refused(ENDED);
            }
            pending.insert(
                request.id.clone(),
                Pending {
                    thread_id: self.thread_id.clone(),
                    scope: self.id.clone(),
                    reply,
                },
            );
        }
        let _pending = PendingGuard {
            links: self.links.clone(),
            id: request.id.clone(),
        };
        let Some(tx) = self.tx.upgrade() else {
            return ThreadReply::refused(ENDED);
        };
        if tx
            .send(OrchestratorEvent::Query(QueryEvent::ThreadRequest(request)))
            .await
            .is_err()
        {
            return ThreadReply::refused(ENDED);
        }
        drop(tx);
        match tokio::time::timeout(limit, receive).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => ThreadReply::refused(ENDED),
            Err(_) if waits => ThreadReply::refused(
                "No reply after an hour. The message was delivered; use read_thread later to see the answer.",
            ),
            Err(_) => ThreadReply::refused(
                "The app did not answer. Its window may be closed; tell the user instead of retrying.",
            ),
        }
    }
}

impl ThreadBroker {
    /// Give the turn the `bruhs_threads` server. The read tools never ask
    /// first. `message_thread` follows the turn's permission mode on Claude;
    /// Codex cannot ask, so it gets the tool approved or (read-only) not at
    /// all.
    pub(crate) fn configure(&self, options: &mut ProviderQueryOptions) -> std::io::Result<()> {
        let executable = std::env::current_exe()?.to_string_lossy().into_owned();
        let connection = serde_json::to_string(&self.connection).map_err(std::io::Error::other)?;
        match options {
            ProviderQueryOptions::Claude(options) => {
                // Joins the servers already configured for the turn.
                let mut config: Value = match options.mcp_config.as_deref() {
                    Some(config) => serde_json::from_str(config).map_err(std::io::Error::other)?,
                    None => json!({}),
                };
                let servers = config
                    .as_object_mut()
                    .map(|config| config.entry("mcpServers").or_insert_with(|| json!({})))
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| std::io::Error::other("MCP configuration is not an object."))?;
                servers.insert(
                    SERVER.into(),
                    json!({"command": executable, "args": [HELPER_FLAG], "env": {BROKER_ENV: connection}}),
                );
                options.mcp_config = Some(config.to_string());
                for tool in READ_TOOLS {
                    let tool = format!("mcp__{SERVER}__{tool}");
                    match &mut options.tools {
                        claude_sdk_rs::Tools::CliDefault => {
                            options.tools = claude_sdk_rs::Tools::Allow(vec![tool]);
                        }
                        claude_sdk_rs::Tools::Allow(tools) if !tools.contains(&tool) => {
                            tools.push(tool);
                        }
                        _ => {}
                    }
                }
            }
            ProviderQueryOptions::Codex(options) => {
                let executable = json!(executable).to_string();
                let connection = json!(connection).to_string();
                let tools: Vec<String> = READ_TOOLS
                    .iter()
                    .chain((!self.connection.read_only).then_some(&WRITE_TOOL))
                    .map(|tool| format!("{tool} = {{ approval_mode = \"approve\" }}"))
                    .collect();
                // A waiting call must outlive Codex's own tool deadline.
                let timeout = if self.connection.can_wait {
                    WAIT.as_secs() + ANSWER.as_secs()
                } else {
                    ANSWER.as_secs() * 2
                };
                // Required, so the tools are in the catalog Codex builds at startup.
                let config = format!(
                    "{{ command = {executable}, args = [\"{HELPER_FLAG}\"], env = {{ {BROKER_ENV} = {connection} }}, tool_timeout_sec = {timeout}, required = true, tools = {{ {} }} }}",
                    tools.join(", ")
                );
                options.config.push(
                    codex_sdk_rs::ConfigOverride::new(format!("mcp_servers.{SERVER}"), config)
                        .map_err(std::io::Error::other)?,
                );
            }
        }
        Ok(())
    }
}

fn same_token(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let length = usize::try_from(stream.read_u32().await?).map_err(std::io::Error::other)?;
    if length > MAX_FRAME_BYTES {
        return Err(std::io::Error::other("Thread request frame is too large."));
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn write_frame(stream: &mut TcpStream, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::other("Thread request frame is too large."));
    }
    stream
        .write_u32(u32::try_from(bytes.len()).map_err(std::io::Error::other)?)
        .await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

// ---------------------------------------------------------------------------
// The MCP helper: the host executable run as `--thread-mcp`.
// ---------------------------------------------------------------------------

pub fn is_mcp_helper() -> bool {
    std::env::args_os().any(|arg| arg == HELPER_FLAG)
}

pub async fn run_mcp_helper() -> std::io::Result<()> {
    let connection: Connection = std::env::var(BROKER_ENV)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| std::io::Error::other("Thread broker is unavailable."))?;
    serve(tokio::io::stdin(), &mut tokio::io::stdout(), &connection).await
}

/// Serve MCP over newline-delimited JSON-RPC until `input` ends. Tool calls
/// run side by side; everything else is answered in order.
async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: R,
    output: &mut W,
    connection: &Connection,
) -> std::io::Result<()> {
    let (responses, mut outbox) = mpsc::channel::<Value>(32);
    let read = async {
        let mut reader = BufReader::new(input);
        // Aborted when this future is dropped, so input ending stops them.
        let mut calls = JoinSet::new();
        let mut running: HashMap<String, tokio::task::AbortHandle> = HashMap::new();
        loop {
            let mut frame = Vec::new();
            let limit = u64::try_from(MAX_FRAME_BYTES + 1).map_err(std::io::Error::other)?;
            if (&mut reader)
                .take(limit)
                .read_until(b'\n', &mut frame)
                .await?
                == 0
            {
                return Ok::<(), std::io::Error>(());
            }
            if frame.len() > MAX_FRAME_BYTES {
                return Err(std::io::Error::other("MCP frame is too large."));
            }
            while calls.try_join_next().is_some() {}
            running.retain(|_, call| !call.is_finished());
            let request: Value = serde_json::from_slice(&frame).unwrap_or(Value::Null);
            if request["method"] == "notifications/cancelled" {
                if let Some(call) = running.remove(&request["params"]["requestId"].to_string()) {
                    call.abort(); // Hangs up on the broker, which withdraws the request.
                }
                continue;
            }
            if request["jsonrpc"] == "2.0"
                && request["method"] == "tools/call"
                && (request["id"].is_number() || request["id"].is_string())
            {
                let (connection, responses) = (connection.clone(), responses.clone());
                let id = request["id"].clone();
                let call = calls.spawn(async move {
                    let result = call_tool(&connection, &request["params"]).await;
                    let _ = responses
                        .send(json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
                        .await;
                });
                running.insert(id.to_string(), call);
            } else if let Some(response) = handle(&request, connection) {
                if responses.send(response).await.is_err() {
                    return Ok(());
                }
            }
        }
    };
    let write = async {
        while let Some(response) = outbox.recv().await {
            let mut bytes = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
            bytes.push(b'\n');
            output.write_all(&bytes).await?;
            output.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    };
    tokio::select! {
        result = read => result,
        result = write => result,
    }
}

fn tools(connection: &Connection) -> Value {
    let reference = json!({"type": "string", "description": "Thread id, or its unique prefix, from list_threads."});
    // Read-only, so a Plan turn may call them.
    let reads = json!({"readOnlyHint": true, "openWorldHint": false});
    let mut tools = vec![
        json!({
            "name": "list_threads",
            "description": "List the user's chat threads in this app: id, title, project, and whether each is running. Each thread has its own agent and context.",
            "annotations": reads,
            "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}}
        }),
        json!({
            "name": "read_thread",
            "description": "Read the latest messages of another thread: what was asked there and what its agent answered.",
            "annotations": reads,
            "inputSchema": {
                "type": "object", "additionalProperties": false, "required": ["thread_id"],
                "properties": {
                    "thread_id": reference,
                    "limit": {"type": "integer", "minimum": 1, "maximum": 20, "description": "Messages to return. Default 6."}
                }
            }
        }),
    ];
    if !connection.read_only {
        let mut properties = json!({
            "thread_id": reference,
            "title": {"type": "string", "description": "Title for a new thread."},
            "message": {"type": "string", "description": "Self-contained: the other agent sees none of this conversation."}
        });
        let returns = if connection.can_wait {
            properties["wait"] = json!({"type": "boolean", "description": "Hold this call until that thread has replied and return its reply. Use it when you need the answer to continue. Default false."});
            "Returns at once unless wait is true; without wait, any answer arrives later as a new message in this thread, so never poll for it."
        } else {
            "Returns at once; any answer arrives later as a new message in this thread, so never wait or poll for it."
        };
        tools.push(json!({
            "name": WRITE_TOOL,
            "description": format!("Send a message to another thread's agent. It runs there as that thread's next turn, with that thread's context. Omit thread_id to start a new thread in this project for work that can run on its own. {returns}"),
            "inputSchema": {
                "type": "object", "additionalProperties": false, "required": ["message"],
                "properties": properties
            }
        }));
    }
    json!({ "tools": tools })
}

/// Answer everything but `tools/call`. `None` for a notification.
fn handle(request: &Value, connection: &Connection) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str);
    if request.is_null() {
        return Some(error(Value::Null, -32700, "Invalid JSON."));
    }
    if request["jsonrpc"] != "2.0"
        || method.is_none()
        || !(id.is_null() || id.is_number() || id.is_string())
    {
        return Some(error(id, -32600, "Invalid JSON-RPC request."));
    }
    if id.is_null() {
        return None;
    }
    let result = match method {
        Some("initialize") => json!({
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "bruhs-threads", "version": env!("CARGO_PKG_VERSION")},
        }),
        Some("ping") => json!({}),
        Some("tools/list") => tools(connection),
        _ => return Some(error(id, -32601, "Method not found.")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Run one tool call and shape its MCP result.
async fn call_tool(connection: &Connection, params: &Value) -> Value {
    let op = match params["name"].as_str() {
        Some("list_threads") => "list",
        Some("read_thread") => "read",
        Some(WRITE_TOOL) if !connection.read_only => "message",
        _ => return tool_result(&ThreadReply::refused("Unknown thread tool.")),
    };
    let mut arguments = match &params["arguments"] {
        Value::Object(arguments) => arguments.clone(),
        Value::Null => serde_json::Map::new(),
        _ => return tool_result(&ThreadReply::refused("Arguments must be an object.")),
    };
    if !connection.can_wait {
        // Not offered here, so a model that sends it anyway is not held.
        arguments.remove("wait");
    }
    arguments.insert("op".into(), json!(op));
    let reply = match serde_json::from_value::<ThreadOperation>(Value::Object(arguments)) {
        Ok(operation) => call(connection, operation).await.unwrap_or_else(|_| {
            ThreadReply::refused("The app is not reachable from this turn any more.")
        }),
        Err(error) => ThreadReply::refused(format!("Invalid arguments: {error}.")),
    };
    tool_result(&reply)
}

fn tool_result(reply: &ThreadReply) -> Value {
    json!({"content": [{"type": "text", "text": reply.text}], "isError": !reply.ok})
}

async fn call(connection: &Connection, operation: ThreadOperation) -> std::io::Result<ThreadReply> {
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, connection.port)).await?;
    write_frame(
        &mut stream,
        &BrokerRequest {
            token: connection.token.clone(),
            operation,
        },
    )
    .await?;
    serde_json::from_slice(&read_frame(&mut stream).await?).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    struct Turn {
        links: ThreadLinks,
        thread_id: ThreadId,
        scope: Arc<ThreadScope>,
        events: mpsc::Receiver<OrchestratorEvent>,
        // Keeps the turn's stream open, as the producer task does.
        _tx: mpsc::Sender<OrchestratorEvent>,
    }

    fn turn() -> Turn {
        turn_in(ThreadLinks::default())
    }

    fn turn_in(links: ThreadLinks) -> Turn {
        let thread_id = ThreadId::new_v4();
        let (tx, events) = mpsc::channel(16);
        let scope = links.turn(thread_id.clone(), &tx);
        Turn {
            links,
            thread_id,
            scope,
            events,
            _tx: tx,
        }
    }

    async fn next_request(events: &mut mpsc::Receiver<OrchestratorEvent>) -> ThreadRequest {
        match tokio::time::timeout(WAIT, events.recv()).await {
            Ok(Some(OrchestratorEvent::Query(QueryEvent::ThreadRequest(request)))) => request,
            other => panic!("expected a thread request, got {other:?}"),
        }
    }

    fn read(thread_id: &str) -> ThreadOperation {
        ThreadOperation::Read {
            thread_id: thread_id.into(),
            limit: None,
        }
    }

    fn message(text: &str) -> ThreadOperation {
        ThreadOperation::Message {
            thread_id: None,
            title: None,
            message: text.into(),
            wait: false,
        }
    }

    fn waiting(text: &str) -> ThreadOperation {
        ThreadOperation::Message {
            thread_id: None,
            title: None,
            message: text.into(),
            wait: true,
        }
    }

    #[tokio::test]
    async fn the_app_answers_a_request_once_and_only_for_its_own_thread() {
        let mut turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let connection = broker.connection.clone();
        let asked = tokio::spawn(async move { call(&connection, read("abcd1234")).await });
        let request = next_request(&mut turn.events).await;
        assert_eq!(request.thread_id, turn.thread_id);
        assert_eq!(request.operation, read("abcd1234"));
        assert_eq!(
            turn.links
                .resolve(&ThreadId::new_v4(), &request.id, ThreadReply::ok("other")),
            Err(ThreadLinkError::NotPending)
        );
        turn.links
            .resolve(&turn.thread_id, &request.id, ThreadReply::ok("transcript"))
            .unwrap();
        let reply = tokio::time::timeout(WAIT, asked)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply, ThreadReply::ok("transcript"));
        assert_eq!(
            turn.links
                .resolve(&turn.thread_id, &request.id, ThreadReply::ok("again")),
            Err(ThreadLinkError::NotPending)
        );
        assert!(turn.links.lock().is_empty());
    }

    #[tokio::test]
    async fn overlapping_calls_are_answered_independently() {
        let mut turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let calls: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|id| {
                let connection = broker.connection.clone();
                tokio::spawn(async move { call(&connection, read(id)).await })
            })
            .collect();
        let first = next_request(&mut turn.events).await;
        let second = next_request(&mut turn.events).await;
        // Answered in the opposite order to prove neither blocks the other.
        for request in [&second, &first] {
            let ThreadOperation::Read { thread_id, .. } = &request.operation else {
                panic!("expected a read");
            };
            turn.links
                .resolve(&turn.thread_id, &request.id, ThreadReply::ok(thread_id))
                .unwrap();
        }
        let mut replies = Vec::new();
        for call in calls {
            replies.push(call.await.unwrap().unwrap().text);
        }
        assert_eq!(replies, ["one", "two"]);
    }

    #[tokio::test]
    async fn a_bad_token_an_invalid_operation_and_a_read_only_write_never_reach_the_app() {
        let mut turn = turn();
        let broker = turn.scope.start(true).await.unwrap();
        let mut forged = broker.connection.clone();
        forged.token = "0".repeat(forged.token.len());
        assert_eq!(
            call(&forged, ThreadOperation::List).await.unwrap().text,
            "Unauthorized."
        );
        let invalid = call(&broker.connection, read(" ")).await.unwrap();
        assert!(!invalid.ok && invalid.text.contains("thread_id is empty"));
        let refused = call(&broker.connection, message("hi")).await.unwrap();
        assert!(!refused.ok && refused.text.contains("read-only"));
        assert!(turn.events.try_recv().is_err());
    }

    #[tokio::test]
    async fn ending_the_turn_ends_its_pending_calls_and_refuses_new_ones() {
        let mut turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let connection = broker.connection.clone();
        let asked = tokio::spawn(async move { call(&connection, ThreadOperation::List).await });
        let request = next_request(&mut turn.events).await;
        let guard = turn.scope.guard();
        drop(guard);
        assert_eq!(
            turn.links
                .resolve(&turn.thread_id, &request.id, ThreadReply::ok("late")),
            Err(ThreadLinkError::NotPending)
        );
        // The listener is gone or the call is refused; either way no answer.
        let late = tokio::time::timeout(WAIT, asked).await.unwrap().unwrap();
        assert!(late.map_or(true, |reply| !reply.ok));
        assert!(!turn.scope.request(ThreadOperation::List).await.ok);
        assert!(turn.links.lock().is_empty());
    }

    #[tokio::test]
    async fn a_cancelled_tool_call_withdraws_its_request() {
        let mut turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let connection = broker.connection.clone();
        let asked = tokio::spawn(async move { call(&connection, ThreadOperation::List).await });
        let request = next_request(&mut turn.events).await;
        asked.abort();
        tokio::time::timeout(WAIT, async {
            while turn.links.lock().contains_key(&request.id) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn an_app_that_never_answers_ends_the_call_with_a_reason() {
        let mut turn = turn();
        let scope = turn.scope.clone();
        let asked = tokio::spawn(async move { scope.request(ThreadOperation::List).await });
        let _request = next_request(&mut turn.events).await;
        let reply = asked.await.unwrap();
        assert!(!reply.ok && reply.text.contains("did not answer"));
        assert!(turn.links.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiting_call_is_held_past_the_ordinary_deadline_only_where_the_app_can_wait() {
        let mut turn = turn_in(ThreadLinks::default().with_waits(true));
        let scope = turn.scope.clone();
        let asked = tokio::spawn(async move { scope.request(waiting("sum")).await });
        let request = next_request(&mut turn.events).await;
        tokio::time::sleep(ANSWER * 10).await;
        assert!(!asked.is_finished(), "the call must still be held");
        turn.links
            .resolve(&turn.thread_id, &request.id, ThreadReply::ok("4"))
            .unwrap();
        assert_eq!(asked.await.unwrap(), ThreadReply::ok("4"));

        // Never answered: it ends after the wait, saying the message went out.
        let scope = turn.scope.clone();
        let asked = tokio::spawn(async move { scope.request(waiting("sum")).await });
        let _request = next_request(&mut turn.events).await;
        let reply = asked.await.unwrap();
        assert!(!reply.ok && reply.text.contains("No reply after an hour"));

        // An app that cannot wait answers at once, so the short deadline holds.
        let mut turn = self::turn();
        let scope = turn.scope.clone();
        let asked = tokio::spawn(async move { scope.request(waiting("sum")).await });
        let _request = next_request(&mut turn.events).await;
        let reply = asked.await.unwrap();
        assert!(!reply.ok && reply.text.contains("did not answer"));
    }

    #[tokio::test]
    async fn wait_is_offered_and_passed_on_only_where_the_app_can_wait() {
        for can_wait in [true, false] {
            let mut turn = turn_in(ThreadLinks::default().with_waits(can_wait));
            let broker = turn.scope.start(false).await.unwrap();
            assert_eq!(broker.connection.can_wait, can_wait);
            let listed = tools(&broker.connection);
            let send = &listed["tools"][2];
            assert_eq!(send["name"], WRITE_TOOL);
            assert_eq!(
                send["inputSchema"]["properties"]["wait"].is_object(),
                can_wait
            );
            assert_eq!(
                send["description"]
                    .as_str()
                    .unwrap()
                    .contains("unless wait is true"),
                can_wait
            );
            for read in [&listed["tools"][0], &listed["tools"][1]] {
                assert_eq!(read["annotations"]["readOnlyHint"], true);
            }
            let connection = broker.connection.clone();
            let called = tokio::spawn(async move {
                call_tool(
                    &connection,
                    &json!({"name": "message_thread", "arguments": {"message": "hi", "wait": true}}),
                )
                .await
            });
            let request = next_request(&mut turn.events).await;
            assert_eq!(request.operation.waits(), can_wait);
            turn.links
                .resolve(&turn.thread_id, &request.id, ThreadReply::ok("done"))
                .unwrap();
            assert_eq!(called.await.unwrap()["isError"], false);
        }
        // A read-only turn never waits, whatever the app can do.
        let turn = turn_in(ThreadLinks::default().with_waits(true));
        assert!(!turn.scope.start(true).await.unwrap().connection.can_wait);
    }

    #[tokio::test]
    async fn the_helper_lists_tools_by_access_and_runs_calls_side_by_side() {
        let mut turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let connection = broker.connection.clone();
        let (client, server) = tokio::io::duplex(MAX_FRAME_BYTES);
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut responses = BufReader::new(client_read).lines();
        let helper = tokio::spawn(async move {
            let (input, mut output) = tokio::io::split(server);
            serve(input, &mut output, &connection).await
        });
        let mut next = async || -> Value {
            let line = tokio::time::timeout(WAIT, responses.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            serde_json::from_str(&line).unwrap()
        };
        client_write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
            .await
            .unwrap();
        let listed = next().await;
        let names: Vec<_> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["list_threads", "read_thread", "message_thread"]);

        // Two calls, then a ping that must not wait for either.
        client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"read_thread\",\"arguments\":{\"thread_id\":\"abcd1234\",\"limit\":3}}}\n{\"jsonrpc\":\"2.0\",\"id\":\"m\",\"method\":\"tools/call\",\"params\":{\"name\":\"message_thread\",\"arguments\":{\"message\":\"hi\",\"title\":\"T\"}}}\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n").await.unwrap();
        assert_eq!(next().await["id"], 3);
        for _ in 0..2 {
            let request = next_request(&mut turn.events).await;
            let (text, ok) = match &request.operation {
                ThreadOperation::Read { thread_id, limit } => {
                    assert_eq!((thread_id.as_str(), *limit), ("abcd1234", Some(3)));
                    ("read it", true)
                }
                ThreadOperation::Message { title, message, .. } => {
                    assert_eq!((title.as_deref(), message.as_str()), (Some("T"), "hi"));
                    ("not sent", false)
                }
                ThreadOperation::List => panic!("unexpected list"),
            };
            turn.links
                .resolve(
                    &turn.thread_id,
                    &request.id,
                    ThreadReply {
                        ok,
                        text: text.into(),
                    },
                )
                .unwrap();
        }
        let mut results = [next().await, next().await];
        results.sort_by_key(|result| result["id"].is_string());
        assert_eq!(results[0]["id"], 2);
        assert_eq!(results[0]["result"]["content"][0]["text"], "read it");
        assert_eq!(results[0]["result"]["isError"], false);
        assert_eq!(results[1]["id"], "m");
        assert_eq!(results[1]["result"]["isError"], true);

        // Bad arguments and unknown tools are tool errors, not app requests.
        client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"read_thread\",\"arguments\":{\"thread_id\":\"a\",\"extra\":1}}}\n").await.unwrap();
        let invalid = next().await;
        assert_eq!(invalid["result"]["isError"], true);
        client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"delete_thread\"}}\nnot json\n").await.unwrap();
        let mut rest = [next().await, next().await];
        rest.sort_by_key(|response| response["id"].is_null());
        assert_eq!(
            rest[0]["result"]["content"][0]["text"],
            "Unknown thread tool."
        );
        assert_eq!(rest[1]["error"]["code"], -32700);
        assert!(turn.events.try_recv().is_err());

        client_write.shutdown().await.unwrap();
        tokio::time::timeout(WAIT, helper)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_read_only_helper_neither_lists_nor_runs_the_write_tool() {
        let connection = Connection {
            port: 1,
            token: "test".into(),
            read_only: true,
            can_wait: false,
        };
        let listed = handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            &connection,
        )
        .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 2);
        let result = call_tool(
            &connection,
            &json!({"name": "message_thread", "arguments": {"message": "hi"}}),
        )
        .await;
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "Unknown thread tool.");
    }

    #[tokio::test]
    async fn the_helper_withdraws_a_call_the_client_cancels() {
        let mut turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let connection = broker.connection.clone();
        let (client, server) = tokio::io::duplex(MAX_FRAME_BYTES);
        let (_client_read, mut client_write) = tokio::io::split(client);
        let helper = tokio::spawn(async move {
            let (input, mut output) = tokio::io::split(server);
            serve(input, &mut output, &connection).await
        });
        client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"list_threads\"}}\n").await.unwrap();
        let request = next_request(&mut turn.events).await;
        client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":7}}\n").await.unwrap();
        tokio::time::timeout(WAIT, async {
            while turn.links.lock().contains_key(&request.id) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        client_write.shutdown().await.unwrap();
        tokio::time::timeout(WAIT, helper)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn configuration_joins_the_turns_other_servers_and_preapproves_only_reads() {
        let turn = turn();
        let broker = turn.scope.start(false).await.unwrap();
        let mut claude = claude_sdk_rs::QueryOptions::new("hello");
        claude.mcp_config =
            Some(json!({"mcpServers": {"bruhs_secrets": {"command": "x"}}}).to_string());
        claude.tools =
            claude_sdk_rs::Tools::Allow(vec!["mcp__bruhs_secrets__request_secret".into()]);
        let mut options = ProviderQueryOptions::Claude(claude);
        broker.configure(&mut options).unwrap();
        let ProviderQueryOptions::Claude(claude) = options else {
            panic!("expected Claude");
        };
        let config: Value = serde_json::from_str(claude.mcp_config.as_deref().unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["bruhs_secrets"]["command"], "x");
        assert_eq!(config["mcpServers"][SERVER]["args"][0], HELPER_FLAG);
        assert!(config["mcpServers"][SERVER]["env"][BROKER_ENV].is_string());
        assert_eq!(
            claude.tools,
            claude_sdk_rs::Tools::Allow(vec![
                "mcp__bruhs_secrets__request_secret".into(),
                "mcp__bruhs_threads__list_threads".into(),
                "mcp__bruhs_threads__read_thread".into(),
            ])
        );
        assert_eq!(claude.prompt, "hello");
        assert_eq!(claude.append_system_prompt, None);

        let mut bare = ProviderQueryOptions::Claude(claude_sdk_rs::QueryOptions::new("hello"));
        broker.configure(&mut bare).unwrap();
        let ProviderQueryOptions::Claude(bare) = bare else {
            panic!("expected Claude");
        };
        assert_eq!(
            bare.tools,
            claude_sdk_rs::Tools::Allow(vec![
                "mcp__bruhs_threads__list_threads".into(),
                "mcp__bruhs_threads__read_thread".into(),
            ])
        );
        let mut broken = claude_sdk_rs::QueryOptions::new("hello");
        broken.mcp_config = Some("[]".into());
        assert!(broker
            .configure(&mut ProviderQueryOptions::Claude(broken))
            .is_err());
    }

    #[tokio::test]
    async fn codex_gets_the_write_tool_only_when_the_turn_may_write() {
        for read_only in [false, true] {
            let turn = turn();
            let broker = turn.scope.start(read_only).await.unwrap();
            let mut options = ProviderQueryOptions::Codex(codex_sdk_rs::QueryOptions::new("hello"));
            broker.configure(&mut options).unwrap();
            let ProviderQueryOptions::Codex(options) = options else {
                panic!("expected Codex");
            };
            assert_eq!(options.prompt, "hello");
            assert_eq!(options.config.len(), 1);
            assert_eq!(options.config[0].key(), "mcp_servers.bruhs_threads");
            let value = options.config[0].value();
            assert!(value.contains("required = true"));
            assert!(value.contains("tool_timeout_sec = 120,"));
            assert!(value.contains("read_thread = { approval_mode = \"approve\" }"));
            assert_eq!(value.contains("message_thread"), !read_only);
            assert!(!value.contains("default_tools_approval_mode"));
        }
    }

    #[tokio::test]
    async fn codex_may_hold_a_waiting_call_past_its_own_tool_deadline() {
        let turn = turn_in(ThreadLinks::default().with_waits(true));
        let broker = turn.scope.start(false).await.unwrap();
        let mut options = ProviderQueryOptions::Codex(codex_sdk_rs::QueryOptions::new("hello"));
        broker.configure(&mut options).unwrap();
        let ProviderQueryOptions::Codex(options) = options else {
            panic!("expected Codex");
        };
        assert!(options.config[0]
            .value()
            .contains("tool_timeout_sec = 3660,"));
    }
}
