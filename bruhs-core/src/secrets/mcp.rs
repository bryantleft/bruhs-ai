//! The host executable's private MCP stdio mode. It cannot submit or read secrets.

use super::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};

pub fn is_mcp_helper() -> bool {
    std::env::args_os().any(|arg| arg == "--secret-mcp")
}

pub async fn run_mcp_helper() -> std::io::Result<()> {
    let connection: Connection = std::env::var(BROKER_ENV)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| std::io::Error::other("Credential broker is unavailable."))?;
    serve(tokio::io::stdin(), &mut tokio::io::stdout(), &connection).await
}

async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: R,
    output: &mut W,
    connection: &Connection,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(input);
    let mut frame = Vec::new();
    let mut active: Option<ActiveCall> = None;
    loop {
        tokio::select! {
            length = read_stdio_line(&mut reader, &mut frame) => {
                if length? == 0 { return Ok(()); }
                if frame.len() > MAX_FRAME_BYTES { return Err(std::io::Error::other("MCP metadata frame is too large.")); }
                let frame = std::mem::take(&mut frame);
                let request: Value = serde_json::from_slice(&frame).unwrap_or(Value::Null);
                if request["jsonrpc"] == "2.0" && request["method"] == "notifications/cancelled" {
                    if active.as_ref().is_some_and(|call| call.id == request["params"]["requestId"]) {
                        active = None; // Drop aborts the broker call, withdrawing its private form.
                    }
                    continue;
                }
                if request["method"] == "tools/call" && request.get("id").is_some() {
                    if active.is_some() {
                        send_response(output, &error(request["id"].clone(), -32600, "Only one private operation may be pending at a time.")).await?;
                    } else {
                        let connection = connection.clone();
                        active = Some(ActiveCall { id: request["id"].clone(), task: tokio::spawn(async move { handle(&frame, &connection).await }) });
                    }
                } else if let Some(response) = handle(&frame, connection).await {
                    send_response(output, &response).await?;
                }
            }
            result = async {
                match active.as_mut() {
                    Some(call) => (&mut call.task).await,
                    None => std::future::pending().await,
                }
            } => {
                let call = active.take();
                match result {
                    Ok(Some(response)) => send_response(output, &response).await?,
                    Ok(None) => {},
                    Err(_) => send_response(output, &error(call.map_or(Value::Null, |call| call.id.clone()), -32603, "Credential operation was interrupted.")).await?,
                }
            }
        }
    }
}

struct ActiveCall {
    id: Value,
    task: JoinHandle<Option<Value>>,
}
impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_stdio_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    frame: &mut Vec<u8>,
) -> std::io::Result<usize> {
    let remaining = MAX_FRAME_BYTES.saturating_sub(frame.len()) + 1;
    reader
        .take(u64::try_from(remaining).map_err(std::io::Error::other)?)
        .read_until(b'\n', frame)
        .await
}

async fn send_response<W: AsyncWrite + Unpin>(
    output: &mut W,
    response: &Value,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(response).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await
}

async fn handle(frame: &[u8], connection: &Connection) -> Option<Value> {
    let request = match serde_json::from_slice::<Value>(frame) {
        Ok(value) => value,
        Err(_) => return Some(error(Value::Null, -32700, "Invalid JSON.")),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str);
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
            "serverInfo": {"name": "bruhs-secrets", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        }),
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools": [{
            "name": "request_secret",
            "description": "Request private credential input and approval to run this exact command. The user can supply once or save in the OS keychain. No credential values or command output return to the model; only exit status. Never ask for credentials in chat. Command runs in this turn's working directory. Shell expansion requires explicit shell argv. Every invocation requires user approval.",
            "inputSchema": {
                "type": "object", "additionalProperties": false,
                "required": ["name", "reason", "env_var", "command"],
                "properties": {
                    "name": {"type": "string", "description": "Stable project-scoped credential name, e.g. GitHub token."},
                    "reason": {"type": "string", "description": "Explain what this command needs the credential for."},
                    "env_var": {"type": "string", "description": "Credential environment variable, e.g. GH_TOKEN."},
                    "command": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "Executable followed by exact literal arguments. No credential values."}
                }
            }
        }, {
            "name": "request_connection",
            "description": "Request approval for a command using the user's Google Workspace, Microsoft 365, or GitHub account. Bruhs offers browser SSO connection if needed. Only a short-lived access token reaches the approved command; no credentials or command output return to the model. Every operation needs user approval. Google grants calendar-events read access, Microsoft grants basic calendar read access, and GitHub grants profile and organization read access, not private repository access.",
            "inputSchema": {
                "type": "object", "additionalProperties": false,
                "required": ["provider", "reason", "env_var", "command"],
                "properties": {
                    "provider": {"type": "string", "enum": ["google", "microsoft", "github"]},
                    "reason": {"type": "string"},
                    "env_var": {"type": "string", "description": "Variable that receives this service's access token."},
                    "command": {"type": "array", "minItems": 1, "items": {"type": "string"}}
                }
            }
        }]}),
        Some("tools/call") => {
            let (operation, provider) = if request["params"]["name"] == "request_secret" {
                (
                    serde_json::from_value::<SecretOperation>(
                        request["params"]["arguments"].clone(),
                    ),
                    None,
                )
            } else if request["params"]["name"] == "request_connection" {
                match serde_json::from_value::<bruhs_models::connections::ServiceOperation>(
                    request["params"]["arguments"].clone(),
                ) {
                    Ok(operation) => (Ok(operation.secret_operation()), Some(operation.provider)),
                    Err(error) => (Err(error), None),
                }
            } else {
                return Some(error(id, -32602, "Unknown credential tool."));
            };
            let result = match operation {
                Ok(operation) if operation.validate().is_ok() => {
                    call_from(connection, operation, provider)
                        .await
                        .unwrap_or_else(|_| OperationResult::status("broker_disconnected"))
                }
                _ => OperationResult::status("invalid_operation"),
            };
            json!({"content": [{"type": "text", "text": json!(&result).to_string()}], "isError": result.status != "completed" || result.exit_code != Some(0)})
        }
        _ => return Some(error(id, -32601, "Method not found.")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
pub(super) async fn call(
    connection: &Connection,
    operation: SecretOperation,
) -> std::io::Result<OperationResult> {
    call_from(connection, operation, None).await
}

async fn call_from(
    connection: &Connection,
    operation: SecretOperation,
    provider: Option<bruhs_models::connections::ServiceProvider>,
) -> std::io::Result<OperationResult> {
    let mut stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, connection.port)).await?;
    write_frame(
        &mut stream,
        &BrokerRequest {
            token: connection.token.clone(),
            operation,
            connection: provider,
        },
    )
    .await?;
    serde_json::from_slice(&read_frame(&mut stream).await?).map_err(std::io::Error::other)
}

/// Add the credential instructions to a Claude turn's system prompt.
/// Claude records a thread's system prompt on its first request and reuses
/// it on every resume, so a Plan turn (which gets no credential tool) adds
/// them too; otherwise a thread that starts in Plan would never learn them.
pub(crate) fn append_claude_instructions(options: &mut claude_sdk_rs::QueryOptions) {
    let instructions = options.append_system_prompt.get_or_insert_with(String::new);
    instructions.push('\n');
    instructions.push_str(INSTRUCTIONS);
}

impl SecretBroker {
    pub(crate) fn configure(
        &self,
        options: &mut crate::agent::ProviderQueryOptions,
    ) -> std::io::Result<()> {
        let executable = std::env::current_exe()?.to_string_lossy().into_owned();
        let connection = serde_json::to_string(&self.connection).map_err(std::io::Error::other)?;
        match options {
            crate::agent::ProviderQueryOptions::Claude(options) => {
                options.mcp_config = Some(json!({"mcpServers": {"bruhs_secrets": {
                    "command": executable, "args": ["--secret-mcp"], "env": {BROKER_ENV: connection}
                }}}).to_string());
                // Opening the form needs no second generic approval; its execution still does.
                for tool in [
                    "mcp__bruhs_secrets__request_secret",
                    "mcp__bruhs_secrets__request_connection",
                ] {
                    match &mut options.tools {
                        claude_sdk_rs::Tools::CliDefault => {
                            options.tools = claude_sdk_rs::Tools::Allow(vec![tool.into()])
                        }
                        claude_sdk_rs::Tools::Allow(tools)
                            if !tools.iter().any(|name| name == tool) =>
                        {
                            tools.push(tool.into())
                        }
                        _ => {}
                    }
                }
                append_claude_instructions(options);
            }
            crate::agent::ProviderQueryOptions::Codex(options) => {
                let executable = json!(executable).to_string();
                let connection = json!(connection).to_string();
                // Interactive input must outlive Codex's default one-minute tool deadline.
                // The one-day ceiling covers an unattended overnight turn; turn teardown revokes it sooner.
                // Required servers finish startup before Codex builds its initial tool catalog.
                // Approving this one tool opens our form; the harness still gates command execution.
                let config = format!("{{ command = {executable}, args = [\"--secret-mcp\"], env = {{ {BROKER_ENV} = {connection} }}, tool_timeout_sec = 86400, required = true, tools = {{ request_secret = {{ approval_mode = \"approve\" }}, request_connection = {{ approval_mode = \"approve\" }} }} }}");
                options.config.push(
                    codex_sdk_rs::ConfigOverride::new("mcp_servers.bruhs_secrets", config)
                        .map_err(std::io::Error::other)?,
                );
                // Preserve developer instructions in the user's Codex configuration.
                options.prompt = format!(
                    "<bruhs_harness>\n{INSTRUCTIONS}\n</bruhs_harness>\n\n{}",
                    options.prompt
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan turns add the same instructions the broker adds, so the system
    /// prompt Claude records on a thread's first request has them whichever
    /// mode the thread starts in.
    #[test]
    fn plan_and_broker_turns_record_the_same_instructions() {
        let mut plan = claude_sdk_rs::QueryOptions::new("hi");
        plan.append_system_prompt = Some("meeting".into());
        append_claude_instructions(&mut plan);
        let prompt = plan.append_system_prompt.unwrap();
        assert!(prompt.starts_with("meeting\n") && prompt.ends_with(INSTRUCTIONS));
    }

    #[tokio::test]
    async fn provider_configuration_carries_only_a_temporary_capability_and_narrow_request_approval(
    ) {
        let manager = SecretManager::new(SecretStore::in_memory());
        let (tx, _) = mpsc::channel(1);
        let scope = manager.turn(ThreadId::new_v4(), PathBuf::from("/tmp"), tx);
        let broker = scope.start().await.unwrap();
        let mut options =
            crate::agent::ProviderQueryOptions::Claude(claude_sdk_rs::QueryOptions::new("hello"));
        broker.configure(&mut options).unwrap();
        let crate::agent::ProviderQueryOptions::Claude(options) = options else {
            panic!("expected Claude");
        };
        assert_eq!(
            options.tools,
            claude_sdk_rs::Tools::Allow(vec![
                "mcp__bruhs_secrets__request_secret".into(),
                "mcp__bruhs_secrets__request_connection".into()
            ])
        );
        let config: Value = serde_json::from_str(options.mcp_config.as_ref().unwrap()).unwrap();
        assert!(config["mcpServers"]["bruhs_secrets"]["env"][BROKER_ENV]
            .as_str()
            .is_some());
        assert_eq!(
            config["mcpServers"]["bruhs_secrets"]["args"][0],
            "--secret-mcp"
        );
        let mut codex =
            crate::agent::ProviderQueryOptions::Codex(codex_sdk_rs::QueryOptions::new("hello"));
        broker.configure(&mut codex).unwrap();
        let crate::agent::ProviderQueryOptions::Codex(options) = codex else {
            panic!("expected Codex");
        };
        assert_eq!(options.config.len(), 1);
        assert_eq!(options.config[0].key(), "mcp_servers.bruhs_secrets");
        assert!(options.config[0].value().contains("required = true"));
        assert!(options.config[0]
            .value()
            .contains("request_secret = { approval_mode = \"approve\" }"));
        assert!(!options.config[0]
            .value()
            .contains("default_tools_approval_mode"));
        assert!(options.prompt.ends_with("hello"));
        scope.revoke();
    }

    #[tokio::test]
    async fn stdio_cancel_and_eof_withdraw_pending_input_without_ending_the_turn() {
        for eof in [false, true] {
            let manager = SecretManager::new(SecretStore::in_memory());
            let directory = tempfile::tempdir().unwrap();
            let (tx, mut events) = mpsc::channel(16);
            let scope = manager.turn(ThreadId::new_v4(), directory.path().into(), tx.clone());
            let broker = scope.start().await.unwrap();
            let connection = broker.connection.clone();
            let (client, server) = tokio::io::duplex(MAX_FRAME_BYTES);
            let (client_read, mut client_write) = tokio::io::split(client);
            let mut responses = BufReader::new(client_read);
            let helper = tokio::spawn(async move {
                let (input, mut output) = tokio::io::split(server);
                serve(input, &mut output, &connection).await
            });
            client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"request_secret\",\"arguments\":{\"name\":\"key\",\"reason\":\"test\",\"env_var\":\"KEY\",\"command\":[\"true\"]}}}\n").await.unwrap();
            let request = loop {
                let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if let OrchestratorEvent::Query(QueryEvent::SecretRequest(request)) = event {
                    break request;
                }
            };
            // The helper must remain responsive while its tool call waits on private input.
            client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":999}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n").await.unwrap();
            let mut response = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                responses.read_line(&mut response),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(serde_json::from_str::<Value>(&response).unwrap()["id"], 2);
            assert!(manager.lock().contains_key(&request.id));
            if eof {
                client_write.shutdown().await.unwrap();
            } else {
                client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":1}}\n").await.unwrap();
            }
            loop {
                let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if matches!(event, OrchestratorEvent::Query(QueryEvent::SecretResolved { ref id }) if id == &request.id)
                {
                    break;
                }
            }
            assert!(
                scope.active.load(Ordering::Acquire),
                "only the tool call was cancelled"
            );
            assert!(matches!(
                manager
                    .resolve(&request.thread_id, &request.id, SecretDecision::UseSaved)
                    .await,
                Err(SecretError::NotPending)
            ));
            client_write.shutdown().await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), helper)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            scope.revoke();
        }
    }

    #[tokio::test]
    async fn mcp_exposes_only_the_private_operation_and_rejects_secret_arguments() {
        let connection = Connection {
            port: 1,
            token: "test".into(),
        };
        let list = handle(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &connection,
        )
        .await
        .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 2);
        let response = handle(br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"request_secret","arguments":{"name":"key","reason":"test","env_var":"KEY","command":["true"],"value":"synthetic-private-value"}}}"#, &connection).await.unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(!response.to_string().contains("synthetic-private-value"));
    }
}
