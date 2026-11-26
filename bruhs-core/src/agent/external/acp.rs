//! ACP v1 JSON-RPC bridge. Capabilities are negotiated; native modes are not OS sandboxes.
use super::{process, Options};
use crate::agent::{PermissionDecision, QueryEvent, TurnContext, UsageInfo};
use bruhs_models::{agent::PermissionMode, agent_extensions::AcpInstance};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io,
    sync::{Mutex, OnceLock},
};
use tokio::{io::AsyncWriteExt, process::ChildStdin, sync::oneshot};

// Reuse Axum's default JSON body budget for this local JSON transport. Media
// client capabilities are not advertised; oversized tool metadata fails early.
const FRAME_BYTES: u64 = 2 * 1024 * 1024;

struct Pending {
    thread: String,
    send: oneshot::Sender<bool>,
}
static APPROVALS: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
fn approvals() -> &'static Mutex<HashMap<String, Pending>> {
    APPROVALS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(in crate::agent) fn resolve(id: &str, decision: &PermissionDecision) -> bool {
    let pending = approvals().lock().ok().and_then(|mut map| map.remove(id));
    pending.is_some_and(|p| {
        p.send
            .send(matches!(decision, PermissionDecision::Allow))
            .is_ok()
    })
}
pub(in crate::agent) fn pending(thread: &str) -> Vec<String> {
    approvals()
        .lock()
        .map(|map| {
            map.iter()
                .filter(|(_, p)| p.thread == thread)
                .map(|(id, _)| id.clone())
                .collect()
        })
        .unwrap_or_default()
}
struct ApprovalGuard(String);
impl Drop for ApprovalGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = approvals().lock() {
            map.remove(&self.0);
        }
    }
}

async fn write(stdin: &mut ChildStdin, message: Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&message).map_err(io::Error::other)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await
}

pub(super) async fn stream(options: Options, turn: TurnContext) {
    let result = run(&options, &turn).await;
    if let Err(error) = result {
        turn.send_start_error(format!("ACP agent failed: {error}"))
            .await;
    }
    turn.finish(!turn.tx.is_closed());
}

fn selected<'a>(
    model: &'a str,
    instances: &'a [AcpInstance],
    availability: &crate::agent_extensions::availability::ProviderAvailability,
) -> io::Result<(&'a AcpInstance, &'a str)> {
    let selection = model
        .strip_prefix("acp/")
        .ok_or_else(|| io::Error::other("ACP model must be acp/<instance>/<model>."))?;
    let (id, model) = selection.split_once('/').ok_or_else(|| {
        io::Error::other("Choose an ACP instance and model, for example acp/my-agent/default.")
    })?;
    let instance = instances
        .iter()
        .find(|i| i.id == id && availability.acp_enabled(i))
        .ok_or_else(|| {
            io::Error::other(format!(
                "ACP instance {id} is missing or disabled. Configure it in Workbench → Providers."
            ))
        })?;
    Ok((instance, model))
}

#[derive(Clone, Copy)]
enum Selector {
    Mode,
    Model,
}
struct Selection {
    method: &'static str,
    params: Value,
    expected: Option<(String, String)>,
}
fn native_selection(
    session: &Value,
    id: &str,
    selector: Selector,
    config_id: &str,
    value: &str,
) -> io::Result<Selection> {
    if let Some(config) = session
        .get("configOptions")
        .filter(|config| !config.is_null())
    {
        let option = config.as_array().and_then(|options| options.iter().find(|option| option["id"] == config_id))
            .ok_or_else(|| io::Error::other(format!("ACP agent does not advertise selector {config_id}. Configure its native selector ID in Workbench → Providers.")))?;
        if !select_offers(option, value) {
            return Err(io::Error::other(format!("ACP selector {config_id} does not advertise value {value}. Review the agent's current native mode/model IDs.")));
        }
        return Ok(Selection {
            method: "session/set_config_option",
            params: json!({"sessionId":id,"configId":config_id,"value":value}),
            expected: Some((config_id.into(), value.into())),
        });
    }
    let (method, params, known) = match selector {
        Selector::Mode => (
            "session/set_mode",
            json!({"sessionId":id,"modeId":value}),
            session["modes"]["availableModes"]
                .as_array()
                .is_some_and(|modes| modes.iter().any(|mode| mode["id"] == value)),
        ),
        Selector::Model => (
            "session/set_model",
            json!({"sessionId":id,"modelId":value}),
            session["models"]["availableModels"]
                .as_array()
                .is_none_or(|models| models.iter().any(|model| model["modelId"] == value)),
        ),
    };
    if !known {
        return Err(io::Error::other(format!("ACP agent does not advertise the configured native value {value}. Review its mode/model settings.")));
    }
    Ok(Selection {
        method,
        params,
        expected: None,
    })
}
fn updated_configuration(
    mut current: Value,
    update: Value,
    expected: Option<(String, String)>,
) -> io::Result<Value> {
    if let Some((id, value)) = expected {
        if !update["configOptions"].as_array().is_some_and(|options| {
            options
                .iter()
                .any(|option| option["id"] == id && option["currentValue"] == value)
        }) {
            return Err(io::Error::other(format!("ACP agent did not confirm selector {id}={value}. No prompt was sent. Update the adapter or review its configuration.")));
        }
    }
    for key in ["configOptions", "modes", "models"] {
        if let Some(value) = update.get(key) {
            current[key] = value.clone();
        }
    }
    Ok(current)
}

async fn run(options: &Options, turn: &TurnContext) -> io::Result<()> {
    let config = tokio::task::spawn_blocking(crate::agent_extensions::load_config)
        .await
        .map_err(io::Error::other)??;
    run_config(options, turn, &config).await
}

async fn run_config(
    options: &Options,
    turn: &TurnContext,
    config: &bruhs_models::agent_extensions::ExtensionConfig,
) -> io::Result<()> {
    if options.unattended {
        return Err(io::Error::other(
            "ACP unattended runs are unavailable: isolate inherited plugins/hooks with an enforced runtime before enabling them.",
        ));
    }
    if !options.workspace_trusted {
        if let Some(files) =
            crate::workspace_trust::has_unisolatable_external_agent_config(&options.cwd)
        {
            return Err(io::Error::other(format!(
                "ACP agents cannot isolate repository configuration ({files}) in an untrusted folder. Review those files and trust this folder first."
            )));
        }
    }
    let availability =
        crate::agent_extensions::availability::ProviderAvailability::load_for_project(
            &options.cwd,
        )?;
    let (instance, model) = selected(&options.model, &config.acp, &availability)?;
    let kiro = instance.driver == Some(bruhs_models::agent_extensions::AcpDriver::Kiro);
    if kiro && !instance.modes.contains_key(options.mode.as_str()) {
        return Err(io::Error::other(
            "Kiro supports Default (Supervised) and Full access. Choose one of those access modes.",
        ));
    }
    let mut command = tokio::process::Command::new(instance.executable());
    command
        .args(&instance.args)
        .current_dir(&options.cwd)
        .stdin(std::process::Stdio::piped());
    process::apply_env(
        &mut command,
        true,
        &crate::connections::PRIVATE_ENV.map(String::from),
        &[],
    );
    crate::agent_extensions::environment::apply(&mut command, instance, &options.secret_env)
        .await?;
    let mut child = process::CliProcess::spawn("ACP agent", &mut command)?;
    turn.register_process(child.pid());
    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| io::Error::other("ACP stdin unavailable."))?;
    let execution = async {
        let initialized=rpc(&mut child,&mut stdin,turn,options.mode,1,"initialize",json!({"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"clientInfo":{"name":"bruhs","version":env!("CARGO_PKG_VERSION")}})).await?;
        if initialized["protocolVersion"] != 1 {
            return Err(io::Error::other(
                "Agent did not negotiate ACP v1. Use a compatible ACP command.",
            ));
        }
        let mcp:Vec<Value>=options.mcp.iter().map(|(name,s)|json!({"name":name,"command":s.command,"args":s.args,"env":s.env.iter().map(|(name,value)|json!({"name":name,"value":value})).collect::<Vec<_>>()})).collect();
        let native = resume_session(options.resume.as_deref(), &instance.id)?;
        let (method, params) = match native {
            Some(session) if initialized["agentCapabilities"]["loadSession"] == true => (
                "session/load",
                json!({"sessionId":session,"cwd":options.cwd,"mcpServers":mcp}),
            ),
            Some(_) => {
                return Err(io::Error::other(
                    "This ACP agent cannot reload saved sessions. Start a fresh session explicitly.",
                ));
            }
            None => ("session/new", json!({"cwd":options.cwd,"mcpServers":mcp})),
        };
        let opened = rpc(
            &mut child,
            &mut stdin,
            turn,
            options.mode,
            2,
            method,
            params.clone(),
        )
        .await;
        let session = match opened {
            Err(err)
                if !kiro
                    && err
                        .get_ref()
                        .is_some_and(|e| e.is::<AuthenticationRequired>()) =>
            {
                let preferred = instance
                    .registry
                    .as_ref()
                    .and_then(|origin| origin.auth_method_id.as_deref());
                let method_id = agent_auth_method(&initialized, preferred)?.ok_or(err)?;
                rpc(
                    &mut child,
                    &mut stdin,
                    turn,
                    options.mode,
                    6,
                    "authenticate",
                    json!({"methodId": method_id}),
                )
                .await?;
                rpc(
                    &mut child,
                    &mut stdin,
                    turn,
                    options.mode,
                    7,
                    method,
                    params,
                )
                .await?
            }
            result => result?,
        };
        let session_id = native
            .or_else(|| session["sessionId"].as_str())
            .ok_or_else(|| io::Error::other("ACP agent returned no session ID."))?
            .to_owned();
        let mode_id = instance.modes.get(options.mode.as_str());
        if options.mode != PermissionMode::BypassPermissions && mode_id.is_none() {
            return Err(io::Error::other(
                "Configure this ACP agent's native mode ID for the selected access mode, or explicitly choose Full access. Bruhs will not substitute an instruction for access enforcement.",
            ));
        }
        let mut configuration = session;
        let mut agent_switched = false;
        if let Some(agent) = options.kiro_selection.agent.as_deref().filter(|_| kiro) {
            if offered_choice(&configuration, "mode", agent)
                && !current_choice(&configuration, "mode", agent)
            {
                let updated = rpc(
                    &mut child,
                    &mut stdin,
                    turn,
                    options.mode,
                    11,
                    "session/set_config_option",
                    json!({"sessionId":session_id,"configId":"mode","value":agent}),
                )
                .await?;
                configuration = updated_configuration(
                    configuration,
                    updated,
                    Some(("mode".into(), agent.into())),
                )?;
                agent_switched = true;
            }
        }
        let model_option = kiro_model_option(&configuration);
        let catalog;
        let requested = if kiro && model == "default" && (agent_switched || model_option.is_some())
        {
            catalog = crate::agent_extensions::kiro::Catalog::load(instance);
            catalog.default_id()
        } else {
            model
        };
        if requested != "default"
            && (!kiro || !model_option.is_some_and(|option| option["currentValue"] == requested))
        {
            let selector = if kiro {
                model_option
                    .and_then(|option| option["id"].as_str())
                    .unwrap_or("model")
            } else {
                instance.model_config_id.as_deref().unwrap_or("model")
            };
            let selection = if kiro {
                Selection {
                    method: "session/set_config_option",
                    params: json!({"sessionId":session_id,"configId":selector,"value":requested}),
                    expected: None,
                }
            } else {
                native_selection(
                    &configuration,
                    &session_id,
                    Selector::Model,
                    selector,
                    requested,
                )?
            };
            let updated = rpc(
                &mut child,
                &mut stdin,
                turn,
                options.mode,
                3,
                selection.method,
                selection.params,
            )
            .await?;
            configuration = updated_configuration(configuration, updated, selection.expected)?;
        }
        if let Some(effort) = options.kiro_selection.effort.as_deref().filter(|_| kiro) {
            // Kiro may publish the new model's options shortly after acknowledging the write.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            while !kiro_model_option(&configuration)
                .is_some_and(|option| option["currentValue"] == requested)
            {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                let ready = tokio::select! {
                    () = turn.tx.closed() => return Err(io::Error::new(io::ErrorKind::Interrupted, "ACP task was cancelled.")),
                    ready = tokio::time::timeout_at(deadline, child.stdout_ready()) => ready,
                };
                match ready {
                    Err(_) => break,
                    Ok(result) => result?,
                }
                let message =
                    tokio::time::timeout(super::CONFIG_TIMEOUT, read_message(&mut child, turn))
                        .await
                        .map_err(|_| io::Error::other("Kiro did not finish an ACP message."))??;
                if let Some(values) = config_notification(&message, &session_id) {
                    configuration["configOptions"] = values.clone();
                }
                handle_message(&mut stdin, turn, options.mode, &message).await?;
            }
            if kiro_model_option(&configuration)
                .is_some_and(|option| option["currentValue"] == requested)
                && offered_choice(&configuration, "effortLevel", effort)
                && !current_choice(&configuration, "effortLevel", effort)
            {
                match rpc(
                    &mut child,
                    &mut stdin,
                    turn,
                    options.mode,
                    12,
                    "session/set_config_option",
                    json!({"sessionId":session_id,"configId":"effortLevel","value":effort}),
                )
                .await
                {
                    Ok(update) => {
                        configuration = updated_configuration(configuration, update, None)?
                    }
                    Err(e) if e.get_ref().is_some_and(|cause| cause.is::<RpcRejection>()) => {
                        tracing::debug!("Kiro declined the selected reasoning level")
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        // Apply access after all settings that can reset it and require native confirmation.
        if let Some(mode_id) = mode_id {
            if kiro && !configuration["configOptions"].is_array() {
                return Err(io::Error::other(
                    "Kiro V3 did not advertise its Autopilot access setting.",
                ));
            }
            let selection = native_selection(
                &configuration,
                &session_id,
                Selector::Mode,
                instance.mode_config_id.as_deref().unwrap_or("mode"),
                mode_id,
            )?;
            let updated = rpc(
                &mut child,
                &mut stdin,
                turn,
                options.mode,
                4,
                selection.method,
                selection.params,
            )
            .await?;
            updated_configuration(configuration, updated, selection.expected)?;
        }
        let stamped = format!("acp:{}:{session_id}", instance.id);
        if !turn
            .forward(QueryEvent::Init {
                session_id: stamped.clone(),
                model: Some(options.model.clone()),
                tools: vec![],
                mcp_servers: vec![],
            })
            .await
        {
            return Ok(());
        }
        let result = rpc(
            &mut child,
            &mut stdin,
            turn,
            options.mode,
            5,
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":options.prompt()}]}),
        )
        .await?;
        let cancelled = result["stopReason"] == "cancelled";
        turn.remember_session(&QueryEvent::Init {
            session_id: stamped.clone(),
            model: Some(options.model.clone()),
            tools: vec![],
            mcp_servers: vec![],
        });
        let _ = turn
            .forward(QueryEvent::Result {
                result: String::new(),
                session_id: stamped,
                duration_ms: 0,
                usage: UsageInfo::default(),
                is_error: cancelled,
            })
            .await;
        Ok(())
    };
    let result = execution.await;
    // ACP agents may stay alive awaiting another RPC after prompt completion.
    // This bridge reloads their persisted native session on the next turn.
    match result {
        Err(error) if !turn.tx.is_closed() => {
            let failure = child
                .finish(process::Ending::Failed(error.to_string()))
                .await;
            Err(failure.map_or(error, |failure| io::Error::other(failure.to_string())))
        }
        result => {
            let _ = child.finish(process::Ending::ConsumerGone).await;
            result
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct AuthenticationRequired(String);

fn agent_auth_method<'a>(
    initialized: &'a Value,
    preferred: Option<&'a str>,
) -> io::Result<Option<&'a str>> {
    let Some(methods) = initialized["authMethods"].as_array() else {
        return Ok(preferred);
    };
    let agent_managed = |m: &Value| m.get("type").is_none() || m["type"] == "agent";
    if let Some(id) = preferred {
        let method = methods.iter().find(|m| m["id"] == id).ok_or_else(|| {
            io::Error::other(format!(
                "The ACP agent did not advertise authentication method {id}."
            ))
        })?;
        if !agent_managed(method) {
            return Err(io::Error::other(format!(
                "Authentication method {id} requires an external sign-in."
            )));
        }
        return Ok(Some(id));
    }
    Ok(methods
        .iter()
        .filter(|m| agent_managed(m))
        .find_map(|m| m["id"].as_str()))
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct RpcRejection(String);

fn config_notification<'a>(message: &'a Value, id: &str) -> Option<&'a Value> {
    let p = &message["params"];
    let update = &p["update"];
    (message["method"] == "session/update"
        && p["sessionId"] == id
        && update["sessionUpdate"] == "config_option_update"
        && update["configOptions"].is_array())
    .then_some(&update["configOptions"])
}
fn kiro_model_option(config: &Value) -> Option<&Value> {
    config["configOptions"]
        .as_array()?
        .iter()
        .find(|o| o["id"] == "model" || o["category"] == "model")
}
fn current_choice(config: &Value, id: &str, value: &str) -> bool {
    config["configOptions"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|o| o["id"] == id && o["currentValue"] == value)
    })
}
fn select_offers(option: &Value, value: &str) -> bool {
    option["type"] == "select"
        && option["options"].as_array().is_some_and(|choices| {
            choices.iter().any(|entry| {
                entry["value"] == value
                    || entry["options"]
                        .as_array()
                        .is_some_and(|group| group.iter().any(|choice| choice["value"] == value))
            })
        })
}
fn offered_choice(config: &Value, id: &str, value: &str) -> bool {
    config["configOptions"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|o| o["id"] == id && select_offers(o, value))
    })
}
async fn read_message(child: &mut process::CliProcess, turn: &TurnContext) -> io::Result<Value> {
    let line = child
        .next_line_with_limit(&turn.tx, Some(FRAME_BYTES))
        .await
        .map_err(|ending| {
            io::Error::other(match ending {
                process::Ending::Failed(message) => message,
                process::Ending::Eof => {
                    "ACP agent closed its output. Check the executable and its login in a terminal."
                        .into()
                }
                process::Ending::ConsumerGone => "ACP task was cancelled.".into(),
            })
        })?;
    serde_json::from_str(&line)
        .map_err(|e| io::Error::other(format!("ACP returned invalid JSON: {e}")))
}
async fn handle_message(
    stdin: &mut ChildStdin,
    turn: &TurnContext,
    mode: PermissionMode,
    message: &Value,
) -> io::Result<()> {
    if message["method"] == "session/request_permission" {
        permission(stdin, turn, mode, message).await?;
    } else if message.get("id").is_some() && message.get("method").is_some() {
        write(stdin,json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"Bruhs did not advertise this client capability."}})).await?;
    } else if message["method"] == "session/update" {
        for event in events(&message["params"]["update"]) {
            if !turn.forward(event).await {
                return Err(io::Error::other("Task cancelled."));
            }
        }
    }
    Ok(())
}

async fn rpc(
    child: &mut process::CliProcess,
    stdin: &mut ChildStdin,
    turn: &TurnContext,
    mode: PermissionMode,
    id: u64,
    method: &str,
    params: Value,
) -> io::Result<Value> {
    let exchange = async {
        write(
            stdin,
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )
        .await?;
        let mut latest = None;
        loop {
            let message = read_message(child, turn).await?;
            if method == "session/set_config_option" {
                if let Some(values) = params["sessionId"]
                    .as_str()
                    .and_then(|id| config_notification(&message, id))
                {
                    latest = Some(values.clone());
                }
            }
            if message["id"].as_u64() == Some(id) && message.get("method").is_none() {
                if let Some(error) = message.get("error") {
                    let message = format!("{method}: {}", error["message"]);
                    if error["code"] == -32000 {
                        return Err(io::Error::other(AuthenticationRequired(message)));
                    }
                    return Err(io::Error::other(RpcRejection(message)));
                }
                let mut result = message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| io::Error::other("ACP result is missing."))?;
                if let Some(options) = latest {
                    let confirms_write = |list: &Value| {
                        list.as_array().is_some_and(|rows| {
                            rows.iter().any(|option| {
                                option["id"] == params["configId"]
                                    && option["currentValue"] == params["value"]
                            })
                        })
                    };
                    if result.is_object()
                        && !confirms_write(&result["configOptions"])
                        && confirms_write(&options)
                    {
                        result["configOptions"] = options;
                    }
                }
                return Ok(result);
            }
            handle_message(stdin, turn, mode, &message).await?;
        }
    };
    tokio::select! {
        biased;
        () = turn.tx.closed() => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "ACP task was cancelled.",
        )),
        result = async {
            // Sign-in can wait for the user in the agent. Cancel still interrupts the RPC.
            if matches!(method, "session/prompt" | "authenticate") {
                exchange.await
            } else {
                tokio::time::timeout(super::CONFIG_TIMEOUT, exchange)
                    .await
                    .map_err(|_| {
                        io::Error::other(format!(
                            "ACP {method} timed out. Check the agent's login and executable."
                        ))
                    })?
            }
        } => result,
    }
}
async fn permission(
    stdin: &mut ChildStdin,
    turn: &TurnContext,
    mode: PermissionMode,
    message: &Value,
) -> io::Result<()> {
    let choices = message["params"]["options"]
        .as_array()
        .ok_or_else(|| io::Error::other("ACP permission options are missing."))?;
    let allow = if mode == PermissionMode::BypassPermissions {
        true
    } else if mode == PermissionMode::Plan {
        false
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let (send, receive) = oneshot::channel();
        approvals()
            .lock()
            .map_err(|_| io::Error::other("ACP permission lock failed."))?
            .insert(
                id.clone(),
                Pending {
                    thread: turn.owner.thread_id.as_str().into(),
                    send,
                },
            );
        let _guard = ApprovalGuard(id.clone());
        let _ = turn
            .forward(QueryEvent::PermissionRequest {
                id,
                can_answer: false,
                tool_use_id: message["params"]["toolCall"]["toolCallId"]
                    .as_str()
                    .map(str::to_string),
                tool: message["params"]["toolCall"]["title"]
                    .as_str()
                    .unwrap_or("ACP tool")
                    .into(),
                input: message["params"]["toolCall"].clone(),
            })
            .await;
        tokio::select! {()=turn.tx.closed()=>false,answer=receive=>answer.unwrap_or(false)}
    };
    let kind = if allow { "allow_once" } else { "reject_once" };
    let outcome = choices
        .iter()
        .find(|c| c["kind"] == kind)
        .and_then(|c| c["optionId"].as_str())
        .map_or_else(
            || json!({"outcome":"cancelled"}),
            |option| json!({"outcome":"selected","optionId":option}),
        );
    write(
        stdin,
        json!({"jsonrpc":"2.0","id":message["id"],"result":{"outcome":outcome}}),
    )
    .await
}

fn resume_session<'a>(resume: Option<&'a str>, instance: &str) -> io::Result<Option<&'a str>> {
    resume.map(|id| id.strip_prefix(&format!("acp:{instance}:"))
        .filter(|session| !session.is_empty())
        .ok_or_else(|| io::Error::other("The saved ACP session belongs to another instance. Start a fresh session explicitly before switching ACP agents."))).transpose()
}

fn events(update: &Value) -> Vec<QueryEvent> {
    let text = update["content"]["text"].as_str().unwrap_or("").to_string();
    match update["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => vec![QueryEvent::Text(text)],
        Some("agent_thought_chunk") => vec![QueryEvent::Thinking(text)],
        Some("tool_call") => vec![QueryEvent::ToolCall {
            id: update["toolCallId"].as_str().unwrap_or("").into(),
            tool: update["title"].as_str().unwrap_or("ACP tool").into(),
            input: update["rawInput"].clone(),
        }],
        Some("tool_call_update") => {
            let status = update["status"].as_str().unwrap_or("");
            let is_terminal = matches!(status, "completed" | "failed" | "cancelled")
                || (status.is_empty() && !update["rawOutput"].is_null());
            if is_terminal {
                vec![QueryEvent::ToolResult {
                    tool_use_id: update["toolCallId"].as_str().unwrap_or("").into(),
                    content: match &update["rawOutput"] {
                        Value::String(text) => text.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    },
                    is_error: status == "failed",
                }]
            } else {
                vec![QueryEvent::Raw(
                    json!({"type":"acp_update","update":update}),
                )]
            }
        }
        _ => vec![QueryEvent::Raw(
            json!({"type":"acp_update","update":update}),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    async fn stalled_rpc_input() -> (
        process::CliProcess,
        ChildStdin,
        TurnContext,
        tokio::sync::mpsc::Receiver<crate::OrchestratorEvent>,
    ) {
        let sessions = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        let registry = std::sync::Arc::new(crate::BackgroundTaskRegistry::new());
        let thread = bruhs_models::ids::ThreadId::new_v4();
        let (turn, events) = crate::agent::tests::turn_context(&sessions, &registry, &thread, 1);
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "printf 'ready\\n'; exec 1>&-; /bin/sleep 30"])
            .env_clear()
            .stdin(std::process::Stdio::piped());
        let mut child =
            process::CliProcess::spawn("ACP stalled-input fixture", &mut command).unwrap();
        let input = child.take_stdin().unwrap();
        assert_eq!(
            child
                .next_line(&turn.tx)
                .await
                .unwrap_or_else(|_| panic!("fixture did not start")),
            "ready"
        );
        // stdout is already closed. A pending RPC with this large payload
        // cannot be waiting for a response: it has not finished its write.
        assert!(matches!(
            child.next_line(&turn.tx).await,
            Err(process::Ending::Eof)
        ));
        (child, input, turn, events)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_interrupts_a_blocked_rpc_write() {
        for method in ["session/prompt", "authenticate"] {
            let (mut child, mut input, turn, events) = stalled_rpc_input().await;
            tokio::time::pause();
            let mut request = Box::pin(rpc(
                &mut child,
                &mut input,
                &turn,
                PermissionMode::Plan,
                1,
                method,
                json!({"text":"x".repeat(1024 * 1024)}),
            ));
            let pending = std::future::poll_fn(|cx| {
                std::task::Poll::Ready(std::future::Future::poll(request.as_mut(), cx).is_pending())
            })
            .await;
            tokio::time::advance(super::super::CONFIG_TIMEOUT).await;
            let still_pending = std::future::poll_fn(|cx| {
                std::task::Poll::Ready(std::future::Future::poll(request.as_mut(), cx).is_pending())
            })
            .await;
            drop(events);
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(1), request.as_mut()).await;
            drop(request);
            drop(input);
            tokio::time::resume();
            // Always kill/reap the owned fixture before asserting the result.
            let cleanup = child.finish(process::Ending::ConsumerGone).await;
            assert!(cleanup.is_none());
            assert!(
                still_pending,
                "{method} timed out while waiting for the user"
            );
            assert!(pending, "the pipe did not exercise write backpressure");
            assert!(
                matches!(result, Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted),
                "cancellation did not interrupt the RPC write"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn configuration_timeout_covers_a_blocked_rpc_write() {
        let (mut child, mut input, turn, events) = stalled_rpc_input().await;
        tokio::time::pause();
        let mut request = Box::pin(rpc(
            &mut child,
            &mut input,
            &turn,
            PermissionMode::Plan,
            1,
            "initialize",
            json!({"metadata":"x".repeat(1024 * 1024)}),
        ));
        let pending = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(request.as_mut(), cx).is_pending())
        })
        .await;
        tokio::time::advance(super::super::CONFIG_TIMEOUT).await;
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(1), request.as_mut()).await;
        drop(request);
        drop(input);
        tokio::time::resume();
        let cleanup = child.finish(process::Ending::ConsumerGone).await;
        drop(events);
        assert!(cleanup.is_none());
        assert!(pending, "the pipe did not exercise write backpressure");
        assert!(
            matches!(result, Ok(Err(error)) if error.to_string().contains("ACP initialize timed out")),
            "the configuration deadline did not include the RPC write"
        );
    }

    #[tokio::test]
    #[ignore = "requires installed OpenCode; creates an isolated local ACP session without a prompt or provider turn"]
    async fn installed_opencode_negotiates_modes_and_reload_without_a_model_turn() {
        let directory = tempfile::tempdir().unwrap();
        let sessions = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        let registry = std::sync::Arc::new(crate::BackgroundTaskRegistry::new());
        let thread = bruhs_models::ids::ThreadId::new_v4();
        let (turn, _events) = crate::agent::tests::turn_context(&sessions, &registry, &thread, 1);
        let mut command = tokio::process::Command::new("opencode");
        command
            .args(["--pure", "acp", "--cwd"])
            .arg(directory.path())
            .current_dir(directory.path())
            .stdin(std::process::Stdio::piped())
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("XDG_CACHE_HOME", directory.path().join("cache"))
            .env("XDG_STATE_HOME", directory.path().join("state"));
        process::apply_env(
            &mut command,
            true,
            &crate::connections::PRIVATE_ENV.map(String::from),
            &[],
        );
        let mut child =
            process::CliProcess::spawn("OpenCode ACP acceptance", &mut command).unwrap();
        let mut input = child.take_stdin().unwrap();
        let initialized = rpc(&mut child, &mut input, &turn, PermissionMode::Plan, 1, "initialize", json!({"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"clientInfo":{"name":"bruhs-acceptance","version":env!("CARGO_PKG_VERSION")}})).await.unwrap();
        assert_eq!(initialized["protocolVersion"], 1);
        let session = rpc(
            &mut child,
            &mut input,
            &turn,
            PermissionMode::Plan,
            2,
            "session/new",
            json!({"cwd":directory.path(),"mcpServers":[]}),
        )
        .await
        .unwrap();
        let id = session["sessionId"].as_str().unwrap().to_owned();
        let selection = native_selection(&session, &id, Selector::Mode, "mode", "plan").unwrap();
        let updated = rpc(
            &mut child,
            &mut input,
            &turn,
            PermissionMode::Plan,
            3,
            selection.method,
            selection.params,
        )
        .await
        .unwrap();
        updated_configuration(session, updated, selection.expected).unwrap();
        if initialized["agentCapabilities"]["loadSession"] == true {
            rpc(
                &mut child,
                &mut input,
                &turn,
                PermissionMode::Plan,
                4,
                "session/load",
                json!({"sessionId":id,"cwd":directory.path(),"mcpServers":[]}),
            )
            .await
            .unwrap();
        }
        let failure = child.finish(process::Ending::ConsumerGone).await;
        assert!(failure.is_none());
    }

    #[test]
    fn registry_auth_chooses_agent_managed_methods_and_checks_the_preference() {
        let initialized = json!({"authMethods":[{"id":"browser","type":"terminal"},{"id":"personal"},{"id":"work","type":"agent"}]});
        assert_eq!(
            agent_auth_method(&initialized, None).unwrap(),
            Some("personal")
        );
        assert_eq!(
            agent_auth_method(&initialized, Some("work")).unwrap(),
            Some("work")
        );
        assert!(agent_auth_method(&initialized, Some("browser")).is_err());
        assert!(agent_auth_method(&initialized, Some("missing")).is_err());
        assert_eq!(
            agent_auth_method(&json!({}), Some("legacy")).unwrap(),
            Some("legacy")
        );
        assert_eq!(
            agent_auth_method(&json!({"authMethods":[]}), None).unwrap(),
            None
        );
    }

    #[test]
    fn config_selection_validates_groups_and_requires_the_native_echo() {
        let session = json!({"configOptions":[{"id":"access","type":"select","currentValue":"build","options":[{"value":"read-only","name":"Read"}]},{"id":"engine","type":"select","currentValue":"first","options":[{"group":"models","options":[{"value":"second","name":"Second"}]}]}]});
        let selected =
            native_selection(&session, "session", Selector::Model, "engine", "second").unwrap();
        assert_eq!(selected.method, "session/set_config_option");
        assert_eq!(selected.params["configId"], "engine");
        assert!(
            native_selection(&session, "session", Selector::Mode, "mode", "read-only").is_err()
        );
        assert!(
            native_selection(&session, "session", Selector::Mode, "access", "unknown").is_err()
        );
        assert!(updated_configuration(session.clone(), json!({}), selected.expected).is_err());
        assert!(updated_configuration(
            session,
            json!({"configOptions":[{"id":"access","currentValue":"build"}]}),
            Some(("access".into(), "read-only".into()))
        )
        .is_err());
    }
    #[test]
    fn switching_acp_instances_requires_an_explicit_fresh_session() {
        assert_eq!(resume_session(None, "second").unwrap(), None);
        assert_eq!(
            resume_session(Some("acp:first:native"), "first").unwrap(),
            Some("native")
        );
        assert!(resume_session(Some("acp:first:native"), "second").is_err());
        assert!(resume_session(Some("acp:first:"), "first").is_err());
    }
    #[test]
    fn maps_text_thought_and_tools_without_treating_metadata_as_text() {
        assert!(
            matches!(&events(&json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}))[0],QueryEvent::Text(t) if t=="hello")
        );
        assert!(matches!(
            &events(
                &json!({"sessionUpdate":"tool_call_update","toolCallId":"a","status":"failed","rawOutput":"no"})
            )[0],
            QueryEvent::ToolResult { is_error: true, .. }
        ));
        assert!(matches!(
            &events(&json!({"sessionUpdate":"plan","entries":[]}))[0],
            QueryEvent::Raw(_)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn registry_override_authenticates_only_when_challenged_and_retries_once() {
        use bruhs_models::agent_extensions::ExtensionConfig;
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("registry.py");
        std::fs::write(&script, r#"
import json, sys
scenario = sys.argv[1]
authenticated = False
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    with open("calls", "a") as calls: print(method, file=calls)
    response = {"jsonrpc":"2.0", "id":request["id"]}
    if method == "initialize":
        response["result"] = {"protocolVersion":1, "authMethods":[{"id":"browser","type":"terminal"},{"id":"device","type":"agent"}]}
    elif method == "session/new":
        if scenario == "other-error" or scenario == "repeated" or (scenario != "ready" and not authenticated):
            response["error"] = {"code":-32001 if scenario == "other-error" else -32000, "message":"fixture challenge"}
        else: response["result"] = {"sessionId":"session"}
    elif method == "authenticate":
        assert request["params"] == {"methodId":"device"}
        authenticated = True
        response["result"] = {}
    elif method == "session/prompt": response["result"] = {"stopReason":"end_turn"}
    else: raise AssertionError(method)
    print(json.dumps(response), flush=True)
"#).unwrap();
        let calls = directory.path().join("calls");
        for (scenario, preferred, expected) in [
            ("ready", "device", "initialize session/new session/prompt"),
            (
                "local",
                "",
                "initialize session/new authenticate session/new session/prompt",
            ),
            (
                "challenge",
                "",
                "initialize session/new authenticate session/new session/prompt",
            ),
            (
                "challenge",
                "device",
                "initialize session/new authenticate session/new session/prompt",
            ),
            ("challenge", "browser", "initialize session/new"),
            ("other-error", "device", "initialize session/new"),
            (
                "repeated",
                "device",
                "initialize session/new authenticate session/new",
            ),
        ] {
            std::fs::write(&calls, "").unwrap();
            let mut instance: AcpInstance = serde_json::from_value(json!({
                "id":"test","name":"Registry fixture","enabled":true,"command":"missing-managed-launcher",
                "args":[script,scenario],
                "registry":{"agent_id":"fixture","version":"1","distribution":"npm","executable_override":"python3"}
            })).unwrap();
            instance.registry.as_mut().unwrap().auth_method_id =
                (!preferred.is_empty()).then(|| preferred.to_owned());
            if scenario == "local" {
                instance.registry = None;
                instance.command = "python3".into();
            }
            let config = ExtensionConfig {
                acp: vec![instance],
                ..Default::default()
            };
            let sessions = Default::default();
            let registry = std::sync::Arc::new(crate::BackgroundTaskRegistry::new());
            let (turn, _events) = crate::agent::tests::turn_context(
                &sessions,
                &registry,
                &bruhs_models::ids::ThreadId::new_v4(),
                1,
            );
            let options = Options::new(
                bruhs_models::agent::AgentProvider::Acp,
                "hello",
                directory.path().to_str().unwrap(),
                "acp/test/default",
                PermissionMode::BypassPermissions,
                None,
                None,
            );
            let result = run_config(&options, &turn, &config).await;
            assert_eq!(
                result.is_ok(),
                expected.ends_with("session/prompt"),
                "{scenario}: {result:?}"
            );
            assert_eq!(
                std::fs::read_to_string(&calls)
                    .unwrap()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                expected
            );
        }
    }

    #[tokio::test]
    async fn modern_config_applies_model_then_confirms_access_before_the_prompt() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("config-agent.py");
        std::fs::write(&script,r#"import json,sys
stage=0
options=[{'id':'engine','type':'select','currentValue':'first','options':[{'value':'first'},{'value':'second'}]},{'id':'access','type':'select','currentValue':'build','options':[{'value':'build'},{'value':'read-only'}]}]
for line in sys.stdin:
 m=json.loads(line);method=m['method']
 if method=='initialize':result={'protocolVersion':1}
 elif method=='session/new':result={'sessionId':'config-session','configOptions':options}
 elif method=='session/set_config_option':
  p=m['params']
  if p['configId']=='engine':
   assert stage==0 and p['value']=='second';stage=1
   options[0]['currentValue']='second';options[1]['currentValue']='build'
  elif p['configId']=='access':
   assert stage==1 and p['value']=='read-only';stage=2
   options[1]['currentValue']='read-only'
  result={'configOptions':options}
 elif method=='session/prompt':
  assert stage==2 and options[1]['currentValue']=='read-only'
  print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'config-session','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'access confirmed'}}}}),flush=True)
  result={'stopReason':'end_turn'}
 else:raise RuntimeError('unexpected method '+method)
 print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':result}),flush=True)
"#).unwrap();
        let config = bruhs_models::agent_extensions::ExtensionConfig {
            acp: vec![AcpInstance {
                driver: None,
                registry: None,
                accent_color: None,
                environment: Default::default(),
                id: "modern".into(),
                name: "Modern fixture".into(),
                command: "python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                models: vec!["second".into()],
                modes: std::collections::BTreeMap::from([("plan".into(), "read-only".into())]),
                mode_config_id: Some("access".into()),
                model_config_id: Some("engine".into()),
                enabled: true,
            }],
            ..Default::default()
        };
        let sessions = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        let registry = std::sync::Arc::new(crate::BackgroundTaskRegistry::new());
        let thread = bruhs_models::ids::ThreadId::new_v4();
        let (turn, mut events) =
            crate::agent::tests::turn_context(&sessions, &registry, &thread, 1);
        let options = Options::new(
            bruhs_models::agent::AgentProvider::Acp,
            "verify access",
            directory.path().to_str().unwrap(),
            "acp/modern/second",
            PermissionMode::Plan,
            None,
            None,
        );
        let consumer = tokio::spawn(async move {
            let mut text = String::new();
            while let Some(event) = events.recv().await {
                match event {
                    crate::OrchestratorEvent::Query(QueryEvent::Text(chunk)) => {
                        text.push_str(&chunk)
                    }
                    crate::OrchestratorEvent::Query(QueryEvent::Result { .. }) => break,
                    _ => {}
                }
            }
            text
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_config(&options, &turn, &config),
        )
        .await
        .unwrap()
        .unwrap();
        drop(turn);
        assert_eq!(consumer.await.unwrap(), "access confirmed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn negotiates_real_rpc_streams_and_relays_owner_permission() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("agent.py");
        std::fs::write(&script,r#"import json,sys
def send(message):
 print(json.dumps(message),flush=True)
for line in sys.stdin:
 m=json.loads(line)
 if m.get('method')=='initialize': result={'protocolVersion':1,'agentCapabilities':{'loadSession':True}}
 elif m.get('method')=='session/new': result={'sessionId':'fixture-session','modes':{'availableModes':[{'id':'review','name':'Review'}]}}
 elif m.get('method')=='session/set_mode':
  assert m['params']['modeId']=='review'
  result={}
 elif m.get('method')=='session/prompt':
  send({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'fixture-session','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'hello'}}}})
  send({'jsonrpc':'2.0','id':'permission','method':'session/request_permission','params':{'toolCall':{'toolCallId':'tool','title':'Write file','rawInput':{'path':'sample'}},'options':[{'optionId':'allow','kind':'allow_once'},{'optionId':'deny','kind':'reject_once'}]}})
  reply=json.loads(sys.stdin.readline())
  assert reply['result']['outcome']=={'outcome':'selected','optionId':'allow'}
  result={'stopReason':'end_turn'}
 else: result={}
 send({'jsonrpc':'2.0','id':m['id'],'result':result})
"#).unwrap();
        let config = bruhs_models::agent_extensions::ExtensionConfig {
            acp: vec![AcpInstance {
                driver: None,
                registry: None,
                accent_color: None,
                environment: Default::default(),
                id: "fixture".into(),
                name: "Fixture".into(),
                command: "python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                models: vec!["default".into()],
                modes: std::collections::BTreeMap::from([("default".into(), "review".into())]),
                mode_config_id: None,
                model_config_id: None,
                enabled: true,
            }],
            ..Default::default()
        };
        let sessions = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        let registry = std::sync::Arc::new(crate::BackgroundTaskRegistry::new());
        let thread = bruhs_models::ids::ThreadId::new_v4();
        let (turn, mut events) =
            crate::agent::tests::turn_context(&sessions, &registry, &thread, 1);
        let options = Options::new(
            bruhs_models::agent::AgentProvider::Acp,
            "hello",
            directory.path().to_str().unwrap(),
            "acp/fixture/default",
            PermissionMode::Default,
            None,
            None,
        );
        let consumer = tokio::spawn(async move {
            let mut text = String::new();
            let mut approved = false;
            let mut complete = false;
            while let Some(event) = events.recv().await {
                if let crate::OrchestratorEvent::Query(event) = event {
                    match event {
                        QueryEvent::Text(t) => text.push_str(&t),
                        QueryEvent::PermissionRequest { id, .. } => {
                            approved = resolve(&id, &PermissionDecision::Allow);
                        }
                        QueryEvent::Result { session_id, .. } => {
                            complete = session_id == "acp:fixture:fixture-session";
                            break;
                        }
                        _ => {}
                    }
                }
            }
            (text, approved, complete)
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            run_config(&options, &turn, &config),
        )
        .await
        .unwrap();
        drop(turn);
        result.unwrap();
        let (text, approved, complete) = consumer.await.unwrap();
        assert_eq!((text.as_str(), approved, complete), ("hello", true, true));
    }
}
