//! The backup routes (D20) for Settings, Backup: the same calls as the
//! web's `api/backup.rs`, minus the browser's own uploads. The desktop
//! saves a backup to a file it chose and opens one from a path on this
//! computer, so neither the file nor the browser's part travels over HTTP.
//! Making and opening start jobs the page polls with [`ServerClient::backup_job`].
//!
//! A body with a password is written straight into the request and the
//! caller wipes its own copy after the call; the password is never logged,
//! and failure bodies keep only the server's fixed sentence. A native
//! client sends no `Origin`, which the owner-only routes allow.

use super::{ServerClient, ServerError};
use bruhs_models::backup::{
    server_answered, valid_id, valid_stamp, ApplyAccepted, ApplyRestore, BackupJob, BackupOverview,
    CreateBackup, OpenBackup, RestoreStatus, ALL_COPY, GONE, UNREADABLE_REPLY,
};
use bruhs_models::vault::VAULT_UNREACHABLE;
use std::time::Duration;

pub(crate) const BACKUP_PATH: &str = "/api/backup";
pub(crate) const OPEN_PATH: &str = "/api/backup/restore";
pub(crate) const APPLY_PATH: &str = "/api/backup/restore/apply";
pub(crate) const STATUS_PATH: &str = "/api/backup/restore/status";
/// The longest sentence a failure may show; longer bodies are not copy.
const MAX_MESSAGE_LEN: usize = 512;
/// An apply answers only once every bruh run stopped and the background
/// service was asked to stop (ruling F22 a), which can take a while.
const APPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// `/api/backup/jobs/{id}`, or `None` for an id the server would refuse.
pub(crate) fn job_path(id: &str) -> Option<String> {
    valid_id(id).then(|| format!("/api/backup/jobs/{id}"))
}

/// `/api/backup/restore/{id}`: an opened backup.
pub(crate) fn staged_path(id: &str) -> Option<String> {
    valid_id(id).then(|| format!("{OPEN_PATH}/{id}"))
}

/// `/api/backup/pre-restore/{stamp}`: a kept set.
pub(crate) fn kept_path(stamp: &str) -> Option<String> {
    valid_stamp(stamp).then(|| format!("/api/backup/pre-restore/{stamp}"))
}

/// A path for an id the server would refuse is refused here the same way.
fn checked(path: Option<String>) -> Result<String, ServerError> {
    path.ok_or(ServerError::Invalid(GONE))
}

impl ServerError {
    /// The server's own sentence for a refused backup call (`{"error": …}`),
    /// if it sent one that may be shown: any 4xx or 503 sentence, and a
    /// 500's only when it is the backup's fixed copy. Kept sets read it to
    /// tell `NOTHING_TO_PUT_BACK` and `KEYCHAIN_SET_UNREADABLE` apart.
    pub(crate) fn backup_detail(&self) -> Option<String> {
        let Self::Status { code, body, .. } = self else {
            return None;
        };
        let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
        let detail = value.get("error")?.as_str()?.trim();
        let code = code.as_u16();
        let shown = (400..500).contains(&code)
            || code == 503
            || (code == 500 && ALL_COPY.contains(&detail));
        (shown && !detail.is_empty() && detail.len() <= MAX_MESSAGE_LEN).then(|| detail.to_owned())
    }

    /// What the Backup page shows for a failed call: the server's sentence,
    /// else the web's wording for the same failure.
    pub(crate) fn backup_message(&self) -> String {
        if let Some(detail) = self.backup_detail() {
            return detail;
        }
        match self {
            Self::Transport { .. } => VAULT_UNREACHABLE.to_owned(),
            Self::Status { code, .. } => server_answered(code.as_u16()),
            Self::Decode { .. } => UNREADABLE_REPLY.to_owned(),
            Self::Invalid(message) => (*message).to_owned(),
            Self::Client(_) => VAULT_UNREACHABLE.to_owned(),
        }
    }
}

/// A request body that holds a password.
pub(crate) trait PasswordBody {
    fn password_mut(&mut self) -> &mut String;
}

impl PasswordBody for CreateBackup {
    fn password_mut(&mut self) -> &mut String {
        &mut self.password
    }
}

impl PasswordBody for OpenBackup {
    fn password_mut(&mut self) -> &mut String {
        &mut self.password
    }
}

/// A password-holding body that wipes its password when it drops, whether
/// it was sent or its call was dropped before it ran (the app quit).
pub(crate) struct Wiped<T: PasswordBody>(pub(crate) T);

impl<T: PasswordBody> std::ops::Deref for Wiped<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: PasswordBody> Drop for Wiped<T> {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self.0.password_mut());
    }
}

impl ServerClient {
    async fn backup_delete(&self, path: &str) -> Result<(), ServerError> {
        self.send_empty::<()>(reqwest::Method::DELETE, path, None)
            .await
    }

    /// `GET /api/backup`: sizes, the running job, a waiting or staged
    /// restore and the kept sets. An older server answers 404.
    pub async fn backup_overview(&self) -> Result<BackupOverview, ServerError> {
        self.get_json(BACKUP_PATH).await
    }

    /// `POST /api/backup`: starts making a backup into `destination`. Poll
    /// the job it answers.
    pub async fn create_backup(&self, body: &CreateBackup) -> Result<BackupJob, ServerError> {
        self.send_json(reqwest::Method::POST, BACKUP_PATH, Some(body))
            .await
    }

    /// `GET /api/backup/jobs/{id}`.
    pub async fn backup_job(&self, id: &str) -> Result<BackupJob, ServerError> {
        self.get_json(&checked(job_path(id))?).await
    }

    /// `DELETE /api/backup/jobs/{id}`: stops a running job; its poll then
    /// answers the stop.
    pub async fn cancel_backup_job(&self, id: &str) -> Result<(), ServerError> {
        self.backup_delete(&checked(job_path(id))?).await
    }

    /// `POST /api/backup/restore`: starts opening a backup file. Poll the
    /// job it answers for the preview.
    pub async fn open_backup(&self, body: &OpenBackup) -> Result<BackupJob, ServerError> {
        self.send_json(reqwest::Method::POST, OPEN_PATH, Some(body))
            .await
    }

    /// `DELETE /api/backup/restore/{id}`: drops an opened backup.
    pub async fn discard_staged(&self, id: &str) -> Result<(), ServerError> {
        self.backup_delete(&checked(staged_path(id))?).await
    }

    /// `POST /api/backup/restore/apply`: the confirmed restore. The server
    /// answers, then stops; the desktop then starts again (D17).
    pub async fn apply_restore(&self, body: &ApplyRestore) -> Result<ApplyAccepted, ServerError> {
        self.send_json_with_timeout(reqwest::Method::POST, APPLY_PATH, Some(body), APPLY_TIMEOUT)
            .await
    }

    /// `GET /api/backup/restore/status`: a waiting restore and the last one
    /// applied.
    pub async fn restore_status(&self) -> Result<RestoreStatus, ServerError> {
        self.get_json(STATUS_PATH).await
    }

    /// `DELETE /api/backup/pre-restore/{stamp}`: deletes a kept set.
    pub async fn delete_kept(&self, stamp: &str) -> Result<(), ServerError> {
        self.backup_delete(&checked(kept_path(stamp))?).await
    }
}

#[cfg(test)]
mod tests {
    use super::{job_path, kept_path, staged_path, ServerClient, ServerError};
    use bruhs_models::backup::{
        ApplyRestore, BackupApp, BackupOptions, CreateBackup, OpenBackup, OpenSource,
        RestoreSource, BACKUP_BUSY, GONE, KEYCHAIN_SET_UNREADABLE, NOTHING_TO_PUT_BACK,
    };
    use bruhs_models::vault::VAULT_UNREACHABLE;

    const ID: &str = "0123456789abcdef0123456789abcdef";
    const STAMP: &str = "2026-11-20-0930-0a1f";

    /// Serve one canned reply per connection and hand back each request.
    async fn serve(
        replies: Vec<(&'static str, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                // Headers and a small body can come in two reads.
                loop {
                    let got = socket.read(&mut buffer[read..]).await.unwrap();
                    read += got;
                    let text = String::from_utf8_lossy(&buffer[..read]);
                    let Some(end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if got == 0 || read >= end + 4 + length {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            requests
        });
        (base, server)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn body(request: &str) -> serde_json::Value {
        let start = request.find("\r\n\r\n").unwrap() + 4;
        serde_json::from_str(&request[start..]).unwrap()
    }

    fn job(state: serde_json::Value) -> String {
        serde_json::json!({ "id": ID, "state": state }).to_string()
    }

    #[test]
    fn paths_and_bodies_match_the_web() {
        let canary = format!("canary-{}", uuid::Uuid::new_v4());
        let running = serde_json::json!({
            "state": "running", "done_bytes": 0, "total_bytes": 10, "step": "collecting"
        });
        let overview = serde_json::json!({ "supported": true }).to_string();
        let requests = runtime().block_on(async {
            let (base, server) = serve(vec![
                ("200 OK", overview),
                ("200 OK", job(running.clone())),
                ("200 OK", job(running.clone())),
                ("204 No Content", String::new()),
                ("200 OK", job(running)),
                ("204 No Content", String::new()),
                ("200 OK", r#"{"service_stopped":true}"#.into()),
                ("200 OK", "{}".into()),
                ("204 No Content", String::new()),
            ])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            assert!(client.backup_overview().await.unwrap().supported);
            let create = CreateBackup {
                password: canary.clone(),
                options: BackupOptions::default(),
                app: BackupApp::Desktop,
                web_part: None,
                web_db_version: None,
                destination: Some("/Users/me/Downloads/b.bruhsbak".into()),
                overwrite: true,
            };
            assert_eq!(client.create_backup(&create).await.unwrap().id, ID);
            drop(create);
            assert_eq!(client.backup_job(ID).await.unwrap().id, ID);
            client.cancel_backup_job(ID).await.unwrap();
            let open = OpenBackup {
                source: OpenSource::Path {
                    path: "/Users/me/b.bruhsbak".into(),
                },
                password: canary.clone(),
            };
            client.open_backup(&open).await.unwrap();
            drop(open);
            client.discard_staged(ID).await.unwrap();
            let apply = ApplyRestore {
                source: RestoreSource::Staged { id: ID.into() },
                replace: true,
                from_desktop: true,
            };
            assert!(client.apply_restore(&apply).await.unwrap().service_stopped);
            assert!(client.restore_status().await.unwrap().result.is_none());
            client.delete_kept(STAMP).await.unwrap();
            server.await.unwrap()
        });
        let lines: Vec<&str> = requests
            .iter()
            .map(|request| request.lines().next().unwrap())
            .collect();
        assert_eq!(
            lines,
            [
                "GET /api/backup HTTP/1.1",
                "POST /api/backup HTTP/1.1",
                &format!("GET /api/backup/jobs/{ID} HTTP/1.1"),
                &format!("DELETE /api/backup/jobs/{ID} HTTP/1.1"),
                "POST /api/backup/restore HTTP/1.1",
                &format!("DELETE /api/backup/restore/{ID} HTTP/1.1"),
                "POST /api/backup/restore/apply HTTP/1.1",
                "GET /api/backup/restore/status HTTP/1.1",
                &format!("DELETE /api/backup/pre-restore/{STAMP} HTTP/1.1"),
            ]
        );
        let create = body(&requests[1]);
        assert_eq!(create["app"], "desktop");
        assert_eq!(create["destination"], "/Users/me/Downloads/b.bruhsbak");
        assert_eq!(create["overwrite"], true);
        assert_eq!(create["password"], canary.as_str());
        let open = body(&requests[4]);
        assert_eq!(
            open["source"],
            serde_json::json!({ "type": "path", "path": "/Users/me/b.bruhsbak" })
        );
        let apply = body(&requests[6]);
        assert_eq!(apply["from_desktop"], true);
        assert_eq!(apply["replace"], true);
        assert_eq!(
            apply["source"],
            serde_json::json!({ "type": "staged", "id": ID })
        );
        for request in &requests {
            assert!(!request.to_lowercase().contains("\r\norigin:"), "{request}");
        }
    }

    /// A body that notes when its password is reached and what it held.
    struct Probe<'a> {
        password: String,
        seen: &'a std::cell::RefCell<Vec<String>>,
    }

    impl super::PasswordBody for Probe<'_> {
        fn password_mut(&mut self) -> &mut String {
            self.seen.borrow_mut().push(self.password.clone());
            &mut self.password
        }
    }

    #[test]
    fn a_body_is_wiped_even_when_its_call_never_ran() {
        let seen = std::cell::RefCell::new(Vec::new());
        let body = super::Wiped(Probe {
            password: "correct horse 1".into(),
            seen: &seen,
        });
        // A call that owns the body and is dropped before it ever runs, as
        // when the app quits mid-flight.
        let call = async move {
            let _body = body;
        };
        drop(call);
        assert_eq!(*seen.borrow(), ["correct horse 1"], "the drop reached it");
        let mut wiped = super::Wiped(Probe {
            password: "correct horse 2".into(),
            seen: &seen,
        });
        zeroize::Zeroize::zeroize(super::PasswordBody::password_mut(&mut wiped.0));
        drop(wiped);
        assert_eq!(
            seen.borrow().last().map(String::as_str),
            Some(""),
            "zeroized"
        );
    }

    #[test]
    fn ids_the_server_would_refuse_are_never_sent() {
        assert_eq!(job_path("../vault"), None);
        assert_eq!(staged_path("ABC"), None);
        assert_eq!(kept_path("2026-11-20"), None);
        assert_eq!(
            kept_path(STAMP).as_deref(),
            Some("/api/backup/pre-restore/2026-11-20-0930-0a1f")
        );
        runtime().block_on(async {
            // Nothing listens here: a request would be a transport error.
            let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
            let error = client.backup_job("../vault").await.unwrap_err();
            assert_eq!(error.backup_message(), GONE);
            let error = client.delete_kept("nope").await.unwrap_err();
            assert!(matches!(error, ServerError::Invalid(GONE)), "{error:?}");
        });
    }

    #[test]
    fn only_the_servers_fixed_sentences_are_shown() {
        let status = |code: u16, body: &str| ServerError::Status {
            request: "POST /api/backup/restore/apply".into(),
            code: reqwest::StatusCode::from_u16(code).unwrap(),
            body: body.into(),
        };
        let sentence = |text: &str| serde_json::json!({ "error": text }).to_string();
        assert_eq!(
            status(409, &sentence(NOTHING_TO_PUT_BACK))
                .backup_detail()
                .as_deref(),
            Some(NOTHING_TO_PUT_BACK)
        );
        assert_eq!(
            status(409, &sentence(KEYCHAIN_SET_UNREADABLE)).backup_message(),
            KEYCHAIN_SET_UNREADABLE
        );
        assert_eq!(
            status(409, &sentence(BACKUP_BUSY)).backup_message(),
            BACKUP_BUSY
        );
        // A 500's fixed copy shows; any other 500 text never does.
        assert_eq!(status(500, &sentence(GONE)).backup_message(), GONE);
        assert_eq!(
            status(500, &sentence("rename failed: /private/path")).backup_message(),
            "The server answered HTTP 500."
        );
        assert_eq!(
            status(502, "<html>proxy</html>").backup_message(),
            "The server answered HTTP 502."
        );
        runtime().block_on(async {
            let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
            let error = client.backup_overview().await.unwrap_err();
            assert!(error.is_transport(), "{error:?}");
            assert_eq!(error.backup_message(), VAULT_UNREACHABLE);
        });
    }
}
