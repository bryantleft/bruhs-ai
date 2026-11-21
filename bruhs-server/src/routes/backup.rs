//! Making a backup over HTTP (D6, D7, D20, D23), for both apps' Settings,
//! Backup section.
//!
//! - `GET /api/backup`: the [`BackupOverview`].
//! - `POST /api/backup/web-parts`: the browser's encrypted part, streamed
//!   to `backups/incoming`; answers its [`UploadId`].
//! - `POST /api/backup` (a [`CreateBackup`], at most [`MAX_BODY_BYTES`]):
//!   starts the one job and answers it.
//! - `GET` and `DELETE /api/backup/jobs/{id}`: a job's state, and stopping
//!   it.
//! - `GET /api/backup/files/{id}`: a finished web backup, streamed as an
//!   attachment once and then deleted.
//! - The restore side lives in [`restore`].
//!
//! Every route sits behind the shared loopback guard and checks
//! [`Policy::owner_request`] before anything else: other loopback pages may
//! neither make a backup nor read what one holds. Ids are 32 lowercase hex
//! characters; anything else is 404. Bodies that aren't requests get
//! [`INVALID_REQUEST`], never serde's text, which can quote a password.
//! Every error is fixed copy, and no handler logs a body, a header value or
//! a path inside a backup folder.

use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{
        rejection::{JsonRejection, PathRejection},
        Path, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse as _, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use bruhs_core::backup::{BackupError, BackupHost, CreateRequest, Output, Roots, UploadKind};
use bruhs_core::local_tools::LocalToolsHost;
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::secrets::SecretStore;
use bruhs_models::backup::{
    valid_id, BackupJob, BackupOverview, CreateBackup, UploadId, BACKUP_BUSY, DESTINATION_EXISTS,
    GONE, INVALID_REQUEST, KEYCHAIN_SET_UNREADABLE, NOTHING_TO_PUT_BACK, OWNER_ONLY_BACKUP,
    RECORDING_FIRST, RESTORE_PENDING, SOURCE_MISSING, UNSUPPORTED_PLATFORM, WEB_PART_TYPE,
    WEB_TOO_LARGE, WEB_UPLOAD_MAX,
};
use futures_util::StreamExt as _;
use tokio::io::AsyncReadExt as _;
use tokio_util::io::ReaderStream;
use zeroize::Zeroizing;

use crate::config::{Config, ServerState};
use crate::error::AppError;
use crate::local_access::Policy;

/// The largest `POST /api/backup` body: a password of at most 1 KiB, the
/// options and a destination path, well under the server's 1 MiB default.
pub const MAX_BODY_BYTES: usize = 64 << 10;

/// The name a download goes by when its job is no longer the last one.
const FALLBACK_NAME: &str = "bruhs-backup.bruhsbak";
/// A web part is written to disk in pieces of about this many bytes.
const WRITE_BYTES: usize = 1 << 20;
/// A download is read from disk in pieces of this many bytes.
const READ_BYTES: usize = 256 << 10;
/// The one content type a web part arrives as (D20).
const WEB_PART_CONTENT_TYPE: &str = "application/octet-stream";
/// How long a start waits for its sweep before it serves anyway (final
/// review C-M4): a keychain that doesn't answer only holds up backups.
const SWEEP_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

mod restore;
// Split so clippy's allow-unwrap-in-tests sees `cfg(test)`.
#[cfg(test)]
mod restore_tests;
#[cfg(test)]
pub(super) mod tests;

/// Every backup route (D20), each with its body limit: this side's and
/// the restore side's.
pub fn router() -> Router<ServerState> {
    Router::new()
        .route(
            "/api/backup",
            get(overview)
                .post(create)
                .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        // The body streams to disk: the upload refuses one past
        // `WEB_UPLOAD_MAX`, and the handler one declared larger.
        .route("/api/backup/web-parts", post(upload_web_part))
        .route("/api/backup/jobs/{id}", get(job).delete(cancel_job))
        .route("/api/backup/files/{id}", get(download))
        .merge(restore::router())
}

/// The one backup host this server runs, after its sweep (ruling B Task 10
/// concern 1): every manifest names the release `/api/health` reports.
pub async fn start(
    store: SecretStore,
    tools: Arc<LocalToolsHost>,
    reactive: ReactiveRuntime,
) -> io::Result<BackupHost> {
    let roots = Roots::from_env().ok_or_else(|| io::Error::other("no home folder"))?;
    let host = BackupHost::new(roots, store)
        .with_computers(tools)
        .with_reactive(reactive)
        .with_app_version(app_version().to_owned());
    let host = sweep_at_start(host, BackupHost::sweep, SWEEP_WAIT).await;
    // Downloads and uploads expire after 30 minutes while it runs too
    // (final review A-I2).
    host.keep_sweeping_transfers();
    Ok(host)
}

/// Runs `sweep` off the async runtime (it may wait on the keychain) and
/// answers the host once it is done, or after `wait` (final review C-M4):
/// a sweep still running then goes on beside serving, holding the job
/// slot so no backup or restore starts until it ends. A sweep that panics
/// is logged and the server serves anyway: the next start sweeps again.
async fn sweep_at_start(
    host: BackupHost,
    sweep: impl FnOnce(&BackupHost) + Send + 'static,
    wait: std::time::Duration,
) -> BackupHost {
    // A fresh host runs no job, so this only fails if something else
    // took the slot; the sweep then runs without it.
    let hold = host.hold_jobs().ok();
    let sweeper = host.clone();
    let mut swept = tokio::task::spawn_blocking(move || {
        let _hold = hold;
        sweep(&sweeper);
    });
    let stopped = || {
        tracing::error!("The backup sweep stopped early; it runs again at the next start");
    };
    match tokio::time::timeout(wait, &mut swept).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => stopped(),
        Err(_) => {
            tracing::warn!("The backup sweep is still running; backups wait until it ends");
            tokio::spawn(async move {
                match swept.await {
                    Ok(()) => tracing::info!("The backup sweep ended"),
                    Err(_) => stopped(),
                }
            });
        }
    }
    host
}

/// The release this server reports at `/api/health`: its release
/// version, else its crate version.
fn app_version() -> &'static str {
    let release = env!("BRUHS_RELEASE_VERSION");
    if release.is_empty() {
        env!("CARGO_PKG_VERSION")
    } else {
        release
    }
}

/// Refuse any page but the owner's own app (D20, D21).
pub fn owner_only_backup(policy: &Policy, headers: &HeaderMap) -> Result<(), AppError> {
    if policy.owner_request(headers) {
        Ok(())
    } else {
        Err(AppError::Forbidden(OWNER_ONLY_BACKUP.into()))
    }
}

/// The owner copy and status for a backup error. Every message is the
/// core's fixed copy; an I/O error's source, which may name a path, is
/// dropped here and never logged (ruling F26).
pub fn backup_error(error: BackupError) -> AppError {
    match error {
        BackupError::Refused(UNSUPPORTED_PLATFORM) => {
            AppError::Unavailable(UNSUPPORTED_PLATFORM.into())
        }
        BackupError::Refused(
            message @ (RESTORE_PENDING | RECORDING_FIRST | DESTINATION_EXISTS | BACKUP_BUSY
            | NOTHING_TO_PUT_BACK),
        ) => AppError::Conflict(message.into()),
        BackupError::Refused(message @ (GONE | SOURCE_MISSING)) => {
            AppError::NotFound(message.into())
        }
        BackupError::Refused(message) | BackupError::Format(message) => {
            AppError::BadRequest(message.into())
        }
        BackupError::Busy(message) => AppError::Conflict(message.into()),
        BackupError::Space(message) => AppError::Unavailable(message),
        // Never passes on a retry: the set can only be deleted (B Task 8
        // R3-M2).
        BackupError::Keychain(KEYCHAIN_SET_UNREADABLE) => {
            AppError::Conflict(KEYCHAIN_SET_UNREADABLE.into())
        }
        BackupError::Keychain(message) => AppError::Unavailable(message.into()),
        error @ BackupError::Io(_) => AppError::internal_msg(error.message()),
    }
}

fn gone() -> AppError {
    AppError::NotFound(GONE.into())
}

/// The host `main` layers in; a router built without one has nothing to
/// back up.
fn host(host: Option<Extension<BackupHost>>) -> Result<BackupHost, AppError> {
    host.map(|Extension(host)| host)
        .ok_or_else(|| backup_error(BackupError::Io(io::Error::other("no backup host"))))
}

/// A path id of 32 lowercase hex characters, or 404.
fn checked_id(id: Result<Path<String>, PathRejection>) -> Result<String, AppError> {
    id.ok()
        .map(|Path(id)| id)
        .filter(|id| valid_id(id))
        .ok_or_else(gone)
}

/// `GET /api/backup`. Owner-only: sizes, the last backup and kept sets are
/// the owner's to see.
pub async fn overview(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
) -> Result<Json<BackupOverview>, AppError> {
    owner_only_backup(&config.access, &headers)?;
    Ok(Json(host(backup)?.overview().await))
}

/// `POST /api/backup/web-parts`: the browser's encrypted part, sent as
/// `application/octet-stream` (else 415 with [`WEB_PART_TYPE`]). Refused
/// when it doesn't start with a web part's header (`NOT_A_BACKUP`) and
/// when it is, or says it is, over `WEB_UPLOAD_MAX` (`WEB_TOO_LARGE`); a
/// refused or cut-off upload leaves nothing behind.
pub async fn upload_web_part(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    request: Request,
) -> Result<Response, AppError> {
    owner_only_backup(&config.access, request.headers())?;
    let backup = host(backup)?;
    if let Some(refusal) = not_raw_bytes(request.headers(), WEB_PART_TYPE) {
        return Ok(refusal);
    }
    let id = receive(&backup, UploadKind::WebPart, request).await?;
    Ok(Json(UploadId { id }).into_response())
}

/// 415 with `copy` unless the body is sent as `application/octet-stream`.
/// `AppError` has no 415 (ruling F26): the same body, by hand.
fn not_raw_bytes(headers: &HeaderMap, copy: &'static str) -> Option<Response> {
    let raw_bytes = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(WEB_PART_CONTENT_TYPE));
    (!raw_bytes).then(|| {
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(serde_json::json!({ "error": copy })),
        )
            .into_response()
    })
}

/// Streams `request`'s body into a new upload of `kind` and answers its
/// id. A browser part declared over `WEB_UPLOAD_MAX` is refused before it
/// is read; the upload itself refuses one that grows past it, and a
/// restore upload that would leave too little free space. A refused or
/// cut-off upload leaves nothing behind.
async fn receive(
    backup: &BackupHost,
    kind: UploadKind,
    request: Request,
) -> Result<String, AppError> {
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let web = matches!(kind, UploadKind::WebPart | UploadKind::PreviousWebPart);
    if web && declared.is_some_and(|bytes| bytes > WEB_UPLOAD_MAX) {
        return Err(AppError::BadRequest(WEB_TOO_LARGE.into()));
    }
    let backup = backup.clone();
    let mut upload = blocking(move || backup.begin_upload(kind)).await?;
    let mut frames = request.into_body().into_data_stream();
    let mut pending = Vec::with_capacity(WRITE_BYTES);
    loop {
        let frame = frames.next().await;
        let end = frame.is_none();
        if let Some(frame) = frame {
            let frame = frame.map_err(|_| AppError::BadRequest(INVALID_REQUEST.into()))?;
            pending.extend_from_slice(&frame);
        }
        if pending.len() >= WRITE_BYTES || (end && !pending.is_empty()) {
            let chunk = std::mem::replace(&mut pending, Vec::with_capacity(WRITE_BYTES));
            upload = blocking(move || upload.write(&chunk).map(|()| upload)).await?;
        }
        if end {
            break;
        }
    }
    blocking(move || upload.finish()).await
}

/// Runs a host step that touches the disk off the async runtime.
async fn blocking<T: Send + 'static>(
    step: impl FnOnce() -> Result<T, BackupError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(step)
        .await
        .map_err(|_| backup_error(BackupError::Io(io::Error::other("the step stopped"))))?
        .map_err(backup_error)
}

/// `POST /api/backup`: starts making a backup and answers the job. The
/// password moves into a wiped buffer at once; a desktop destination is
/// checked before the job starts.
pub async fn create(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    body: Result<Json<CreateBackup>, JsonRejection>,
) -> Result<Json<BackupJob>, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let Ok(Json(CreateBackup {
        password,
        options,
        app,
        web_part,
        web_db_version,
        destination,
        overwrite,
    })) = body
    else {
        return Err(AppError::BadRequest(INVALID_REQUEST.into()));
    };
    let password = Zeroizing::new(password);
    let web_part = match web_part {
        Some(id) => Some(backup.uploaded(UploadKind::WebPart, &id).ok_or_else(gone)?),
        None => None,
    };
    let output = match destination {
        Some(path) => Output::File {
            path: PathBuf::from(path),
            overwrite,
        },
        None => Output::Outgoing,
    };
    let job = backup
        .start_create(CreateRequest {
            password,
            options,
            app,
            web_part,
            web_db_version,
            output,
        })
        .await
        .map_err(backup_error)?;
    Ok(Json(job))
}

/// `GET /api/backup/jobs/{id}`: the job's state while it runs and after,
/// until the next job starts.
pub async fn job(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Result<Json<BackupJob>, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let id = checked_id(id)?;
    backup.job(&id).map(Json).ok_or_else(gone)
}

/// `DELETE /api/backup/jobs/{id}`: stops the job if it still runs; it
/// removes its partial file and ends `Failed`.
pub async fn cancel_job(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let id = checked_id(id)?;
    if backup.job(&id).is_none() {
        return Err(gone());
    }
    backup.cancel(&id);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/backup/files/{id}`: a finished web backup as an attachment,
/// streamed from disk. It is deleted once its last byte went out; a
/// download cut short leaves it for another try until the sweep.
pub async fn download(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Result<Response, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let id = checked_id(id)?;
    let path = backup.outgoing(&id).ok_or_else(gone)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| gone())?;
    let metadata = file.metadata().map_err(|_| gone())?;
    if !metadata.is_file() {
        return Err(gone());
    }
    let bytes = metadata.len();
    let name = backup
        .download_name(&id)
        .filter(|name| plain_name(name))
        .unwrap_or_else(|| FALLBACK_NAME.to_owned());
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
        .map_err(|_| backup_error(BackupError::Io(io::Error::other("a bad file name"))))?;

    let stream = async_stream::stream! {
        let mut sent = Sent { backup, id, bytes, sent: 0 };
        let mut chunks =
            ReaderStream::with_capacity(tokio::fs::File::from_std(file).take(bytes), READ_BYTES);
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(chunk) => {
                    let length = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
                    sent.sent = sent.sent.saturating_add(length);
                    yield Ok::<_, io::Error>(chunk);
                }
                Err(error) => {
                    yield Err(error);
                    break;
                }
            }
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, bytes)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .map_err(|_| backup_error(BackupError::Io(io::Error::other("a bad response"))))
}

/// Whether `name` needs no quoting inside a `Content-Disposition` header.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// Counts what a download handed to the connection. Dropped once every
/// byte went out, which the connection does right after it took the
/// last one, it removes the file (`finish_download`).
struct Sent {
    backup: BackupHost,
    id: String,
    bytes: u64,
    sent: u64,
}

impl Drop for Sent {
    fn drop(&mut self) {
        if self.sent == self.bytes {
            self.backup.finish_download(&self.id);
        }
    }
}
