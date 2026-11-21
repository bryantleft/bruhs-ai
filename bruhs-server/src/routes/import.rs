//! Threads from the other agent tools on this machine, chat service
//! exports, local agents' personas, and sending one thread to T3 Code.
//! See `bruhs_core::import`.
//!
//! - `GET /api/import/sessions?source=&q=&limit=`: a local tool's threads.
//! - `GET /api/import/thread?source=&id=&upload=`: one thread with its
//!   messages, from a local tool, or from the export batch `upload`.
//! - `POST /api/import/uploads` (multipart, one or more `file` fields) and
//!   `POST /api/import/files` (an [`ImportPaths`], at most
//!   [`MAX_PATHS_BODY`]): start reading an export; both answer 202 with
//!   its [`ImportJob`].
//! - `GET` and `DELETE /api/import/uploads/{id}`: a read's state, and
//!   ending it or discarding its batch.
//! - `GET /api/import/agents?source=&path=`: an [`AgentScan`].
//! - `POST /api/export/t3`: starts a T3 thread carrying a bruhs transcript.
//!
//! Every route sits behind the shared loopback guard and checks
//! [`Policy::owner_request`] before anything else (D15, F20): other
//! loopback pages may neither read the owner's other tools nor write to
//! T3. Every answer is fixed copy from `bruhs_models::imports`, and the
//! guard marks each one `Cache-Control: no-store`. No handler logs a body,
//! a path, a name or an I/O error's text.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{
        multipart::MultipartRejection,
        rejection::{JsonRejection, PathRejection, QueryRejection},
        DefaultBodyLimit, Multipart, Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use bruhs_core::import::{self, ImportError, ImportHost, ImportRoots, ImportUpload};
use bruhs_models::imports::{
    valid_batch_id, AgentScan, AgentScanQuery, ImportJob, ImportPaths, EXPORT_TOO_LARGE,
    FOLDER_MISSING, IMPORT_BUSY, IMPORT_FAILED, IMPORT_GONE, IMPORT_INVALID, MAX_PATHS_BODY,
    OWNER_ONLY_IMPORT, TOO_MANY_FILES,
};
use bruhs_models::logic::portable::{
    ExportT3Request, ExportT3Response, ImportCandidate, ImportListQuery, ImportSource,
    ImportThreadQuery, ImportedThread,
};

use crate::config::{Config, ServerState};
use crate::error::AppError;
use crate::local_access::Policy;
use crate::routes::paths::HomePath;

#[cfg(test)]
mod tests;

/// An upload is written to disk in pieces of about this many bytes.
const WRITE_BYTES: usize = 1 << 20;

/// The read and agent routes; `main` adds the sessions, thread and T3
/// routes beside them. Only the upload lifts the body limit: the core
/// upload enforces its own caps.
pub fn router() -> Router<ServerState> {
    Router::new()
        .route(
            "/api/import/uploads",
            post(post_upload).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/import/files",
            post(post_files).layer(DefaultBodyLimit::max(MAX_PATHS_BODY)),
        )
        .route("/api/import/uploads/{id}", get(get_job).delete(delete_job))
        .route("/api/import/agents", get(get_agents))
}

/// The one import host this server runs, after a sweep at start and then
/// every hour (D7); each sweep runs off the async workers. `None` when no
/// home folder can be named: the routes then answer `IMPORT_FAILED`.
pub fn start() -> Option<ImportHost> {
    let host = ImportHost::from_env()?;
    let sweeper = host.clone();
    tokio::spawn(async move {
        if tokio::task::spawn_blocking(move || sweeper.sweep())
            .await
            .is_err()
        {
            tracing::warn!("The import sweep stopped early; it runs again within the hour");
        }
    });
    host.keep_sweeping();
    Some(host)
}

/// `app` with the import host layered in, when there is one (F18).
pub fn with_host(app: Router, host: Option<ImportHost>) -> Router {
    match host {
        Some(host) => app.layer(Extension(host)),
        None => app,
    }
}

/// Refuse any page but the owner's own app (D15).
pub fn owner_only_import(policy: &Policy, headers: &HeaderMap) -> Result<(), AppError> {
    if policy.owner_request(headers) {
        Ok(())
    } else {
        Err(AppError::Forbidden(OWNER_ONLY_IMPORT.into()))
    }
}

/// An import route's failure: an [`AppError`], or a 413, which `AppError`
/// lacks (F18) and which is built by hand the same way. Either way the
/// body is `{"error": <fixed copy>}`.
pub enum Refusal {
    App(AppError),
    TooLarge(&'static str),
}

impl From<AppError> for Refusal {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        match self {
            Self::App(error) => error.into_response(),
            Self::TooLarge(copy) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": copy })),
            )
                .into_response(),
        }
    }
}

/// The status and fixed copy for an import error. An I/O error's path and
/// source, and a missing thread's id, are dropped here and never logged
/// (F18).
fn import_error(error: ImportError) -> Refusal {
    let error = match error {
        ImportError::Refused(copy @ (EXPORT_TOO_LARGE | TOO_MANY_FILES)) => {
            return Refusal::TooLarge(copy)
        }
        ImportError::Refused(copy) => AppError::BadRequest(copy.into()),
        // `NO_SPACE_IMPORT` with the size to free, as Backup's `NO_SPACE`.
        error @ ImportError::Space(_) => AppError::Unavailable(error.to_string()),
        ImportError::Busy => AppError::Conflict(IMPORT_BUSY.into()),
        ImportError::Gone | ImportError::NotFound { .. } => gone(),
        ImportError::NotListed(_) => invalid(),
        _ => failed(),
    };
    Refusal::App(error)
}

fn gone() -> AppError {
    AppError::NotFound(IMPORT_GONE.into())
}

fn invalid() -> AppError {
    AppError::BadRequest(IMPORT_INVALID.into())
}

fn failed() -> AppError {
    AppError::internal_msg(IMPORT_FAILED)
}

/// The host `main` layers in; without one there is nothing to read into.
fn host(host: Option<Extension<ImportHost>>) -> Result<ImportHost, AppError> {
    host.map(|Extension(host)| host).ok_or_else(failed)
}

/// A path id of 32 lowercase hex characters, or 404.
fn checked_id(id: Result<Path<String>, PathRejection>) -> Result<String, AppError> {
    id.ok()
        .map(|Path(id)| id)
        .filter(|id| valid_batch_id(id))
        .ok_or_else(gone)
}

fn source(name: &str) -> Result<ImportSource, AppError> {
    ImportSource::parse(name).ok_or_else(invalid)
}

/// Runs a step that touches the disk off the async runtime. A step that
/// panics answers `IMPORT_FAILED`, never its message.
async fn blocking<T: Send + 'static>(
    step: impl FnOnce() -> Result<T, ImportError> + Send + 'static,
) -> Result<T, Refusal> {
    tokio::task::spawn_blocking(step)
        .await
        .map_err(|_| Refusal::App(failed()))?
        .map_err(import_error)
}

/// `GET /api/import/sessions`: a local tool's threads, newest first.
pub async fn get_sessions(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    query: Result<Query<ImportListQuery>, QueryRejection>,
) -> Result<Json<Vec<ImportCandidate>>, Refusal> {
    owner_only_import(&config.access, &headers)?;
    let Query(query) = query.map_err(|_| invalid())?;
    let source = source(query.source.as_deref().unwrap_or_default())?;
    let found = blocking(move || {
        import::list(
            &ImportRoots::from_env(),
            source,
            query.q.as_deref(),
            query.limit,
        )
    })
    .await?;
    Ok(Json(found))
}

/// `GET /api/import/thread`: from the export batch `upload` when it is
/// set (an export's source only), else from the local tool's own store.
pub async fn get_thread(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    query: Result<Query<ImportThreadQuery>, QueryRejection>,
) -> Result<Json<ImportedThread>, Refusal> {
    owner_only_import(&config.access, &headers)?;
    let Query(ImportThreadQuery {
        source: name,
        id,
        upload,
    }) = query.map_err(|_| invalid())?;
    let source = source(&name)?;
    let thread = match upload {
        Some(batch) => {
            if !ImportSource::EXPORTS.contains(&source) {
                return Err(invalid().into());
            }
            let host = host(imports)?;
            blocking(move || host.thread(&batch, source, &id)).await?
        }
        None => blocking(move || import::load(&ImportRoots::from_env(), source, &id)).await?,
    };
    Ok(Json(thread))
}

/// `POST /api/import/uploads`: every `file` field, in order, becomes one
/// uploaded file (its file name is never read, F33); any other field is
/// `IMPORT_INVALID`. Answers 202 with the read, which then runs on its
/// own. A refused or cut-off upload leaves nothing behind.
pub async fn post_upload(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<ImportJob>), Refusal> {
    owner_only_import(&config.access, &headers)?;
    let host = host(imports)?;
    let mut multipart = multipart.map_err(|_| invalid())?;
    let mut upload = blocking(move || host.begin_upload()).await?;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => return Err(abandon(upload).await),
        };
        if field.name() != Some("file") {
            return Err(abandon(upload).await);
        }
        upload = blocking(move || upload.next_file().map(|()| upload)).await?;
        upload = receive(field, upload).await?;
    }
    let job = blocking(move || upload.finish()).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// Writes one field's bytes to the upload's current file, off the async
/// workers in pieces of about [`WRITE_BYTES`].
async fn receive(
    mut field: axum::extract::multipart::Field<'_>,
    mut upload: ImportUpload,
) -> Result<ImportUpload, Refusal> {
    let mut pending = Vec::with_capacity(WRITE_BYTES);
    loop {
        let chunk = match field.chunk().await {
            Ok(chunk) => chunk,
            Err(_) => return Err(abandon(upload).await),
        };
        let end = chunk.is_none();
        if let Some(chunk) = chunk {
            pending.extend_from_slice(&chunk);
        }
        if pending.len() >= WRITE_BYTES || (end && !pending.is_empty()) {
            let piece = std::mem::replace(&mut pending, Vec::with_capacity(WRITE_BYTES));
            upload = blocking(move || upload.write(&piece).map(|()| upload)).await?;
        }
        if end {
            return Ok(upload);
        }
    }
}

/// Drops a refused upload off the async workers (its drop deletes its
/// folder, then frees the read slot) and answers `IMPORT_INVALID`.
async fn abandon(upload: ImportUpload) -> Refusal {
    let _ = tokio::task::spawn_blocking(move || drop(upload)).await;
    Refusal::App(invalid())
}

/// `POST /api/import/files`: files or folders the desktop's picker chose,
/// read in place. Each must be absolute: nothing resolves against the
/// server's working folder.
pub async fn post_files(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    body: Result<Json<ImportPaths>, JsonRejection>,
) -> Result<(StatusCode, Json<ImportJob>), Refusal> {
    owner_only_import(&config.access, &headers)?;
    let host = host(imports)?;
    let Json(ImportPaths { paths }) = body.map_err(|_| invalid())?;
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    if !paths.iter().all(|path| path.is_absolute()) {
        return Err(AppError::BadRequest(FOLDER_MISSING.into()).into());
    }
    let job = blocking(move || host.read_paths(paths)).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// `GET /api/import/uploads/{id}`: the read's state, until its batch is
/// discarded or expires.
pub async fn get_job(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Result<Json<ImportJob>, Refusal> {
    owner_only_import(&config.access, &headers)?;
    let host = host(imports)?;
    let id = checked_id(id)?;
    let job = blocking(move || host.job(&id).ok_or(ImportError::Gone)).await?;
    Ok(Json(job))
}

/// `DELETE /api/import/uploads/{id}`: ends the read, or discards its
/// batch.
pub async fn delete_job(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, Refusal> {
    owner_only_import(&config.access, &headers)?;
    let host = host(imports)?;
    let id = checked_id(id)?;
    blocking(move || host.discard(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/import/agents`: an OpenClaw or Hermes Agent scan, of `path`
/// when given (passed as typed; the scan reads `~` itself), else of the
/// agent's default places.
pub async fn get_agents(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    query: Result<Query<AgentScanQuery>, QueryRejection>,
) -> Result<Json<AgentScan>, Refusal> {
    owner_only_import(&config.access, &headers)?;
    let host = host(imports)?;
    let Query(AgentScanQuery { source: name, path }) = query.map_err(|_| invalid())?;
    let source = source(&name)?;
    if !ImportSource::AGENTS.contains(&source) {
        return Err(invalid().into());
    }
    let picked = path.filter(|path| !path.is_empty()).map(PathBuf::from);
    let scan = blocking(move || host.scan(source, picked.as_deref())).await?;
    Ok(Json(scan))
}

/// `POST /api/export/t3`: a T3 thread carrying a bruhs transcript, in a
/// project folder inside the home. Owner only (F20): it writes to T3's
/// database.
pub async fn post_export_t3(
    State(config): State<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Json<ExportT3Request>, JsonRejection>,
) -> Result<Json<ExportT3Response>, Refusal> {
    owner_only_import(&config.access, &headers)?;
    let Json(body) = body.map_err(|_| invalid())?;
    let project = HomePath::resolve(&body.project_path)
        .await
        .map_err(AppError::from)?
        .into_path_buf();
    let thread_id =
        blocking(move || import::export_to_t3(&project, &body.title, &body.model, &body.markdown))
            .await?;
    Ok(Json(ExportT3Response { thread_id }))
}
