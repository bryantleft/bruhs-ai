use axum::{
    extract::{rejection::JsonRejection, State},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    Json,
};
use bruhs_core::{write_paste_files, AgentTurn, AppState, OrchestratorEvent, QueryEvent};
use bruhs_models::agent::{ModelId, PermissionMode};
use bruhs_models::ids::ThreadId;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt;

use crate::error::AppError;
use crate::routes::paths::{HomePath, HomePathError};

/// `POST /api/agent`. Deserializing validates the thread id, the model id
/// and the permission mode, so a malformed one is a 400 before anything
/// runs; the project path must resolve inside the home directory.
#[derive(Deserialize)]
pub struct AgentRequest {
    pub prompt: String,
    /// Missing (a chat without a project) runs the turn in the home
    /// directory. Clients used to send `"."`, which resolved against the
    /// server's own working directory.
    #[serde(default)]
    pub project_path: Option<String>,
    pub thread_id: ThreadId,
    /// Missing means [`bruhs_models::agent::DEFAULT_AGENT_MODEL`].
    #[serde(default)]
    pub model: Option<ModelId>,
    /// Missing means [`PermissionMode::Default`] (ask first); an unknown
    /// value is rejected.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Provider session to resume (from a previous turn's `init`/`result`).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Forget the session this thread would resume and start a new one.
    #[serde(default)]
    pub fresh_session: bool,
    /// Pasted text written to temp files before the turn. The prompt names them.
    #[serde(default)]
    pub text_attachments: Vec<bruhs_models::logic::composer_text::TextAttachment>,
    /// Files already stored by `POST /api/agent/uploads`.
    #[serde(default)]
    pub file_attachments: Vec<bruhs_models::logic::attachments::UploadedFile>,
    /// Claude `--effort`. Missing leaves the CLI default. Codex ignores it.
    #[serde(default)]
    pub effort: Option<String>,
    /// Claude `--autocompact`. Missing omits the flag. Codex ignores it.
    #[serde(default)]
    pub autocompact: Option<String>,
    /// What the agent may do with other threads. Missing means list, read
    /// and message; an unknown value is rejected.
    #[serde(default)]
    pub thread_access: Option<bruhs_models::thread_link::ThreadAccess>,
    /// The thread's role instructions. Missing or blank adds nothing; text
    /// longer than a role may hold is rejected.
    #[serde(default)]
    pub instructions: Option<String>,
}

/// Convert a `QueryEvent` into a `serde_json::Value`.
fn query_event_to_json(event: &QueryEvent) -> Value {
    match event {
        QueryEvent::SecretRequest(request) => {
            json!({"type": "secret_request", "id": request.id, "thread_id": request.thread_id, "operation": request.operation, "cwd": request.cwd, "saved_available": request.saved_available, "storage_unavailable": request.storage_unavailable})
        }
        QueryEvent::SecretResolved { id } => json!({"type": "secret_resolved", "id": id}),
        QueryEvent::ThreadRequest(request) => json!({
            "type": "thread_request",
            "id": request.id,
            "thread_id": request.thread_id,
            "operation": request.operation,
        }),
        QueryEvent::Init {
            session_id,
            model,
            tools,
            mcp_servers,
        } => json!({
            "type": "init",
            "session_id": session_id,
            "model": model,
            "tools": tools,
            "mcp_servers": mcp_servers,
        }),
        QueryEvent::Text(text) => json!({ "type": "text", "text": text }),
        QueryEvent::Thinking(text) => json!({ "type": "thinking", "thinking": text }),
        QueryEvent::ToolCall { id, tool, input } => json!({
            "type": "tool_call",
            "id": id,
            "tool": tool,
            "input": input,
        }),
        QueryEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
        QueryEvent::Usage(usage) => json!({
            "type": "usage",
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cache_read_input_tokens": usage.cache_read_input_tokens,
            "cache_creation_input_tokens": usage.cache_creation_input_tokens,
            "context_tokens": usage.context_tokens,
        }),
        QueryEvent::Result {
            result,
            session_id,
            duration_ms,
            usage,
            is_error,
        } => json!({
            "type": "result",
            "result": result,
            "session_id": session_id,
            "duration_ms": duration_ms,
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_input_tokens": usage.cache_read_input_tokens,
                "cache_creation_input_tokens": usage.cache_creation_input_tokens,
                "context_tokens": usage.context_tokens,
            },
            "is_error": is_error,
        }),
        QueryEvent::PermissionRequest {
            id,
            tool,
            input,
            tool_use_id,
        } => json!({
            "type": "permission_request",
            "id": id,
            "tool": tool,
            "input": input,
            "tool_use_id": tool_use_id,
        }),
        QueryEvent::PermissionCancelled { id } => json!({
            "type": "permission_cancelled",
            "id": id,
        }),
        QueryEvent::Error(msg) => json!({
            "type": "error",
            "message": msg,
        }),
        QueryEvent::Raw(value) => json!({
            "type": "raw",
            "data": value,
        }),
    }
}

/// Convert an `OrchestratorEvent` into a `serde_json::Value`.
fn orchestrator_event_to_json(event: &OrchestratorEvent) -> Value {
    match event {
        OrchestratorEvent::Query(query_event) => {
            let mut v = query_event_to_json(query_event);
            // Wrap with an envelope so the client knows this is a query event
            v["envelope"] = json!("query");
            v
        }
        OrchestratorEvent::BackgroundTaskRegistered(task) => {
            // BackgroundTask has Serialize
            json!({
                "envelope": "background_task_registered",
                "task": task,
            })
        }
    }
}

/// Where a turn runs: its project, or the home directory when it has none.
async fn turn_dir(
    project_path: Option<&str>,
    home: Option<&std::path::Path>,
) -> Result<HomePath, HomePathError> {
    HomePath::or_home_in(project_path, home).await
}

pub async fn post_agent(
    State(state): State<Arc<AppState>>,
    body: Result<Json<AgentRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    if body
        .instructions
        .as_deref()
        .is_some_and(|text| text.chars().count() > bruhs_models::roles::MAX_INSTRUCTIONS_CHARS)
    {
        return Err(AppError::BadRequest(
            "role instructions are too long".into(),
        ));
    }
    crate::routes::uploads::reject_foreign_uploads(&body.file_attachments)?;
    let project = turn_dir(body.project_path.as_deref(), home::home_dir().as_deref()).await?;
    let model = body.model.unwrap_or_else(ModelId::default_agent_model);
    // The same routing the pickers use, so the UI and the server agree on
    // which CLI serves a model.
    let provider = model.provider();
    let files = write_paste_files(&body.text_attachments);
    let prompt = bruhs_models::logic::composer_text::agent_prompt(&body.prompt, &files);
    let prompt =
        bruhs_models::logic::attachments::append_uploaded_files(&prompt, &body.file_attachments);
    let mut turn = AgentTurn::new(
        body.thread_id,
        prompt,
        project.as_path().to_string_lossy(),
        model.into_inner(),
        provider,
        body.permission_mode.unwrap_or_default(),
    )
    .resume(body.session_id)
    .effort(body.effort.as_deref())
    .autocompact(body.autocompact.as_deref())
    .thread_access(body.thread_access.unwrap_or_default())
    .instructions(body.instructions)
    // A chat with no project (life surfaces, meetings) runs in the home
    // directory and needs no coding plugins.
    .lean(project.is_home());
    if body.fresh_session {
        turn = turn.fresh();
    }

    let event_stream = state.send_message(turn).await?;

    // Map each OrchestratorEvent into an SSE Event
    let sse_stream = event_stream.map(|orchestrator_event| {
        let json_str = orchestrator_event_to_json(&orchestrator_event).to_string();
        let sse_event = Event::default().data(json_str);
        Ok::<Event, Infallible>(sse_event)
    });

    // Idle proxies drop long agent turns without periodic bytes; the tasks
    // stream already keeps itself alive the same way.
    Ok(Sse::new(sse_stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

/// `POST /api/agent/second-opinion`. Runs the other provider read-only on
/// the thread's last answer and returns its review. A failed review is a
/// 200 carrying `error`, so the client shows it like any other notice.
pub async fn post_second_opinion(
    body: Result<Json<bruhs_models::api::SecondOpinionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    if body.answer.trim().is_empty() {
        return Err(AppError::BadRequest(
            "a second opinion needs an answer".into(),
        ));
    }
    let project = turn_dir(body.project_path.as_deref(), home::home_dir().as_deref()).await?;
    let reviewer = bruhs_models::logic::handoff::reviewer_for(body.thread_provider);
    // The other provider reviews; it cannot run this provider's models.
    if body
        .reviewer_model
        .as_ref()
        .is_some_and(|model| model.provider() != reviewer)
    {
        return Err(AppError::BadRequest(
            "the reviewer model belongs to the other provider".into(),
        ));
    }
    let result =
        bruhs_core::second_opinion::second_opinion(bruhs_core::second_opinion::ReviewRequest {
            thread_provider: body.thread_provider,
            project_path: project.as_path().to_string_lossy().into_owned(),
            task: body.task,
            answer: body.answer,
            focus: body.focus,
            reviewer_model: body
                .reviewer_model
                .map(bruhs_models::agent::ModelId::into_inner),
        })
        .await;
    Ok(Json(match result {
        Ok(review) => bruhs_models::api::SecondOpinionResponse {
            reviewer,
            review: review.text,
            error: None,
            usage: Some(bruhs_models::api::UsageInfo {
                input_tokens: review.usage.input_tokens,
                output_tokens: review.usage.output_tokens,
                cache_read_input_tokens: review.usage.cache_read_input_tokens,
                cache_creation_input_tokens: review.usage.cache_creation_input_tokens,
                context_tokens: review.usage.context_tokens,
            }),
        },
        Err(error) => bruhs_models::api::SecondOpinionResponse {
            reviewer,
            review: String::new(),
            error: Some(error.to_string()),
            usage: None,
        },
    }))
}

/// `POST /api/agent/thread-reply`. The web app's answer to a
/// `thread_request` event: it owns the threads, so it lists, reads and
/// queues; this only hands the answer back to the waiting tool call.
pub async fn post_thread_reply(
    State(state): State<Arc<AppState>>,
    body: Result<Json<bruhs_models::thread_link::ThreadReplySubmission>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    state
        .thread_links()
        .resolve(&body.thread_id, &body.request_id, body.reply)
        .map_err(|error| AppError::NotFound(error.to_string()))?;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/agent/steer`. Writes one user message into the running Claude turn.
pub async fn post_steer(
    State(state): State<Arc<AppState>>,
    body: Result<Json<bruhs_models::api::SteerRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    let prompt = body.prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::BadRequest("a steer needs a message".into()));
    }
    state.steer(&body.thread_id, prompt).await?;
    Ok(Json(json!({ "success": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Server(tokio::task::JoinHandle<()>);

    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn serve(state: Arc<AppState>) -> (Server, String) {
        let app = axum::Router::new()
            .route("/api/agent", axum::routing::post(post_agent))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api/agent", listener.local_addr().unwrap());
        let server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        (server, url)
    }

    /// A working directory that does not exist, so no request in these
    /// tests can start a real CLI.
    fn missing_dir() -> String {
        std::env::temp_dir()
            .join(format!("bruhs-missing-{}", ThreadId::new_v4()))
            .to_string_lossy()
            .into_owned()
    }

    fn body(thread_id: &str, permission_mode: Option<&str>) -> Value {
        json!({
            "prompt": "hi",
            "project_path": missing_dir(),
            "thread_id": thread_id,
            "model": "claude-sonnet-4-6",
            "permission_mode": permission_mode,
        })
    }

    async fn post(url: &str, body: &Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(url)
            .json(body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn unknown_permission_modes_are_rejected() {
        let (_server, url) = serve(Arc::new(AppState::new())).await;
        let thread = ThreadId::new_v4();
        for mode in ["typo", "bypass", "DEFAULT", ""] {
            let response = post(&url, &body(thread.as_str(), Some(mode))).await;
            assert_eq!(
                response.status(),
                reqwest::StatusCode::BAD_REQUEST,
                "{mode}"
            );
            let error: Value = response.json().await.unwrap();
            assert!(error["error"].as_str().unwrap().contains("permission_mode"));
        }
    }

    #[tokio::test]
    async fn malformed_thread_ids_are_rejected() {
        let (_server, url) = serve(Arc::new(AppState::new())).await;
        for thread_id in ["", "../tasks", "a/b", "x?y"] {
            let response = post(&url, &body(thread_id, Some("plan"))).await;
            assert_eq!(
                response.status(),
                reqwest::StatusCode::BAD_REQUEST,
                "{thread_id:?}"
            );
        }
    }

    /// Role instructions are bounded before anything runs: a role holds at
    /// most `MAX_INSTRUCTIONS_CHARS`, so longer text did not come from one.
    #[tokio::test]
    async fn oversized_role_instructions_are_rejected() {
        let (_server, url) = serve(Arc::new(AppState::new())).await;
        let mut request = body(ThreadId::new_v4().as_str(), Some("plan"));
        request["instructions"] =
            json!("x".repeat(bruhs_models::roles::MAX_INSTRUCTIONS_CHARS + 1));
        let response = post(&url, &request).await;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let error: Value = response.json().await.unwrap();
        assert!(error["error"].as_str().unwrap().contains("instructions"));
    }

    /// The other provider reviews, so a reviewer model of the thread's own
    /// provider is refused rather than quietly swapped.
    #[tokio::test]
    async fn a_reviewer_model_of_the_wrong_provider_is_rejected() {
        use bruhs_models::agent::{AgentProvider, ModelId};
        for (thread_provider, model) in [
            (AgentProvider::Claude, "claude-sonnet-4-6"),
            (AgentProvider::Codex, "gpt-6.1-sol"),
        ] {
            let result = post_second_opinion(Ok(Json(bruhs_models::api::SecondOpinionRequest {
                project_path: None,
                thread_provider,
                task: "task".into(),
                answer: "answer".into(),
                focus: String::new(),
                reviewer_model: Some(ModelId::try_from(model.to_string()).unwrap()),
            })))
            .await;
            let Err(AppError::BadRequest(message)) = result.map(|_| ()) else {
                panic!("expected a bad request for {model}");
            };
            assert!(message.contains("other provider"), "{message}");
        }
    }

    #[tokio::test]
    async fn malformed_models_are_rejected() {
        let (_server, url) = serve(Arc::new(AppState::new())).await;
        for model in ["--dangerously-skip-permissions", "", "a b"] {
            let mut request = body(ThreadId::new_v4().as_str(), Some("plan"));
            request["model"] = json!(model);
            let response = post(&url, &request).await;
            assert_eq!(
                response.status(),
                reqwest::StatusCode::BAD_REQUEST,
                "{model:?}"
            );
            let error: Value = response.json().await.unwrap();
            assert!(
                error["error"].as_str().unwrap().contains("model"),
                "{error}"
            );
        }
    }

    /// The project path is confined to the home directory like every other
    /// file-system route (it used to reach the CLI unchecked).
    #[tokio::test]
    async fn project_paths_outside_home_or_missing_are_rejected() {
        let (_server, url) = serve(Arc::new(AppState::new())).await;
        for (path, status) in [
            ("/", reqwest::StatusCode::FORBIDDEN),
            ("/usr", reqwest::StatusCode::FORBIDDEN),
            ("", reqwest::StatusCode::BAD_REQUEST),
        ] {
            let mut request = body(ThreadId::new_v4().as_str(), Some("plan"));
            request["project_path"] = json!(path);
            assert_eq!(post(&url, &request).await.status(), status, "{path:?}");
        }
        // A missing directory never reaches the provider CLI.
        let missing = body(ThreadId::new_v4().as_str(), Some("plan"));
        assert_eq!(
            post(&url, &missing).await.status(),
            reqwest::StatusCode::BAD_REQUEST
        );
    }

    /// A chat without a project runs in the home directory, wherever the
    /// server was started (the web used to send `"."`, which resolved
    /// against the server's working directory and could be refused).
    #[tokio::test]
    async fn a_turn_without_a_project_runs_in_the_home_directory() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("project");
        std::fs::create_dir_all(&project).unwrap();

        let dir = turn_dir(None, Some(&home)).await.unwrap();
        assert!(dir.is_home());
        assert_eq!(dir.as_path(), home.canonicalize().unwrap());
        let dir = turn_dir(project.to_str(), Some(&home)).await.unwrap();
        assert_eq!(dir.as_path(), project.canonicalize().unwrap());
        // An explicit empty path is still a malformed request.
        assert!(matches!(
            turn_dir(Some(""), Some(&home)).await,
            Err(HomePathError::Missing)
        ));
        // An explicit request body without the field deserializes.
        let body: AgentRequest = serde_json::from_value(json!({
            "prompt": "hi",
            "thread_id": ThreadId::new_v4().as_str(),
        }))
        .unwrap();
        assert_eq!(body.project_path, None);
    }

    /// The stream JSON is built by hand here and read by web and desktop
    /// through the shared `AgentStreamMessage`; every event the server can
    /// send must parse to the typed event carrying the same values.
    #[test]
    fn every_stream_event_parses_as_the_shared_typed_message() {
        use bruhs_models::api::{
            AgentStreamEvent as Typed, AgentStreamMessage, McpServerInfo as TypedMcp,
            UsageInfo as TypedUsage,
        };

        let usage = bruhs_core::UsageInfo {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
            context_tokens: Some(8),
        };
        let typed_usage = TypedUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
            context_tokens: Some(8),
        };
        let cases = [
            (
                QueryEvent::SecretResolved {
                    id: "private-1".into(),
                },
                Typed::SecretResolved {
                    id: "private-1".into(),
                },
            ),
            {
                let request = bruhs_models::secrets::SecretRequest {
                    id: "private-1".into(),
                    thread_id: bruhs_models::ids::ThreadId::new_v4(),
                    operation: bruhs_models::secrets::SecretOperation {
                        name: "Deploy token".into(),
                        reason: "Deploy project".into(),
                        env_var: "DEPLOY_TOKEN".into(),
                        command: vec!["deploy".into()],
                    },
                    cwd: "/tmp/project".into(),
                    saved_available: false,
                    storage_unavailable: false,
                    connection: None,
                };
                (
                    QueryEvent::SecretRequest(request.clone()),
                    Typed::SecretRequest(request),
                )
            },
            {
                let request = bruhs_models::thread_link::ThreadRequest {
                    id: "ask-1".into(),
                    thread_id: bruhs_models::ids::ThreadId::new_v4(),
                    operation: bruhs_models::thread_link::ThreadOperation::Message {
                        thread_id: Some("abcd1234".into()),
                        title: None,
                        message: "Report back".into(),
                        wait: true,
                        role: None,
                    },
                };
                (
                    QueryEvent::ThreadRequest(request.clone()),
                    Typed::ThreadRequest(request),
                )
            },
            (
                QueryEvent::Init {
                    session_id: "s-1".into(),
                    model: Some("opus".into()),
                    tools: vec!["Bash".into()],
                    mcp_servers: vec![bruhs_core::McpServerInfo {
                        name: "linear".into(),
                        status: "connected".into(),
                    }],
                },
                Typed::Init {
                    session_id: "s-1".into(),
                    model: Some("opus".into()),
                    tools: vec!["Bash".into()],
                    mcp_servers: vec![TypedMcp {
                        name: "linear".into(),
                        status: "connected".into(),
                    }],
                },
            ),
            (
                QueryEvent::Init {
                    session_id: "s-2".into(),
                    model: None,
                    tools: Vec::new(),
                    mcp_servers: Vec::new(),
                },
                Typed::Init {
                    session_id: "s-2".into(),
                    model: None,
                    tools: Vec::new(),
                    mcp_servers: Vec::new(),
                },
            ),
            (
                QueryEvent::Text("hello".into()),
                Typed::Text {
                    text: "hello".into(),
                },
            ),
            (
                QueryEvent::Thinking("hmm".into()),
                Typed::Thinking {
                    thinking: "hmm".into(),
                },
            ),
            (
                QueryEvent::ToolCall {
                    id: "t-1".into(),
                    tool: "Bash".into(),
                    input: json!({ "command": "ls" }),
                },
                Typed::ToolCall {
                    id: "t-1".into(),
                    tool: "Bash".into(),
                    input: json!({ "command": "ls" }),
                },
            ),
            (
                QueryEvent::ToolResult {
                    tool_use_id: "t-1".into(),
                    content: "out".into(),
                    is_error: true,
                },
                Typed::ToolResult {
                    tool_use_id: "t-1".into(),
                    content: "out".into(),
                    is_error: true,
                },
            ),
            (QueryEvent::Usage(usage.clone()), Typed::Usage(typed_usage)),
            (
                QueryEvent::Result {
                    result: "done".into(),
                    session_id: "s-1".into(),
                    duration_ms: 42,
                    usage,
                    is_error: true,
                },
                Typed::Result {
                    result: "done".into(),
                    session_id: "s-1".into(),
                    duration_ms: 42,
                    usage: typed_usage,
                    is_error: true,
                },
            ),
            (
                QueryEvent::PermissionRequest {
                    id: "p-1".into(),
                    tool: "Edit".into(),
                    input: json!({ "file_path": "a.rs" }),
                    tool_use_id: Some("t-2".into()),
                },
                Typed::PermissionRequest {
                    id: "p-1".into(),
                    tool: "Edit".into(),
                    input: json!({ "file_path": "a.rs" }),
                    tool_use_id: Some("t-2".into()),
                },
            ),
            (
                QueryEvent::PermissionRequest {
                    id: "p-2".into(),
                    tool: "Edit".into(),
                    input: Value::Null,
                    tool_use_id: None,
                },
                Typed::PermissionRequest {
                    id: "p-2".into(),
                    tool: "Edit".into(),
                    input: Value::Null,
                    tool_use_id: None,
                },
            ),
            (
                QueryEvent::PermissionCancelled { id: "p-1".into() },
                Typed::PermissionCancelled { id: "p-1".into() },
            ),
            (
                QueryEvent::Error("boom".into()),
                Typed::Error {
                    message: "boom".into(),
                },
            ),
            (
                QueryEvent::Raw(json!({ "reconnect": 1 })),
                Typed::Raw {
                    data: json!({ "reconnect": 1 }),
                },
            ),
        ];
        // Every QueryEvent variant is covered: adding one fails to compile
        // here until it gets a case above.
        for (event, _) in &cases {
            match event {
                QueryEvent::SecretRequest(_)
                | QueryEvent::SecretResolved { .. }
                | QueryEvent::ThreadRequest(_)
                | QueryEvent::Init { .. }
                | QueryEvent::Text(_)
                | QueryEvent::Thinking(_)
                | QueryEvent::ToolCall { .. }
                | QueryEvent::ToolResult { .. }
                | QueryEvent::Usage(_)
                | QueryEvent::Result { .. }
                | QueryEvent::PermissionRequest { .. }
                | QueryEvent::PermissionCancelled { .. }
                | QueryEvent::Error(_)
                | QueryEvent::Raw(_) => {}
            }
        }
        for (event, expected) in cases {
            let wire = orchestrator_event_to_json(&OrchestratorEvent::Query(event));
            assert_eq!(wire["envelope"], "query", "{wire}");
            // Through the SSE text, as clients receive it.
            let parsed: AgentStreamMessage = serde_json::from_str(&wire.to_string()).unwrap();
            assert_eq!(parsed, AgentStreamMessage::Event(expected), "{wire}");
        }

        let task = bruhs_core::BackgroundTask {
            id: "task-1".into(),
            thread_id: ThreadId::new_v4().to_string(),
            parent_task_id: Some("parent".into()),
            kind: bruhs_core::TaskKind::Shell,
            label: "npm run dev".into(),
            status: bruhs_core::TaskStatus::Running,
            started_at: 1_000,
            ended_at: Some(2_000),
            exit_code: Some(0),
            meta: json!({ "command": "npm run dev" }),
            log_tail: "ready".into(),
        };
        let wire =
            orchestrator_event_to_json(&OrchestratorEvent::BackgroundTaskRegistered(task.clone()));
        let AgentStreamMessage::BackgroundTaskRegistered(parsed) =
            AgentStreamMessage::from_value(wire.clone())
        else {
            panic!("not a typed task: {wire}");
        };
        assert_eq!(parsed.id, task.id);
        assert_eq!(parsed.thread_id.as_str(), task.thread_id);
        assert_eq!(parsed.parent_task_id, task.parent_task_id);
        assert_eq!(parsed.kind, bruhs_models::api::TaskKind::Shell);
        assert_eq!(parsed.label, task.label);
        assert_eq!(parsed.status, bruhs_models::api::TaskStatus::Running);
        assert_eq!(parsed.started_at.as_millis(), 1_000);
        assert_eq!(parsed.ended_at.map(|at| at.as_millis()), Some(2_000));
        assert_eq!(parsed.exit_code, Some(0));
        assert_eq!(parsed.meta, task.meta);
        assert_eq!(parsed.log_tail, task.log_tail);
    }

    /// Core refuses a send while the thread streams (covered in
    /// bruhs-core); the route reports that as 409.
    #[test]
    fn a_send_to_a_busy_thread_is_a_conflict() {
        let error = AppError::from(bruhs_core::SendMessageError::ThreadBusy(ThreadId::new_v4()));
        assert_eq!(
            error.into_response().status(),
            axum::http::StatusCode::CONFLICT
        );
    }
}
