//! The backup routes (D20). Every call takes the page's `AbortSignal`, so
//! leaving Settings drops it. Making and opening a backup start jobs the
//! page polls with [`job`], so no call waits on a long job. Uploads go as
//! `application/octet-stream`: the web part's ciphertext, or the chosen
//! `File` itself, which the browser streams from disk. A body with a
//! password is built in a `Zeroizing` buffer and never logged.
#![expect(
    dead_code,
    reason = "used from Task 14: Settings, Backup calls these routes"
)]

use bruhs_models::backup::{
    valid_id, valid_stamp, ApplyAccepted, ApplyRestore, BackupJob, BackupOverview, CreateBackup,
    OpenBackup, RestoreStatus, UploadId, GONE, WEB_UPLOAD_MAX,
};
use gloo_net::http::{Method, Request, RequestBuilder, Response};
use serde::Serialize;
use std::io::{self, Write};
use wasm_bindgen::JsValue;
use web_sys::AbortSignal;
use zeroize::Zeroizing;

use super::http::{decode, send_request, url};
use super::ApiError;

const BACKUP_PATH: &str = "/api/backup";
const WEB_PARTS_PATH: &str = "/api/backup/web-parts";
const UPLOADS_PATH: &str = "/api/backup/restore/uploads";
const OPEN_PATH: &str = "/api/backup/restore";
const APPLY_PATH: &str = "/api/backup/restore/apply";
const STATUS_PATH: &str = "/api/backup/restore/status";
const WEB_PART_PATH: &str = "/api/backup/restore/web-part";
const PREVIOUS_WEB_PART_PATH: &str = "/api/backup/restore/web-part/previous";
const HEALTH_PATH: &str = "/api/health";
const RAW_BYTES: &str = "application/octet-stream";

fn id_path(prefix: &str, id: &str) -> Option<String> {
    valid_id(id).then(|| format!("{prefix}/{id}"))
}

fn job_path(id: &str) -> Option<String> {
    id_path("/api/backup/jobs", id)
}

fn file_path(id: &str) -> Option<String> {
    id_path("/api/backup/files", id)
}

fn staged_path(id: &str) -> Option<String> {
    id_path(OPEN_PATH, id)
}

fn kept_path(stamp: &str) -> Option<String> {
    valid_stamp(stamp).then(|| format!("/api/backup/pre-restore/{stamp}"))
}

/// A path for an id or stamp the server would refuse is refused here the
/// same way: 404 with `GONE`.
fn checked(path: Option<String>) -> Result<String, ApiError> {
    path.ok_or_else(|| ApiError::Http {
        status: 404,
        detail: Some(GONE.into()),
    })
}

fn builder(method: Method, route: &str, signal: &AbortSignal) -> RequestBuilder {
    RequestBuilder::new(&url(route))
        .method(method)
        .abort_signal(Some(signal))
}

/// Counts the bytes written to it and keeps none.
struct Measure(usize);

impl Write for Measure {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `body` as JSON in a buffer that is wiped when dropped. It is measured
/// first and allocated once at that size, so it never regrows and leaves an
/// unwiped copy of the password behind (B Task 13 M3).
fn secret_json<B: Serialize>(body: &B) -> Result<Zeroizing<Vec<u8>>, ApiError> {
    let failed = |error: serde_json::Error| ApiError::Decode(error.to_string());
    let mut measure = Measure(0);
    serde_json::to_writer(&mut measure, body).map_err(failed)?;
    let mut json = Zeroizing::new(Vec::with_capacity(measure.0));
    serde_json::to_writer(&mut *json, body).map_err(failed)?;
    Ok(json)
}

/// Whether a web part reply's `Content-Length` is there and within
/// `WEB_UPLOAD_MAX`, so the page never reads a larger body (B Task 13 M2).
/// The server always sends it.
fn web_part_fits(content_length: Option<&str>) -> bool {
    content_length
        .and_then(|length| length.trim().parse::<u64>().ok())
        .is_some_and(|length| length <= WEB_UPLOAD_MAX)
}

/// A JSON request whose body holds a password. The browser's own copy of
/// the body can't be wiped; this side's is.
fn secret_request<B: Serialize>(
    method: Method,
    route: &str,
    body: &B,
    signal: &AbortSignal,
) -> Result<Request, ApiError> {
    let json = secret_json(body)?;
    let text = std::str::from_utf8(&json).map_err(|error| ApiError::Decode(error.to_string()))?;
    Ok(builder(method, route, signal)
        .header("Content-Type", "application/json")
        .body(text)?)
}

fn raw_request(
    route: &str,
    body: impl Into<JsValue>,
    signal: &AbortSignal,
) -> Result<Request, ApiError> {
    Ok(builder(Method::POST, route, signal)
        .header("Content-Type", RAW_BYTES)
        .body(body)?)
}

async fn get(route: &str, signal: &AbortSignal) -> Result<Response, ApiError> {
    send_request(builder(Method::GET, route, signal).build()?).await
}

async fn delete(route: &str, signal: &AbortSignal) -> Result<(), ApiError> {
    send_request(builder(Method::DELETE, route, signal).build()?).await?;
    Ok(())
}

async fn upload_id(request: Request) -> Result<String, ApiError> {
    let UploadId { id } = decode(send_request(request).await?).await?;
    Ok(id)
}

/// `GET /api/backup`: sizes, the running job, a waiting or staged restore,
/// the kept sets and whether a web part waits.
pub async fn overview(signal: &AbortSignal) -> Result<BackupOverview, ApiError> {
    decode(get(BACKUP_PATH, signal).await?).await
}

/// `POST /api/backup/web-parts`: the locked web part, before `create`
/// names it. Answers its upload id.
pub async fn upload_web_part(bytes: &[u8], signal: &AbortSignal) -> Result<String, ApiError> {
    upload_id(raw_request(
        WEB_PARTS_PATH,
        js_sys::Uint8Array::from(bytes),
        signal,
    )?)
    .await
}

/// `POST /api/backup`: starts making a backup. Poll the job it answers.
pub async fn create(body: &CreateBackup, signal: &AbortSignal) -> Result<BackupJob, ApiError> {
    decode(send_request(secret_request(Method::POST, BACKUP_PATH, body, signal)?).await?).await
}

/// `GET /api/backup/jobs/{id}`.
pub async fn job(id: &str, signal: &AbortSignal) -> Result<BackupJob, ApiError> {
    decode(get(&checked(job_path(id))?, signal).await?).await
}

/// `DELETE /api/backup/jobs/{id}`: stops a running job.
pub async fn cancel_job(id: &str, signal: &AbortSignal) -> Result<(), ApiError> {
    delete(&checked(job_path(id))?, signal).await
}

/// Where the page downloads a made backup: `None` for anything but an id.
pub fn download_url(id: &str) -> Option<String> {
    file_path(id).map(|path| url(&path))
}

/// `POST /api/backup/restore/uploads`: the chosen backup file, sent as the
/// request body so the browser streams it from disk. Answers its upload id.
pub async fn upload_restore(
    file: &web_sys::File,
    signal: &AbortSignal,
) -> Result<String, ApiError> {
    upload_id(raw_request(
        UPLOADS_PATH,
        JsValue::from(file.clone()),
        signal,
    )?)
    .await
}

/// `POST /api/backup/restore`: starts opening a backup. Poll the job it
/// answers for the preview.
pub async fn open(body: &OpenBackup, signal: &AbortSignal) -> Result<BackupJob, ApiError> {
    decode(send_request(secret_request(Method::POST, OPEN_PATH, body, signal)?).await?).await
}

/// `DELETE /api/backup/restore/{id}`: drops an opened backup.
pub async fn discard(id: &str, signal: &AbortSignal) -> Result<(), ApiError> {
    delete(&checked(staged_path(id))?, signal).await
}

/// `POST /api/backup/restore/apply`. The server stops just after it
/// answers; poll [`health`] for it to come back.
pub async fn apply(body: &ApplyRestore, signal: &AbortSignal) -> Result<ApplyAccepted, ApiError> {
    let request = builder(Method::POST, APPLY_PATH, signal).json(body)?;
    decode(send_request(request).await?).await
}

/// `GET /api/backup/restore/status`: the last restore and a waiting web part.
pub async fn status(signal: &AbortSignal) -> Result<RestoreStatus, ApiError> {
    decode(get(STATUS_PATH, signal).await?).await
}

/// `GET /api/backup/restore/web-part`: the waiting web part's ciphertext.
/// A reply that doesn't say it is within `WEB_UPLOAD_MAX` isn't read.
pub async fn web_part(signal: &AbortSignal) -> Result<Vec<u8>, ApiError> {
    let too_large = || ApiError::Decode("the web part is over its cap".into());
    let response = get(WEB_PART_PATH, signal).await?;
    if !web_part_fits(response.headers().get("content-length").as_deref()) {
        return Err(too_large());
    }
    let bytes = response.binary().await?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > WEB_UPLOAD_MAX) {
        return Err(too_large());
    }
    Ok(bytes)
}

/// `DELETE /api/backup/restore/web-part`: the browser has restored it.
pub async fn web_part_done(signal: &AbortSignal) -> Result<(), ApiError> {
    delete(WEB_PART_PATH, signal).await
}

/// `POST /api/backup/restore/web-part/previous`: this browser's data from
/// before its restore, locked with the restore password, for the newest
/// kept set.
pub async fn web_part_previous(bytes: &[u8], signal: &AbortSignal) -> Result<(), ApiError> {
    let request = raw_request(
        PREVIOUS_WEB_PART_PATH,
        js_sys::Uint8Array::from(bytes),
        signal,
    )?;
    send_request(request).await?;
    Ok(())
}

/// `DELETE /api/backup/pre-restore/{stamp}`: deletes a kept set.
pub async fn delete_kept(stamp: &str, signal: &AbortSignal) -> Result<(), ApiError> {
    delete(&checked(kept_path(stamp))?, signal).await
}

/// Whether the server answers `/api/health`, for the wait after an apply.
pub async fn health(signal: &AbortSignal) -> bool {
    match builder(Method::GET, HEALTH_PATH, signal).build() {
        Ok(request) => send_request(request).await.is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::BASE_URL;
    use bruhs_models::backup::OpenSource;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn paths_are_api_shaped() {
        assert_eq!(
            download_url(ID).as_deref(),
            Some(format!("{BASE_URL}/api/backup/files/{ID}").as_str())
        );
        for bad in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "../../api/vault/0123456789abcdef",
            "0123456789abcdef/123456789abcdef",
        ] {
            assert_eq!(download_url(bad), None, "{bad}");
            assert_eq!(job_path(bad), None, "{bad}");
            assert_eq!(staged_path(bad), None, "{bad}");
        }
        assert_eq!(
            job_path(ID).as_deref(),
            Some(format!("/api/backup/jobs/{ID}").as_str())
        );
        assert_eq!(
            staged_path(ID).as_deref(),
            Some(format!("/api/backup/restore/{ID}").as_str())
        );
        assert_eq!(
            kept_path("2026-11-20-0930-0a1f").as_deref(),
            Some("/api/backup/pre-restore/2026-11-20-0930-0a1f")
        );
        for bad in ["2026-11-20-0930-0A1F", "2026-11-20-0930", "../x", ID] {
            assert_eq!(kept_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_refused_id_answers_as_the_server_would() {
        assert_eq!(
            checked(None),
            Err(ApiError::Http {
                status: 404,
                detail: Some(GONE.into())
            })
        );
    }

    #[test]
    fn a_web_part_reply_must_say_it_fits() {
        assert!(web_part_fits(Some("0")));
        assert!(web_part_fits(Some(&WEB_UPLOAD_MAX.to_string())));
        for over in [
            None,
            Some((WEB_UPLOAD_MAX + 1).to_string()),
            Some("-1".to_string()),
            Some("lots".to_string()),
        ] {
            assert!(!web_part_fits(over.as_deref()), "{over:?}");
        }
    }

    #[test]
    fn a_long_secret_body_never_regrows_its_buffer() {
        let create = CreateBackup {
            password: "correct horse battery".into(),
            options: Default::default(),
            app: bruhs_models::backup::BackupApp::Web,
            web_part: None,
            web_db_version: None,
            destination: Some(format!("/{}", "d".repeat(40 * 1024))),
            overwrite: false,
        };
        let body = secret_json(&create).unwrap();
        assert_eq!(*body, serde_json::to_vec(&create).unwrap());
        assert_eq!(body.capacity(), body.len());
    }

    #[test]
    fn secret_bodies_are_the_same_json_serde_writes() {
        let open = OpenBackup {
            source: OpenSource::Upload { id: ID.into() },
            password: "a \"quoted\" pässword".into(),
        };
        let body = secret_json(&open).unwrap();
        assert_eq!(*body, serde_json::to_vec(&open).unwrap());
    }
}
