//! Typed client for the bruhs-server HTTP API. Every call returns
//! `Result<_, ApiError>`; the caller decides whether a failure is shown,
//! retried or degraded to an empty state. Request and response bodies come
//! from `bruhs_models::api` wherever the shared contract defines them.

use gloo_net::http::Method;
use serde::Deserialize;

pub use bruhs_models::api::{
    CaptureStartBody, CaptureStatus, CreateMeetingBody, StartOAuthResponse, StopMeetingBody,
};

mod agent_stream;
mod http;
mod people_sync;

pub use agent_stream::{send_agent_message_streaming, StreamError};
pub use http::ApiError;
use http::{decode, found, send_json, send_request, url};
pub(crate) use http::{get_json, segment, send};
pub use people_sync::{get_people, person_versions};

// The API origin is baked at build time. Override via env when building (e.g.
// `BRUHS_API_URL=https://api.bruhs.localhost trunk build`) to point the WASM
// bundle at portless named hosts; otherwise defaults to the local dev port.
// WASM has no runtime env, so this must be `option_env!`. The capture daemon
// is reached only through the server (`/api/capture/*`), which holds its token.
pub const BASE_URL: &str = match option_env!("BRUHS_API_URL") {
    Some(u) => u,
    None => "http://127.0.0.1:3001",
};

/// Private endpoint; this body must never go through the chat/steer APIs.
pub async fn submit_secret(
    body: &bruhs_models::secrets::SecretSubmission,
) -> Result<bool, ApiError> {
    #[derive(Deserialize)]
    struct Resolution {
        resolved: bool,
    }
    let result: Resolution = send_json(Method::POST, "/api/agent/secret", Some(body)).await?;
    Ok(result.resolved)
}

/// The app's answer to a `thread_request` stream event.
pub async fn submit_thread_reply(
    body: &bruhs_models::thread_link::ThreadReplySubmission,
) -> Result<(), ApiError> {
    let _: serde_json::Value =
        send_json(Method::POST, "/api/agent/thread-reply", Some(body)).await?;
    Ok(())
}

pub async fn saved_credentials() -> Result<Vec<bruhs_models::secrets::SavedCredential>, ApiError> {
    get_json("/api/credentials").await
}

pub async fn service_connection(
    provider: bruhs_models::connections::ServiceProvider,
) -> Result<bruhs_models::connections::ServiceConnection, ApiError> {
    get_json(&format!("/api/connections/{}", provider.id())).await
}
pub async fn connect_service(
    provider: bruhs_models::connections::ServiceProvider,
    login: &bruhs_models::connections::ConnectionLogin,
) -> Result<bruhs_models::connections::ConnectionStart, ApiError> {
    send_json(
        Method::POST,
        &format!("/api/connections/{}/start", provider.id()),
        Some(login),
    )
    .await
}
pub async fn connection_attempt(
    id: &str,
) -> Result<bruhs_models::connections::ConnectionAttempt, ApiError> {
    get_json(&format!("/api/connections/attempts/{}", segment(id))).await
}
pub async fn cancel_connection(id: &str) -> Result<(), ApiError> {
    send(
        Method::DELETE,
        &format!("/api/connections/attempts/{}", segment(id)),
        NO_BODY,
    )
    .await
    .map(drop)
}
pub async fn disconnect_service(
    provider: bruhs_models::connections::ServiceProvider,
) -> Result<(), ApiError> {
    send(
        Method::DELETE,
        &format!("/api/connections/{}", provider.id()),
        NO_BODY,
    )
    .await
    .map(drop)
}

pub async fn replace_credential(id: &str, value: String) -> Result<(), ApiError> {
    let body = bruhs_models::secrets::CredentialReplacement { value };
    send(
        Method::PUT,
        &format!("/api/credentials/{}", segment(id)),
        Some(&body),
    )
    .await
    .map(drop)
}

pub async fn delete_credential(id: &str) -> Result<(), ApiError> {
    send(
        Method::DELETE,
        &format!("/api/credentials/{}", segment(id)),
        NO_BODY,
    )
    .await
    .map(drop)
}

/// A body-less request (`None` with a concrete body type).
const NO_BODY: Option<&()> = None;

// ── Files, directories and editors ──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    #[serde(default, alias = "isDirectory")]
    pub is_dir: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct FilesResponse {
    #[serde(default)]
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListDirResponse {
    pub current: String,
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub dirs: Vec<DirEntry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LocalServer {
    pub port: u16,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub pid: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalServersResponse {
    #[serde(default)]
    servers: Vec<LocalServer>,
}

/// Open `path` (optionally at `line`) in the editor `editor`; the editor's
/// display label on success.
pub async fn open_in_editor(
    path: &str,
    editor: &str,
    line: Option<u32>,
) -> Result<String, ApiError> {
    #[derive(Deserialize)]
    struct Opened {
        editor: String,
    }
    let body = serde_json::json!({ "path": path, "editor": editor, "line": line });
    let opened: Opened = send_json(Method::POST, "/api/open-in-editor", Some(&body)).await?;
    Ok(opened.editor)
}

/// Local TCP listeners the Preview surface can offer.
pub async fn list_local_servers() -> Result<Vec<LocalServer>, ApiError> {
    let reply: LocalServersResponse = get_json("/api/preview/servers").await?;
    Ok(reply.servers)
}

/// Browse directories under `path` (or $HOME if `None`).
pub async fn list_dir(path: Option<&str>) -> Result<ListDirResponse, ApiError> {
    let body = serde_json::json!({ "path": path });
    send_json(Method::POST, "/api/agent/list-dir", Some(&body)).await
}

/// Files and folders in `project_path` matching `query`.
pub async fn list_files(
    query: &str,
    project_path: Option<&str>,
) -> Result<Vec<FileEntry>, ApiError> {
    let body = files_request(query, project_path);
    let reply: FilesResponse = send_json(Method::POST, "/api/agent/files", Some(&body)).await?;
    Ok(reply.files)
}

/// `POST /api/agent/files` body. A chat without a project omits `path`, so
/// the server searches the home directory rather than its own working
/// directory.
fn files_request(query: &str, project_path: Option<&str>) -> serde_json::Value {
    match project_path {
        Some(path) => serde_json::json!({ "path": path, "query": query }),
        None => serde_json::json!({ "query": query }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PullHit {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PullsResponse {
    #[serde(default)]
    pulls: Vec<PullHit>,
    #[serde(default)]
    error: Option<String>,
}

/// One directory of the Files surface. `dir` is relative to `path` and empty
/// at the project root.
pub async fn list_children(path: &str, dir: &str) -> Result<Vec<FileEntry>, ApiError> {
    let body = serde_json::json!({ "path": path, "dir": dir });
    let reply: FilesResponse =
        send_json(Method::POST, "/api/agent/files/children", Some(&body)).await?;
    Ok(reply.files)
}

#[derive(Debug, Clone, Deserialize)]
pub struct BranchPullLookup {
    pub signed_in: bool,
    #[serde(default)]
    pub pull: Option<PullHit>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The GitHub pull request for `branch` in the project at `path`.
/// `signed_in` is false when `gh` is missing or signed out.
pub async fn branch_pull(path: &str, branch: &str) -> Result<BranchPullLookup, ApiError> {
    let body = serde_json::json!({ "path": path, "branch": branch });
    send_json(Method::POST, "/api/agent/branch-pull", Some(&body)).await
}

/// Pull requests in the project at `project_path`. `error` is set when `gh`
/// is missing or the chat has no project. The list is empty in that case.
pub async fn list_pulls(
    query: &str,
    project_path: Option<&str>,
) -> Result<(Vec<PullHit>, Option<String>), ApiError> {
    let body = files_request(query, project_path);
    let reply: PullsResponse = send_json(Method::POST, "/api/agent/pulls", Some(&body)).await?;
    Ok((reply.pulls, reply.error))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SkillHit {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillsResponse {
    #[serde(default)]
    skills: Vec<SkillHit>,
    #[serde(default)]
    error: Option<String>,
}

/// Skills for `provider` (`"claude"` or `"codex"`). A chat without a project
/// still lists skills from the home directory.
pub async fn list_skills(
    query: &str,
    project_path: Option<&str>,
    provider: &str,
) -> Result<(Vec<SkillHit>, Option<String>), ApiError> {
    let mut body = files_request(query, project_path);
    if let Some(map) = body.as_object_mut() {
        map.insert("provider".to_string(), serde_json::json!(provider));
    }
    let reply: SkillsResponse = send_json(Method::POST, "/api/agent/skills", Some(&body)).await?;
    Ok((reply.skills, reply.error))
}

/// One file the viewer fetched. Media keeps `bytes`. Text and HTML keep `text`.
#[derive(Clone, Debug)]
pub struct OpenedPreview {
    pub name: String,
    pub path: String,
    pub kind: bruhs_models::logic::attachments::FileKind,
    pub mime: String,
    pub truncated: bool,
    pub text: Option<String>,
    pub bytes: Option<Vec<u8>>,
}

/// `POST /api/agent/uploads`. The body is the file bytes.
pub async fn upload_attachment(
    name: &str,
    bytes: &[u8],
) -> Result<bruhs_models::logic::attachments::UploadedFile, ApiError> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props)
        .map_err(|_| ApiError::Network("the browser could not package that file".into()))?;
    let route = format!("/api/agent/uploads?name={}", segment(name));
    let request = gloo_net::http::RequestBuilder::new(&url(&route))
        .method(Method::POST)
        .header("Content-Type", "application/octet-stream")
        .body(blob)?;
    decode(send_request(request).await?).await
}

/// `GET /api/agent/uploads/open`. A relative `path` needs `project`.
pub async fn open_attachment(path: &str, project: Option<&str>) -> Result<OpenedPreview, ApiError> {
    use bruhs_models::logic::attachments::{kind_for_name, upload_label, FileKind, OpenedText};

    let mut route = format!("/api/agent/uploads/open?path={}", segment(path));
    if let Some(project) = project.filter(|project| !project.is_empty()) {
        route.push_str("&project=");
        route.push_str(&segment(project));
    }
    let request = gloo_net::http::RequestBuilder::new(&url(&route))
        .method(Method::GET)
        .build()?;
    let response = send_request(request).await?;
    let content_type = response.headers().get("content-type").unwrap_or_default();
    if content_type.contains("application/json") {
        let opened: OpenedText = decode(response).await?;
        let text = if matches!(opened.kind, FileKind::Text | FileKind::Html) {
            Some(opened.text)
        } else {
            None
        };
        return Ok(OpenedPreview {
            name: opened.name,
            path: opened.path,
            kind: opened.kind,
            mime: opened.mime,
            truncated: opened.truncated,
            text,
            bytes: None,
        });
    }
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();
    let bytes = response.binary().await?;
    Ok(OpenedPreview {
        name: upload_label(path),
        path: path.to_string(),
        kind: kind_for_name(path),
        mime,
        truncated: false,
        text: None,
        bytes: Some(bytes),
    })
}

// ── Terminal sessions ───────────────────────────────────────────────────────

/// Server-side terminal session metadata (`bruhs_core::terminal::TerminalInfo`).
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct TerminalSessionInfo {
    pub id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub title: String,
    #[serde(default)]
    pub project_name: Option<String>,
}

#[derive(serde::Deserialize)]
struct TerminalSessionsResponse {
    sessions: Vec<TerminalSessionInfo>,
}

/// Live terminal sessions.
pub async fn list_terminal_sessions() -> Result<Vec<TerminalSessionInfo>, ApiError> {
    let reply: TerminalSessionsResponse = get_json("/api/terminal/sessions").await?;
    Ok(reply.sessions)
}

/// Spawn a login shell on the server. `cwd` of `None` means the home directory.
pub async fn create_terminal_session(
    cwd: Option<&str>,
    project_name: Option<&str>,
    cols: u32,
    rows: u32,
) -> Result<TerminalSessionInfo, ApiError> {
    let body = serde_json::json!({
        "cwd": cwd,
        "project_name": project_name,
        "cols": cols,
        "rows": rows,
    });
    send_json(Method::POST, "/api/terminal/sessions", Some(&body)).await
}

/// Kill a server-side terminal session. A 404 means it was already gone.
pub async fn delete_terminal_session(id: &str) -> Result<(), ApiError> {
    let route = format!("/api/terminal/sessions/{}", segment(id));
    send(Method::DELETE, &route, NO_BODY).await.map(drop)
}

// ── Git ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BranchesResponse {
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub current: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GitDiffResponse {
    #[serde(default)]
    pub diff: String,
    #[serde(default)]
    pub truncated: bool,
}

pub async fn list_git_branches(path: &str) -> Result<BranchesResponse, ApiError> {
    let body = serde_json::json!({ "path": path });
    send_json(Method::POST, "/api/agent/git-branches", Some(&body)).await
}

/// Branch / ahead-behind / dirty files for the project working tree.
pub async fn git_status(path: &str) -> Result<crate::state::GitStatus, ApiError> {
    let body = serde_json::json!({ "path": path });
    send_json(Method::POST, "/api/agent/git-status", Some(&body)).await
}

/// Unified diff of the working tree against HEAD (plus untracked files).
pub async fn git_diff(path: &str) -> Result<GitDiffResponse, ApiError> {
    let body = serde_json::json!({ "path": path });
    send_json(Method::POST, "/api/agent/git-diff", Some(&body)).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeCreated {
    pub path: String,
    pub branch: String,
}

/// Create a `bruhs/` worktree of `path` from `base`. The checkout stays put.
pub async fn create_worktree(
    path: &str,
    base: &str,
    branch: &str,
) -> Result<WorktreeCreated, ApiError> {
    let body = serde_json::json!({ "path": path, "base": base, "branch": branch });
    send_json(Method::POST, "/api/agent/worktree", Some(&body)).await
}

#[derive(Debug, Deserialize)]
struct SnapshotReply {
    tree: String,
}

/// Tree of a bruhs worktree, including the user's edits at send time.
pub async fn snapshot_worktree(path: &str) -> Result<String, ApiError> {
    let body = serde_json::json!({ "path": path });
    let reply: SnapshotReply =
        send_json(Method::POST, "/api/agent/worktree-snapshot", Some(&body)).await?;
    let tree = reply.tree.trim();
    if tree.len() == 40 && tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(tree.to_string())
    } else {
        Err(ApiError::Decode("snapshot was not a tree".into()))
    }
}

/// Put a bruhs worktree back to a tree from [`snapshot_worktree`].
pub async fn restore_worktree(path: &str, tree: &str) -> Result<(), ApiError> {
    let body = serde_json::json!({ "path": path, "tree": tree });
    send(Method::POST, "/api/agent/worktree-restore", Some(&body))
        .await
        .map(drop)
}

pub async fn checkout_git_branch(path: &str, branch: &str) -> Result<(), ApiError> {
    let body = serde_json::json!({ "path": path, "branch": branch });
    send(Method::POST, "/api/agent/git-checkout", Some(&body))
        .await
        .map(drop)
}

// ── Agent ───────────────────────────────────────────────────────────────────

/// Remove a finished background task from the server's registry.
pub async fn dismiss_task(task_id: &str) -> Result<(), ApiError> {
    let route = format!("/api/tasks/{}", segment(task_id));
    send(Method::DELETE, &route, NO_BODY).await.map(drop)
}

/// The user's answer to a `permission_request`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

/// Answer the permission request `request_id`.
pub async fn approve_permission(request_id: &str, decision: Decision) -> Result<(), ApiError> {
    let (approved, reason) = match decision {
        Decision::Allow => (true, None),
        Decision::Deny { reason } => (false, Some(reason)),
    };
    let body = bruhs_models::api::ApproveRequest {
        request_id: request_id.to_owned(),
        approved,
        reason,
    };
    let reply: bruhs_models::api::ApproveResponse =
        send_json(Method::POST, "/api/agent/approve", Some(&body)).await?;
    if reply.success {
        Ok(())
    } else {
        Err(ApiError::Decode("approve reply had success: false".into()))
    }
}

/// Ask the provider the thread does not use to review its last answer.
pub async fn second_opinion(
    request: &bruhs_models::api::SecondOpinionRequest,
) -> Result<bruhs_models::api::SecondOpinionResponse, ApiError> {
    send_json(Method::POST, "/api/agent/second-opinion", Some(request)).await
}

/// Write `prompt` into the Claude turn that is streaming for `thread_id`.
pub async fn steer_turn(thread_id: &str, prompt: &str) -> Result<(), ApiError> {
    let thread_id = bruhs_models::ids::ThreadId::try_from(thread_id)
        .map_err(|error| ApiError::Decode(error.to_string()))?;
    let body = bruhs_models::api::SteerRequest {
        thread_id,
        prompt: prompt.to_string(),
    };
    send(Method::POST, "/api/agent/steer", Some(&body))
        .await
        .map(drop)
}

// ── Updates ──────────────────────────────────────────────────────────────────

/// The provider CLI and app update report with the server's update runs.
/// `refresh` checks now instead of using the cache; `online` off reads
/// installed versions only.
pub async fn fetch_updates(
    refresh: bool,
    online: bool,
) -> Result<bruhs_models::updates::UpdatesReport, ApiError> {
    let route = format!(
        "/api/updates?refresh={}&online={}",
        u8::from(refresh),
        u8::from(online)
    );
    get_json(&route).await
}

/// Start `provider`'s installer on the server; the run comes back at once.
pub async fn update_provider(
    provider: bruhs_models::agent::AgentProvider,
) -> Result<bruhs_models::updates::ProviderRun, ApiError> {
    let route = format!("/api/updates/providers/{}", segment(provider.as_str()));
    send_json::<(), _>(Method::POST, &route, None).await
}

// ── Meetings ─────────────────────────────────────────────────────────────────

use crate::state::{Meeting, PersonProfile};

/// A JavaScript clock reading (`Date.now()`, a meeting's `started_at_ms`) as
/// the shared wire timestamp.
pub fn epoch_ms(ms: f64) -> bruhs_models::time::EpochMs {
    // Browser clocks are finite and in range; a value that is not reads as
    // the epoch instead of failing the request that carries it.
    bruhs_models::time::EpochMs::from_f64_floor(ms)
        .unwrap_or(bruhs_models::time::EpochMs::UNIX_EPOCH)
}

pub async fn fetch_meetings() -> Result<Vec<Meeting>, ApiError> {
    get_json("/api/meetings").await
}

/// `GET /api/meetings/{id}`; `Ok(None)` when the server has no such meeting.
pub async fn get_meeting(id: &str) -> Result<Option<Meeting>, ApiError> {
    found(get_json(&format!("/api/meetings/{}", segment(id))).await)
}

/// Fetch one authoritative profile without loading the entire contact directory.
pub async fn get_person(id: &str) -> Result<Option<PersonProfile>, ApiError> {
    let person: Option<PersonProfile> =
        found(get_json(&format!("/api/people/{}", segment(id))).await)?;
    match person {
        Some(person) if person.id != id => Err(ApiError::Decode(format!(
            "asked for contact {id}, got {}",
            person.id
        ))),
        person => Ok(person),
    }
}

/// Resolve only missing cached contacts; this endpoint never mutates the server.
pub async fn resolve_person_merges(
    ids: &[String],
) -> Result<Vec<bruhs_models::PersonMergeResolution>, ApiError> {
    let body = serde_json::json!({ "ids": ids });
    let resolved: Vec<bruhs_models::PersonMergeResolution> =
        send_json(Method::POST, "/api/people/resolve-merges", Some(&body)).await?;
    let mut seen = std::collections::HashSet::new();
    if resolved.len() > ids.len()
        || resolved.iter().any(|item| {
            !ids.contains(&item.source_id)
                || !seen.insert(&item.source_id)
                || item.source_id == item.target.id
                || item.target.id.is_empty()
        })
    {
        return Err(ApiError::Decode(
            "contact merge resolution returned invalid identities".into(),
        ));
    }
    Ok(resolved)
}

/// `POST /api/people` — persist a manually captured contact.
pub async fn create_person(person: &PersonProfile) -> Result<PersonProfile, ApiError> {
    let body = serde_json::json!({
        "id": person.id,
        "display_name": person.display_name,
        "created_at": person.created_at,
        "updated_at": person.updated_at,
    });
    send_json(Method::POST, "/api/people", Some(&body)).await
}

pub async fn create_meeting(body: &CreateMeetingBody) -> Result<Meeting, ApiError> {
    send_json(Method::POST, "/api/meetings", Some(body)).await
}

pub async fn stop_meeting(id: &str, body: &StopMeetingBody) -> Result<Meeting, ApiError> {
    let route = format!("/api/meetings/{}/stop", segment(id));
    send_json(Method::POST, &route, Some(body)).await
}

/// `PATCH /api/meetings/{id}/speakers/{speaker_id}` — rename a speaker.
pub async fn update_speaker_label(
    meeting_id: &str,
    speaker_id: &str,
    label: String,
) -> Result<Meeting, ApiError> {
    let route = format!(
        "/api/meetings/{}/speakers/{}",
        segment(meeting_id),
        segment(speaker_id)
    );
    let body = serde_json::json!({ "label": label });
    send_json(Method::PATCH, &route, Some(&body)).await
}

pub async fn generate_meeting_summary(
    id: &str,
    request_id: &str,
    extract_observations: bool,
    replace_observations: bool,
) -> Result<crate::state::MeetingSummary, ApiError> {
    let route = format!("/api/meetings/{}/summarize", segment(id));
    let body = bruhs_models::api::SummarizeRequest {
        request_id: Some(request_id.to_owned()),
        extract_observations,
        replace_observations,
    };
    send_json(Method::POST, &route, Some(&body)).await
}

/// The receipt of generation `request_id`; `Ok(None)` when the server never
/// saw it.
pub async fn summary_generation(
    id: &str,
    request_id: &str,
) -> Result<Option<bruhs_models::SummaryGenerationReceipt>, ApiError> {
    let route = format!(
        "/api/meetings/{}/generations/{}",
        segment(id),
        segment(request_id)
    );
    found(get_json(&route).await)
}

/// Discover an unresolved server request whose client kept no identity.
/// `Err` includes an unknown meeting; callers must not treat that as "none pending".
pub async fn pending_summary_generation(
    id: &str,
) -> Result<Option<bruhs_models::SummaryGenerationReceipt>, ApiError> {
    get_json(&format!("/api/meetings/{}/generations", segment(id))).await
}

/// Fence a saved request on the server, including one that never arrived.
/// `None` means the meeting is unknown there; the saved request must be retained.
pub async fn cancel_summary_generation(
    request: &bruhs_models::SummaryGenerationRequest,
) -> Result<Option<bruhs_models::SummaryGenerationReceipt>, ApiError> {
    let route = format!(
        "/api/meetings/{}/generations/{}",
        segment(&request.meeting_id),
        segment(&request.request_id)
    );
    let body = bruhs_models::api::CancelSummaryGenerationBody {
        extract_observations: request.extract_observations,
        replace_observations: request.replace_observations,
    };
    found(send_json(Method::DELETE, &route, Some(&body)).await)
}

/// The message the summary recovery region shows for a failed summary call:
/// the server's own message when it sent a short one, never its raw body.
pub fn summary_error_message(error: &ApiError) -> String {
    match error {
        ApiError::Http {
            detail: Some(detail),
            ..
        } => detail.clone(),
        ApiError::Http {
            status,
            detail: None,
        } => {
            format!("The summary service could not complete the request (HTTP {status}).")
        }
        other => other.to_string(),
    }
}

/// `PATCH /api/people/{id}/rename` — update a profile's display_name.
pub async fn rename_person(person_id: &str, new_name: &str) -> Result<PersonProfile, ApiError> {
    let route = format!("/api/people/{}/rename", segment(person_id));
    let body = serde_json::json!({ "display_name": new_name });
    send_json(Method::PATCH, &route, Some(&body)).await
}

/// `POST /api/people/{source_id}/merge_into/{target_id}` — absorb source
/// into target; the updated target on success.
pub async fn merge_people(source_id: &str, target_id: &str) -> Result<PersonProfile, ApiError> {
    let route = format!(
        "/api/people/{}/merge_into/{}",
        segment(source_id),
        segment(target_id)
    );
    send_json(Method::POST, &route, NO_BODY).await
}

/// `PATCH /api/people/{id}/tracked` — toggle whether a profile accumulates
/// new observations.
pub async fn set_person_tracked(person_id: &str, tracked: bool) -> Result<PersonProfile, ApiError> {
    let route = format!("/api/people/{}/tracked", segment(person_id));
    let body = serde_json::json!({ "tracked": tracked });
    send_json(Method::PATCH, &route, Some(&body)).await
}

/// `PATCH /api/meetings/{id}/skip_observations` — toggle per-meeting opt-out.
pub async fn set_meeting_skip_observations(
    meeting_id: &str,
    skip: bool,
) -> Result<Meeting, ApiError> {
    let route = format!("/api/meetings/{}/skip_observations", segment(meeting_id));
    let body = serde_json::json!({ "skip": skip });
    send_json(Method::PATCH, &route, Some(&body)).await
}

/// The persisted transcript segments of a meeting.
pub struct TranscriptReply {
    pub segments: Vec<crate::state::TranscriptSegment>,
    /// The server answered from the saved transcript, not the live one.
    pub saved_fallback: bool,
}

pub async fn delete_transcript_segment(meeting_id: &str, segment_id: &str) -> Result<(), ApiError> {
    let route = format!(
        "/api/meetings/{}/segments/{}",
        segment(meeting_id),
        segment(segment_id)
    );
    send(Method::DELETE, &route, NO_BODY).await.map(|_| ())
}

pub async fn removed_transcript_segments(meeting_id: &str) -> Result<Vec<String>, ApiError> {
    let route = format!("/api/meetings/{}/segment-removals", segment(meeting_id));
    get_json(&route).await
}

pub async fn fetch_transcript(meeting_id: &str) -> Result<TranscriptReply, ApiError> {
    let route = format!("/api/meetings/{}/segments", segment(meeting_id));
    let response = send(Method::GET, &route, NO_BODY).await?;
    let saved_fallback = response
        .headers()
        .get("x-bruhs-transcript-source")
        .as_deref()
        == Some("saved");
    Ok(TranscriptReply {
        segments: decode(response).await?,
        saved_fallback,
    })
}

/// `POST /api/meetings/{id}/chat` — the thread bound to this meeting, created
/// with the transcript and summary in its system prompt when there is none.
pub async fn create_or_get_meeting_chat(
    meeting_id: &str,
) -> Result<bruhs_models::api::Thread, ApiError> {
    let route = format!("/api/meetings/{}/chat", segment(meeting_id));
    send_json(Method::POST, &route, NO_BODY).await
}

/// `POST /api/meetings/{meeting_id}/speakers/{speaker_id}/link/{person_id}` —
/// link a speaker to an existing person; the updated meeting on success.
pub async fn link_speaker_to_person(
    meeting_id: &str,
    speaker_id: &str,
    person_id: &str,
) -> Result<Meeting, ApiError> {
    let route = format!(
        "/api/meetings/{}/speakers/{}/link/{}",
        segment(meeting_id),
        segment(speaker_id),
        segment(person_id)
    );
    send_json(Method::POST, &route, NO_BODY).await
}

// ── Google Calendar ──────────────────────────────────────────────────────────

use crate::state::{Calendar, GoogleCalendarSyncState};

pub async fn fetch_google_sync_state() -> Result<GoogleCalendarSyncState, ApiError> {
    get_json("/api/calendar/google/state").await
}

pub async fn start_google_oauth() -> Result<StartOAuthResponse, ApiError> {
    send_json(Method::POST, "/api/calendar/google/oauth/start", NO_BODY).await
}

pub async fn disconnect_google() -> Result<(), ApiError> {
    send(Method::POST, "/api/calendar/google/disconnect", NO_BODY)
        .await
        .map(drop)
}

pub async fn force_google_sync() -> Result<(), ApiError> {
    send(Method::POST, "/api/calendar/google/sync", NO_BODY)
        .await
        .map(drop)
}

pub async fn fetch_google_calendars() -> Result<Vec<Calendar>, ApiError> {
    get_json("/api/calendar/google/calendars").await
}

#[derive(serde::Serialize)]
pub struct PatchCalendarBody {
    pub sync_enabled: bool,
}

pub async fn patch_google_calendar(
    id: &str,
    body: &PatchCalendarBody,
) -> Result<Calendar, ApiError> {
    let route = format!("/api/calendar/google/calendars/{}", segment(id));
    send_json(Method::PATCH, &route, Some(body)).await
}

// ── Capture daemon (through the server's proxy) ─────────────────────────────

pub async fn fetch_capture_status() -> Result<CaptureStatus, ApiError> {
    get_json("/api/capture/status").await
}

/// How long the activity-monitor permission call may take before it is
/// abandoned.
const ACTIVITY_MONITOR_TIMEOUT_MS: u32 = 5_000;

/// Read (`None`) or change the microphone activity-monitor permission.
/// [`ApiError::Timeout`] when the daemon does not answer in time.
pub async fn activity_monitor(
    command: Option<&bruhs_models::ActivityMonitorCommand>,
) -> Result<bruhs_models::ActivityMonitorStatus, ApiError> {
    let abort = web_sys::AbortController::new()
        .map_err(|_| ApiError::Network("the browser cannot cancel requests".into()))?;
    let request = async {
        let builder = gloo_net::http::RequestBuilder::new(&url("/api/capture/activity-monitor"))
            .abort_signal(Some(&abort.signal()));
        let request = match command {
            Some(command) => builder.method(Method::POST).json(command)?,
            None => builder.method(Method::GET).build()?,
        };
        decode(send_request(request).await?).await
    };
    futures_util::pin_mut!(request);
    let timeout = gloo_timers::future::TimeoutFuture::new(ACTIVITY_MONITOR_TIMEOUT_MS);
    futures_util::pin_mut!(timeout);
    match futures_util::future::select(request, timeout).await {
        futures_util::future::Either::Left((result, _)) => result,
        futures_util::future::Either::Right(_) => {
            abort.abort();
            Err(ApiError::Timeout {
                after_ms: ACTIVITY_MONITOR_TIMEOUT_MS,
            })
        }
    }
}

/// `POST /api/people/{id}/enroll` — upload a WAV clip; the server extracts
/// the embedding via the capture daemon and upserts the person.
pub async fn enroll_voice(person_id: &str, wav_bytes: &[u8]) -> Result<PersonProfile, ApiError> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(wav_bytes));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type("audio/wav");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props)
        .map_err(|_| ApiError::Network("the browser could not package the recording".into()))?;
    let request = gloo_net::http::RequestBuilder::new(&url(&format!(
        "/api/people/{}/enroll",
        segment(person_id)
    )))
    .method(Method::POST)
    .header("Content-Type", "audio/wav")
    .body(blob)?;
    decode(send_request(request).await?).await
}

pub async fn capture_start(body: &CaptureStartBody) -> Result<(), ApiError> {
    send(Method::POST, "/api/capture/start", Some(body))
        .await
        .map(drop)
}

pub async fn capture_stop() -> Result<(), ApiError> {
    send(Method::POST, "/api/capture/stop", NO_BODY)
        .await
        .map(drop)
}

// ── Threads from other tools ────────────────────────────────────────────────

/// `GET /api/import/sessions`: `source`'s threads matching `query`, newest
/// first.
pub async fn import_sessions(
    source: bruhs_models::logic::portable::ImportSource,
    query: &str,
) -> Result<Vec<bruhs_models::logic::portable::ImportCandidate>, ApiError> {
    get_json(&format!(
        "/api/import/sessions?source={}&limit=100&q={}",
        source.as_str(),
        segment(query.trim())
    ))
    .await
}

/// `GET /api/import/thread`: one thread from another tool, with its messages.
pub async fn import_thread(
    source: bruhs_models::logic::portable::ImportSource,
    id: &str,
) -> Result<bruhs_models::logic::portable::ImportedThread, ApiError> {
    get_json(&format!(
        "/api/import/thread?source={}&id={}",
        source.as_str(),
        segment(id)
    ))
    .await
}

/// `POST /api/export/t3`: start a T3 Code thread carrying a transcript.
pub async fn export_to_t3(
    request: &bruhs_models::logic::portable::ExportT3Request,
) -> Result<bruhs_models::logic::portable::ExportT3Response, ApiError> {
    send_json(Method::POST, "/api/export/t3", Some(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_errors_show_the_server_message_or_a_fixed_line() {
        assert_eq!(
            summary_error_message(&ApiError::http(
                503,
                r#"{"error":"The model is unavailable."}"#
            )),
            "The model is unavailable."
        );
        for body in [
            "<html>Proxy failure</html>".into(),
            r#"{"error":7}"#.into(),
            serde_json::json!({"error":"x".repeat(513)}).to_string(),
        ] {
            assert_eq!(
                summary_error_message(&ApiError::http(503, &body)),
                "The summary service could not complete the request (HTTP 503)."
            );
        }
    }

    /// A chat without a project sends no path (the server searches the home
    /// directory); it used to send `"."`, the server's own working directory.
    #[test]
    fn a_file_search_without_a_project_omits_the_path() {
        assert_eq!(
            files_request("main", None),
            serde_json::json!({"query":"main"})
        );
        assert_eq!(
            files_request("main", Some("/Users/me/p")),
            serde_json::json!({"path":"/Users/me/p","query":"main"})
        );
    }

    /// The web sends the shared approve body; the server reads `request_id`
    /// (and the older `requestId`).
    #[test]
    fn approve_body_uses_the_shared_wire_shape() {
        let body = bruhs_models::api::ApproveRequest {
            request_id: "r1".into(),
            approved: false,
            reason: Some("Denied by user".into()),
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"request_id":"r1","approved":false,"reason":"Denied by user"})
        );
    }
}
