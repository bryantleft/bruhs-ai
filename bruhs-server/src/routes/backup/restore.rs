//! Restoring a backup over HTTP (D8, D15, D18 to D20; rulings F19, F21,
//! F22, F24), for both apps' Settings, Backup section.
//!
//! - `POST /api/backup/restore/uploads`: a backup file from the browser,
//!   streamed to `backups/incoming` with no body limit; answers its
//!   [`UploadId`].
//! - `POST /api/backup/restore` (an [`OpenBackup`]): starts the job that
//!   checks and stages it, ending `Opened` with the preview.
//! - `DELETE /api/backup/restore/{id}`: drops a staged restore.
//! - `POST /api/backup/restore/apply` (an [`ApplyRestore`]): after the
//!   owner's confirmation, schedules it, stops this server, its bruhs and
//!   the background service, and leaves the swap to the detached applier.
//! - `GET /api/backup/restore/status`: a waiting restore, the last one
//!   applied and a browser part that waits.
//! - `GET` and `DELETE /api/backup/restore/web-part`: that browser part,
//!   and dropping it once the browser restored it.
//! - `POST /api/backup/restore/web-part/previous`: the browser's data from
//!   before its own restore, kept with the newest pre-restore set.
//! - `DELETE /api/backup/pre-restore/{stamp}`: deletes a kept set.
//!
//! Every route checks [`super::owner_only_backup`] first. Ids are 32
//! lowercase hex characters and stamps D18's form; anything else is 404.
//! The password moves into a wiped buffer at once, and every answer is
//! fixed copy: no body, header value or path inside a backup folder is
//! logged or echoed.

use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    extract::{
        rejection::{JsonRejection, PathRejection},
        DefaultBodyLimit, Path, Request, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse as _, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use bruhs_core::backup::{
    desktop_running, finish_web_part, keep_previous_web_part, pending, result, schedule,
    unschedule, waiting_web_part, waiting_web_part_file, BackupError, BackupHost, Roots,
    UploadKind,
};
use bruhs_core::local_tools::LocalToolsHost;
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::AppState;
use bruhs_models::backup::{
    valid_stamp, ApplyAccepted, ApplyRestore, BackupJob, OpenBackup, OpenSource, RestoreStatus,
    UploadId, BACKUP_BUSY, CONFIRM_REPLACE, GONE, INVALID_REQUEST, QUIT_DESKTOP_FIRST,
    RECORDING_FIRST, RESTORE_AT_NEXT_START, RESTORE_NOT_STARTED, SERVICE_NOT_STOPPED,
    SERVICE_TOO_OLD, SOURCE_MISSING, UPLOAD_TYPE, WEB_PART_TYPE,
};
use tokio::io::AsyncReadExt as _;
use tokio_util::io::ReaderStream;
use zeroize::Zeroizing;

use super::{
    backup_error, blocking, checked_id, gone, host, not_raw_bytes, owner_only_backup, receive,
    MAX_BODY_BYTES, READ_BYTES,
};
use crate::config::{Config, ServerState};
use crate::error::AppError;
use crate::restore_control::Control;

/// The restore side's routes (D20), each with its body limit.
pub(super) fn router() -> Router<ServerState> {
    Router::new()
        // Streamed to disk with no limit; the upload stops when the disk
        // would fill (D8).
        .route(
            "/api/backup/restore/uploads",
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/api/backup/restore",
            post(open).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/api/backup/restore/{id}", delete(discard))
        .route(
            "/api/backup/restore/apply",
            post(apply).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/api/backup/restore/status", get(status))
        .route(
            "/api/backup/restore/web-part",
            get(web_part).delete(web_part_done),
        )
        // Capped like the browser's backup part, by the upload itself.
        .route(
            "/api/backup/restore/web-part/previous",
            post(web_part_previous),
        )
        .route("/api/backup/pre-restore/{stamp}", delete(delete_kept))
}

/// The control `main` layers in; a router built without one can't stop
/// anything, so it applies nothing.
fn control(control: Option<Extension<Control>>) -> Result<Control, AppError> {
    control
        .map(|Extension(control)| control)
        .ok_or_else(|| AppError::Unavailable(RESTORE_NOT_STARTED.into()))
}

/// `POST /api/backup/restore/uploads`: a backup file sent as
/// `application/octet-stream` (else 415 with [`UPLOAD_TYPE`]).
pub async fn upload(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    request: Request,
) -> Result<Response, AppError> {
    owner_only_backup(&config.access, request.headers())?;
    let backup = host(backup)?;
    if let Some(refusal) = not_raw_bytes(request.headers(), UPLOAD_TYPE) {
        return Ok(refusal);
    }
    let id = receive(&backup, UploadKind::Restore, request).await?;
    Ok(Json(UploadId { id }).into_response())
}

/// `POST /api/backup/restore`: starts opening an upload (removed when the
/// job ends) or a file the desktop's dialog chose, which must be a regular
/// file at an absolute path.
pub async fn open(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    body: Result<Json<OpenBackup>, JsonRejection>,
) -> Result<Json<BackupJob>, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let Ok(Json(OpenBackup { source, password })) = body else {
        return Err(AppError::BadRequest(INVALID_REQUEST.into()));
    };
    let password = Zeroizing::new(password);
    let (path, remove_after) = match source {
        OpenSource::Upload { id } => {
            let host = backup.clone();
            let found = blocking(move || {
                host.uploaded(UploadKind::Restore, &id)
                    .ok_or(BackupError::Refused(GONE))
            });
            (found.await?, true)
        }
        OpenSource::Path { path } => (blocking(move || regular_file(&path)).await?, false),
    };
    backup
        .start_open(path, password, remove_after)
        .map(Json)
        .map_err(backup_error)
}

/// `path` resolved, when it names a regular file.
fn regular_file(path: &str) -> Result<PathBuf, BackupError> {
    let missing = || BackupError::Refused(SOURCE_MISSING);
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err(missing());
    }
    let path = std::fs::canonicalize(path).map_err(|_| missing())?;
    std::fs::metadata(&path)
        .is_ok_and(|metadata| metadata.is_file())
        .then_some(path)
        .ok_or_else(missing)
}

/// `DELETE /api/backup/restore/{id}`: removes a staged restore and its key.
pub async fn discard(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let id = checked_id(id)?;
    blocking(move || backup.discard_staged(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What the apply handler stops besides the service: the crew's worker
/// and the bruhs' browsers.
struct Running {
    app: Arc<AppState>,
    reactive: Option<ReactiveRuntime>,
    tools: Option<Arc<LocalToolsHost>>,
}

impl Running {
    /// Stops every bruh run without pausing or ungranting anything (ruling
    /// F19), then closes every bruh browser.
    async fn halt(&self) {
        if let Some(reactive) = &self.reactive {
            if reactive.halt_for_restore().await.is_err() {
                tracing::warn!("Bruhs couldn't mark the stopped runs before the restore");
            }
        }
        if let Some(tools) = &self.tools {
            tools.close_all().await;
        }
    }

    /// Undoes [`Self::halt`] after a restore that was called off (B Task
    /// 12 I1): the bruhs' browsers can open again, and the worker runs
    /// again with a sync pass that gives them fresh slots. Runs the halt
    /// interrupted stay interrupted, as after a restart.
    fn resume(&self) {
        if let Some(tools) = &self.tools {
            tools.reopen_after_restore();
        }
        if let Some(reactive) = &self.reactive {
            if reactive
                .resume_after_restore(Arc::clone(&self.app))
                .is_err()
            {
                tracing::warn!("Bruhs runs start again at the next start");
            }
        }
    }
}

/// `POST /api/backup/restore/apply`: the owner's confirmation (D15, ruling
/// F22 a). Refused unless `replace` (`CONFIRM_REPLACE`), while a job runs
/// (`BACKUP_BUSY`), while a meeting records (`RECORDING_FIRST`), while the
/// desktop runs and this isn't the desktop asking (`QUIT_DESKTOP_FIRST`),
/// when the installed service is older than the backup's data
/// (`SERVICE_TOO_OLD`) and when the source can't be applied; a refusal
/// schedules and stops nothing. Then, in order: schedule the restore,
/// spawn the applier, stop the bruh runs and browsers, stop the service
/// unless this is it, answer and shut down. The service's own process
/// answers first and stops itself last.
pub async fn apply(
    State(server): State<ServerState>,
    backup: Option<Extension<BackupHost>>,
    restore_control: Option<Extension<Control>>,
    reactive: Option<Extension<ReactiveRuntime>>,
    tools: Option<Extension<Arc<LocalToolsHost>>>,
    headers: HeaderMap,
    body: Result<Json<ApplyRestore>, JsonRejection>,
) -> Result<Response, AppError> {
    owner_only_backup(&server.config.access, &headers)?;
    let backup = host(backup)?;
    let Ok(Json(ApplyRestore {
        source,
        replace,
        from_desktop,
    })) = body
    else {
        return Err(AppError::BadRequest(INVALID_REQUEST.into()));
    };
    if !replace {
        return Err(AppError::BadRequest(CONFIRM_REPLACE.into()));
    }
    let control = control(restore_control)?;
    // First, since it may wait on the keychain: the moment checks below
    // then hold right up to the schedule (B Task 12 M1).
    let (checker, checked) = (backup.clone(), source.clone());
    let protocol = blocking(move || checker.check_restore(&checked)).await?;
    if backup.running() {
        return Err(AppError::Conflict(BACKUP_BUSY.into()));
    }
    if backup.recording().await {
        return Err(AppError::Conflict(RECORDING_FIRST.into()));
    }
    let roots = backup.roots().clone();
    if !from_desktop && blocking(move || desktop_running(&roots)).await? {
        return Err(AppError::Conflict(QUIT_DESKTOP_FIRST.into()));
    }
    let asked = Arc::clone(&control);
    let (service, loaded, own) = tokio::task::spawn_blocking(move || {
        (
            asked.service_installed_protocol(),
            asked.service_loaded(),
            asked.is_service_process(),
        )
    })
    .await
    .map_err(|_| AppError::Unavailable(RESTORE_NOT_STARTED.into()))?;
    if service
        .zip(protocol)
        .is_some_and(|(service, data)| service < data)
    {
        return Err(AppError::Conflict(SERVICE_TOO_OLD.into()));
    }

    let roots = backup.roots().clone();
    blocking(move || schedule(&roots, &source, loaded)).await?;
    let spawner = Arc::clone(&control);
    let spawned = tokio::task::spawn_blocking(move || spawner.spawn_applier()).await;
    if !matches!(spawned, Ok(Ok(()))) {
        tracing::warn!("the restore's applier didn't start");
        return Err(called_off(backup.roots(), RESTORE_NOT_STARTED).await);
    }
    let running = Running {
        app: server.app,
        reactive: reactive.map(|Extension(reactive)| reactive),
        tools: tools.map(|Extension(tools)| tools),
    };
    running.halt().await;
    tracing::info!("a restore is scheduled; this server stops for it");

    if own {
        // Answer first: stopping is this process's own shutdown.
        return Ok(answer_then(
            ApplyAccepted {
                service_stopped: true,
            },
            move || {
                if control.stop_service().is_err() {
                    tracing::warn!("the background service didn't confirm its stop");
                }
                control.exit_soon();
            },
        ));
    }
    let stopper = Arc::clone(&control);
    let stopped = tokio::task::spawn_blocking(move || stopper.stop_service()).await;
    let Ok(Ok(service_stopped)) = stopped else {
        tracing::warn!("the background service didn't stop for the restore");
        running.resume();
        return Err(called_off(backup.roots(), SERVICE_NOT_STOPPED).await);
    };
    control.exit_soon();
    Ok(Json(ApplyAccepted { service_stopped }).into_response())
}

/// Removes the restore this server scheduled but couldn't hand over, and
/// answers `copy`; when it stays scheduled, the answer says it finishes at
/// the next start (B Task 12 M2).
async fn called_off(roots: &Roots, copy: &'static str) -> AppError {
    let roots = roots.clone();
    if blocking(move || unschedule(&roots)).await.is_ok() {
        AppError::Unavailable(copy.into())
    } else {
        tracing::warn!("Bruhs couldn't take back a restore it couldn't start");
        AppError::Unavailable(RESTORE_AT_NEXT_START.into())
    }
}

/// `accepted` as the answer; `after` runs on a thread of its own once the
/// answer's body was handed to the connection, or the connection dropped
/// it.
fn answer_then(accepted: ApplyAccepted, after: impl FnOnce() + Send + 'static) -> Response {
    /// Runs its step on a thread of its own when dropped, or right here
    /// when no thread starts (B Task 12 M4): in the service's own process
    /// the step only asks for the shutdown, so it doesn't block.
    struct After<F: FnOnce() + Send + 'static>(Option<F>);
    impl<F: FnOnce() + Send + 'static> Drop for After<F> {
        fn drop(&mut self) {
            let Some(step) = self.0.take() else { return };
            let shared = Arc::new(std::sync::Mutex::new(Some(step)));
            let theirs = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name("restore-stop".into())
                .spawn(move || run_once(&theirs));
            if spawned.is_err() {
                tracing::warn!("no thread to stop the service after the answer");
                run_once(&shared);
            }
        }
    }
    /// Takes the step and runs it, once.
    fn run_once<F: FnOnce()>(step: &std::sync::Mutex<Option<F>>) {
        let taken = step
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(step) = taken {
            step();
        }
    }
    let body = serde_json::to_vec(&accepted).unwrap_or_default();
    let length = body.len();
    let after = After(Some(after));
    let stream = async_stream::stream! {
        let after = after;
        yield Ok::<_, io::Error>(Bytes::from(body));
        drop(after);
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, length)
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::OK.into_response())
}

/// `GET /api/backup/restore/status`: a waiting restore, the last restore
/// applied and the browser part that waits, with the path rewrites the
/// browser applies to its records (D19).
pub async fn status(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
) -> Result<Json<RestoreStatus>, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let roots = host(backup)?.roots().clone();
    blocking(move || {
        Ok(RestoreStatus {
            pending: pending(&roots),
            result: result(&roots),
            web_part: waiting_web_part(&roots),
        })
    })
    .await
    .map(Json)
}

/// `GET /api/backup/restore/web-part`: the waiting browser part's
/// ciphertext, exactly as the backup held it, streamed from disk. It stays
/// until the browser deletes it.
pub async fn web_part(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let roots = host(backup)?.roots().clone();
    let (file, bytes) = blocking(move || {
        let gone = || BackupError::Refused(GONE);
        let (_, path) = waiting_web_part_file(&roots).ok_or_else(gone)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| gone())?;
        let metadata = file.metadata().map_err(|_| gone())?;
        if !metadata.is_file() {
            return Err(gone());
        }
        Ok((file, metadata.len()))
    })
    .await?;
    let chunks =
        ReaderStream::with_capacity(tokio::fs::File::from_std(file).take(bytes), READ_BYTES);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(chunks))
        .map_err(|_| backup_error(BackupError::Io(io::Error::other("a bad response"))))
}

/// `DELETE /api/backup/restore/web-part`: the browser restored its part.
pub async fn web_part_done(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let roots = host(backup)?.roots().clone();
    blocking(move || finish_web_part(&roots)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/backup/restore/web-part/previous`: the browser's data from
/// before its own restore, encrypted in the browser, sent as
/// `application/octet-stream` (else 415 with [`WEB_PART_TYPE`]). It goes
/// into the newest pre-restore set, replacing one kept before.
pub async fn web_part_previous(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    request: Request,
) -> Result<Response, AppError> {
    owner_only_backup(&config.access, request.headers())?;
    let backup = host(backup)?;
    if let Some(refusal) = not_raw_bytes(request.headers(), WEB_PART_TYPE) {
        return Ok(refusal);
    }
    let id = receive(&backup, UploadKind::PreviousWebPart, request).await?;
    let path = backup
        .uploaded(UploadKind::PreviousWebPart, &id)
        .ok_or_else(gone)?;
    let roots = backup.roots().clone();
    blocking(move || {
        let kept = keep_previous_web_part(&roots, &path);
        if kept.is_err() {
            // Best effort: the sweep removes it later otherwise.
            let _ = std::fs::remove_file(&path);
        }
        kept
    })
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /api/backup/pre-restore/{stamp}`: deletes a kept set, its
/// folders and its sealed key. A second delete finishes one a stop cut
/// short.
pub async fn delete_kept(
    State(config): State<Arc<Config>>,
    backup: Option<Extension<BackupHost>>,
    headers: HeaderMap,
    stamp: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, AppError> {
    owner_only_backup(&config.access, &headers)?;
    let backup = host(backup)?;
    let stamp = stamp
        .ok()
        .map(|Path(stamp)| stamp)
        .filter(|stamp| valid_stamp(stamp))
        .ok_or_else(gone)?;
    blocking(move || backup.delete_kept(&stamp)).await?;
    Ok(StatusCode::NO_CONTENT)
}
