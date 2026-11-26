//! Threads from the other agent tools on this machine, chat service
//! exports and local agent personas.
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
//!
//! Every route sits behind the shared loopback guard and checks
//! [`Policy::owner_request`] before anything else (D15, F20): other
//! loopback pages may neither read the owner's other tools nor write to
//! private files. Every answer is fixed copy from `bruhs_models::imports`, and the
//! guard marks each one `Cache-Control: no-store`. No handler logs a body,
//! a path, a name or an I/O error's text.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{
        multipart::MultipartRejection,
        rejection::{JsonRejection, PathRejection, QueryRejection},
        DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use bruhs_core::import::{self, ImportError, ImportHost, ImportRoots, ImportUpload};
use bruhs_models::imports::{
    valid_batch_id, AgentScan, AgentScanQuery, ImportJob, ImportPaths, EXPORT_TOO_LARGE,
    IMPORT_BUSY, IMPORT_CHAT_GONE, IMPORT_FAILED, IMPORT_GONE, IMPORT_INVALID, MAX_PATHS_BODY,
    OWNER_ONLY_IMPORT, TOO_MANY_FILES,
};
use bruhs_models::logic::portable::{
    ImportCandidate, ImportListQuery, ImportSource, ImportThreadQuery, ImportedThread,
};
use futures_util::StreamExt as _;

use crate::config::{Config, ServerState};
use crate::error::AppError;
use crate::local_access::Policy;

#[cfg(test)]
mod tests;

/// An upload is written to disk in pieces of about this many bytes.
const WRITE_BYTES: usize = 1 << 20;
/// How often an upload waiting on its client looks for a `DELETE` (M2).
const WATCH_EVERY: Duration = Duration::from_millis(200);
/// The most bytes of an upload's body read but not yet handed on as a
/// field's data: what the multipart parser may hold in a preamble, a
/// boundary or a field's header block (final review B). Field data passes
/// through in pieces far smaller.
const MAX_HELD: u64 = 8 << 20;

/// The read and agent routes; `main` adds the sessions and thread
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
        ImportError::Gone => gone(),
        // The batch or the tool is still here; only the chat isn't.
        ImportError::NotFound { .. } => AppError::NotFound(IMPORT_CHAT_GONE.into()),
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
/// own. A refused, cut-off, abandoned or discarded upload leaves nothing
/// behind and frees the read slot.
pub async fn post_upload(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    request: Request,
) -> Result<(StatusCode, Json<ImportJob>), Refusal> {
    owner_only_import(&config.access, &headers)?;
    let host = host(imports)?;
    let held = Arc::new(Held::default());
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, held.body(body));
    let mut multipart = Multipart::from_request(request, &())
        .await
        .map_err(|_: MultipartRejection| invalid())?;
    let mut receiving = Receiving(Some(blocking(move || host.begin_upload()).await?));
    loop {
        let field = match receiving.wait(multipart.next_field()).await {
            None => return Err(receiving.end(gone()).await),
            Some(Ok(Some(field))) => field,
            Some(Ok(None)) => break,
            Some(Err(_)) => return Err(receiving.end(invalid()).await),
        };
        if field.name() != Some("file") {
            return Err(receiving.end(invalid()).await);
        }
        receiving.step(ImportUpload::next_file).await?;
        receive(field, &mut receiving, &held).await?;
    }
    let upload = receiving.take()?;
    let job = blocking(move || upload.finish()).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// Writes one field's bytes to the upload's current file, off the async
/// workers in pieces of about [`WRITE_BYTES`].
async fn receive(
    mut field: axum::extract::multipart::Field<'_>,
    receiving: &mut Receiving,
    held: &Held,
) -> Result<(), Refusal> {
    let mut pending = Vec::with_capacity(WRITE_BYTES);
    loop {
        let chunk = match receiving.wait(field.chunk()).await {
            None => return Err(receiving.end(gone()).await),
            Some(Ok(chunk)) => chunk,
            Some(Err(_)) => return Err(receiving.end(invalid()).await),
        };
        let end = chunk.is_none();
        if let Some(chunk) = chunk {
            held.pass(&chunk);
            pending.extend_from_slice(&chunk);
        }
        if pending.len() >= WRITE_BYTES || (end && !pending.is_empty()) {
            let piece = std::mem::replace(&mut pending, Vec::with_capacity(WRITE_BYTES));
            receiving.step(move |upload| upload.write(&piece)).await?;
        }
        if end {
            return Ok(());
        }
    }
}

/// How much of an upload's body was read and how much was handed on as
/// field data. The body ends with an error once more than [`MAX_HELD`]
/// was read and not handed on, which the parser then answers as
/// `IMPORT_INVALID`: axum's parser has no limit of its own on what it
/// holds while it looks for a boundary or a header block's end.
#[derive(Default)]
struct Held {
    read: AtomicU64,
    passed: AtomicU64,
}

impl Held {
    /// `body`, counted.
    fn body(self: &Arc<Self>, body: Body) -> Body {
        let held = Arc::clone(self);
        Body::from_stream(body.into_data_stream().map(move |chunk| {
            let chunk = chunk?;
            let read = held.read.fetch_add(byte_len(&chunk), Ordering::Relaxed) + byte_len(&chunk);
            if read.saturating_sub(held.passed.load(Ordering::Relaxed)) > MAX_HELD {
                return Err(axum::BoxError::from("held too long"));
            }
            Ok(chunk)
        }))
    }

    /// Counts `chunk` as handed on.
    fn pass(&self, chunk: &[u8]) {
        self.passed.fetch_add(byte_len(chunk), Ordering::Relaxed);
    }
}

fn byte_len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// The upload a request is writing. Whatever ends the request, the
/// upload's drop (which deletes its folder, then frees the read slot) runs
/// off the async workers: a refused step drops it inside its blocking
/// task, [`Receiving::end`] waits for its drop there, and a request whose
/// client went away drops this guard, which hands the upload to a
/// blocking thread (M1).
struct Receiving(Option<ImportUpload>);

impl Receiving {
    /// Runs `step` on the upload off the async workers.
    async fn step(
        &mut self,
        step: impl FnOnce(&mut ImportUpload) -> Result<(), ImportError> + Send + 'static,
    ) -> Result<(), Refusal> {
        let mut upload = self.take()?;
        self.0 = Some(blocking(move || step(&mut upload).map(|()| upload)).await?);
        Ok(())
    }

    /// The upload, leaving nothing for this guard to drop.
    fn take(&mut self) -> Result<ImportUpload, Refusal> {
        self.0.take().ok_or(Refusal::App(failed()))
    }

    /// `next` from the client, or `None` once the upload was discarded
    /// (M2): a `DELETE` while the client stalls still frees the slot.
    /// Borrowed mutably: an upload is `Send` but not `Sync`.
    async fn wait<T>(&mut self, next: impl Future<Output = T>) -> Option<T> {
        tokio::pin!(next);
        let mut ticks = tokio::time::interval(WATCH_EVERY);
        loop {
            tokio::select! {
                out = &mut next => return Some(out),
                _ = ticks.tick() => {
                    if self.0.as_ref().is_some_and(ImportUpload::is_discarded) {
                        return None;
                    }
                }
            }
        }
    }

    /// Ends the upload and answers `refusal` once its folder is gone and
    /// the read slot is free.
    async fn end(&mut self, refusal: AppError) -> Refusal {
        if let Some(upload) = self.0.take() {
            let _ = tokio::task::spawn_blocking(move || drop(upload)).await;
        }
        Refusal::App(refusal)
    }
}

impl Drop for Receiving {
    fn drop(&mut self) {
        if let Some(upload) = self.0.take() {
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    runtime.spawn_blocking(move || drop(upload));
                }
                Err(_) => drop(upload),
            }
        }
    }
}

/// `POST /api/import/files`: files or folders the desktop's picker chose,
/// read in place (`~` read against the home, nothing relative). Only the
/// desktop sends these, and it sends no `Origin`: a page, the app's own
/// included, is refused (M5).
pub async fn post_files(
    State(config): State<Arc<Config>>,
    imports: Option<Extension<ImportHost>>,
    headers: HeaderMap,
    body: Result<Json<ImportPaths>, JsonRejection>,
) -> Result<(StatusCode, Json<ImportJob>), Refusal> {
    owner_only_import(&config.access, &headers)?;
    if headers.contains_key(header::ORIGIN) {
        return Err(AppError::Forbidden(OWNER_ONLY_IMPORT.into()).into());
    }
    let host = host(imports)?;
    let Json(ImportPaths { paths }) = body.map_err(|_| invalid())?;
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
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
