//! The import routes (D3, D10) for the Import thread dialog's Export file
//! tab and Move in: the same calls as the web's `api/imports.rs`, except
//! that the desktop names the files or folders its picker chose
//! (`POST /api/import/files`) instead of uploading them. The server reads
//! them in place and never deletes them. Only the server owns
//! `<core>/imports`; the desktop never reads a batch itself.
//!
//! A native client sends no `Origin`, which `/files` requires: it refuses
//! any request that carries one. Every failure becomes the line the dialog
//! shows: the server's fixed copy, else the import's own failure copy.

use super::{ServerClient, ServerError};
use bruhs_models::imports::{
    valid_batch_id, AgentScan, ImportJob, ImportPaths, ALL_COPY, IMPORT_FAILED, IMPORT_GONE,
};
use bruhs_models::logic::portable::{ImportSource, ImportedThread};
use bruhs_models::vault::VAULT_UNREACHABLE;

pub(crate) const FILES_PATH: &str = "/api/import/files";
const UPLOADS_PATH: &str = "/api/import/uploads";
/// The longest sentence a failure may show; longer bodies are not copy.
const MAX_MESSAGE_LEN: usize = 512;

/// `raw` as one percent-encoded URL component, byte for byte as the web's
/// `api::http::segment` writes it.
fn segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `/api/import/uploads/{id}`, for a batch id the server would take.
pub(crate) fn job_path(id: &str) -> Option<String> {
    valid_batch_id(id).then(|| format!("{UPLOADS_PATH}/{id}"))
}

/// One conversation of a read batch.
pub(crate) fn thread_path(source: ImportSource, upload: &str, id: &str) -> Option<String> {
    valid_batch_id(upload).then(|| {
        format!(
            "/api/import/thread?source={}&id={}&upload={upload}",
            source.as_str(),
            segment(id)
        )
    })
}

/// An agent scan, of `path` when one was given.
pub(crate) fn agents_path(source: ImportSource, path: Option<&str>) -> String {
    let mut route = format!("/api/import/agents?source={}", source.as_str());
    if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
        route.push_str("&path=");
        route.push_str(&segment(path));
    }
    route
}

/// A batch id the server would refuse is refused here the same way,
/// before any request.
fn checked(path: Option<String>) -> Result<String, ServerError> {
    path.ok_or(ServerError::Invalid(IMPORT_GONE))
}

impl ServerError {
    /// What the Import thread dialog shows for a failed import call: the
    /// server's sentence (any 4xx or 503 sentence, and a 500's only when it
    /// is the import's fixed copy), else the import's own failure copy.
    /// An unreachable server says so as the other server pages do.
    pub(crate) fn import_message(&self) -> String {
        match self {
            Self::Status { code, body, .. } => {
                let code = code.as_u16();
                serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("error")?
                            .as_str()
                            .map(str::trim)
                            .map(str::to_owned)
                    })
                    .filter(|detail| {
                        let showable = (400..500).contains(&code)
                            || code == 503
                            || (code == 500 && ALL_COPY.contains(&detail.as_str()));
                        showable && !detail.is_empty() && detail.len() <= MAX_MESSAGE_LEN
                    })
                    .unwrap_or_else(|| IMPORT_FAILED.to_owned())
            }
            Self::Transport { .. } | Self::Client(_) => VAULT_UNREACHABLE.to_owned(),
            Self::Decode { .. } => IMPORT_FAILED.to_owned(),
            Self::Invalid(message) => (*message).to_owned(),
        }
    }
}

/// A failed call as the line the dialog shows.
fn shown(error: ServerError) -> String {
    error.import_message()
}

impl ServerClient {
    /// `POST /api/import/files`: starts reading the picked files or folders
    /// in place. Answers 202 with the read; poll it with
    /// [`Self::import_job`].
    pub async fn read_export_files(&self, paths: Vec<String>) -> Result<ImportJob, String> {
        self.send_json(
            reqwest::Method::POST,
            FILES_PATH,
            Some(&ImportPaths { paths }),
        )
        .await
        .map_err(shown)
    }

    /// `GET /api/import/uploads/{id}`: where the read stands.
    pub async fn import_job(&self, id: &str) -> Result<ImportJob, String> {
        let path = checked(job_path(id)).map_err(shown)?;
        self.get_json(&path).await.map_err(shown)
    }

    /// `DELETE /api/import/uploads/{id}`: ends the read, or discards its
    /// batch.
    pub async fn discard_import(&self, id: &str) -> Result<(), String> {
        let path = checked(job_path(id)).map_err(shown)?;
        self.send_empty::<()>(reqwest::Method::DELETE, &path, None)
            .await
            .map_err(shown)
    }

    /// `GET /api/import/thread?source=&id=&upload=`: one conversation of a
    /// read batch, ready to become a thread.
    pub async fn export_thread(
        &self,
        source: ImportSource,
        upload: &str,
        id: &str,
    ) -> Result<ImportedThread, String> {
        let path = checked(thread_path(source, upload, id)).map_err(shown)?;
        self.get_json(&path).await.map_err(shown)
    }

    /// `GET /api/import/agents?source=&path=`: OpenClaw or Hermes Agent
    /// personas, in `path` or the agent's default places.
    #[cfg_attr(not(test), expect(dead_code, reason = "used from Task 13"))]
    pub async fn scan_agents(
        &self,
        source: ImportSource,
        path: Option<&str>,
    ) -> Result<AgentScan, String> {
        self.get_json(&agents_path(source, path))
            .await
            .map_err(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::{agents_path, job_path, thread_path, ServerClient, FILES_PATH};
    use bruhs_models::agent::Role;
    use bruhs_models::imports::{
        ImportBatch, ImportJob, ImportJobState, IMPORT_BUSY, IMPORT_FAILED, IMPORT_GONE,
    };
    use bruhs_models::logic::portable::{
        ImportCandidate, ImportSource, ImportedThread, PortableMessage, PortablePart,
    };
    use bruhs_models::EpochMs;

    const ID: &str = "0123456789abcdef0123456789abcdef";

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

    fn job(state: ImportJobState) -> String {
        serde_json::to_string(&ImportJob {
            id: ID.into(),
            state,
        })
        .unwrap()
    }

    fn thread() -> ImportedThread {
        ImportedThread::new(
            ImportCandidate {
                source: ImportSource::ChatGpt,
                id: "a b/c".into(),
                title: "Trip plan".into(),
                cwd: None,
                updated_at: EpochMs::saturating_from_i64(1_600_000_000_000),
                provider: None,
                session_id: None,
                model: None,
            },
            vec![PortableMessage {
                role: Role::User,
                parts: vec![PortablePart::Text("Where to?".into())],
                started_at_ms: None,
                model: None,
            }],
        )
    }

    #[test]
    fn paths_and_bodies_match_the_web() {
        let reading = ImportJobState::Reading {
            files_done: 0,
            files: 2,
            conversations: 0,
        };
        let ready = ImportJobState::Ready {
            batch: Box::new(ImportBatch {
                id: ID.into(),
                ..ImportBatch::default()
            }),
        };
        let busy = serde_json::json!({ "error": IMPORT_BUSY }).to_string();
        let requests = runtime().block_on(async {
            let (base, server) = serve(vec![
                ("202 Accepted", job(reading)),
                ("200 OK", job(ready)),
                ("204 No Content", String::new()),
                ("200 OK", serde_json::to_string(&thread()).unwrap()),
                ("200 OK", "{}".into()),
                ("200 OK", "{}".into()),
                ("409 Conflict", busy),
                (
                    "500 Internal Server Error",
                    r#"{"error":"disk on fire"}"#.into(),
                ),
            ])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let paths = vec![
                "/Users/me/Downloads/chatgpt export.zip".to_string(),
                "~/Downloads/grok".to_string(),
            ];
            let read = client.read_export_files(paths.clone()).await.unwrap();
            assert_eq!(read.id, ID);
            assert!(matches!(
                client.import_job(ID).await.unwrap().state,
                ImportJobState::Ready { .. }
            ));
            client.discard_import(ID).await.unwrap();
            let imported = client
                .export_thread(ImportSource::ChatGpt, ID, "a b/c")
                .await
                .unwrap();
            assert_eq!(imported.candidate.title, "Trip plan");
            client
                .scan_agents(ImportSource::OpenClaw, Some(" ~/my agent "))
                .await
                .unwrap();
            client
                .scan_agents(ImportSource::Hermes, None)
                .await
                .unwrap();
            // The server's sentence shows; a 500 that isn't import copy
            // shows the import's own failure line.
            assert_eq!(
                client.read_export_files(paths.clone()).await.unwrap_err(),
                IMPORT_BUSY
            );
            assert_eq!(client.import_job(ID).await.unwrap_err(), IMPORT_FAILED);
            // An id the server would refuse never reaches it.
            assert_eq!(client.import_job("../x").await.unwrap_err(), IMPORT_GONE);
            assert_eq!(client.discard_import("").await.unwrap_err(), IMPORT_GONE);
            assert_eq!(
                client
                    .export_thread(ImportSource::Grok, "../x", "g1")
                    .await
                    .unwrap_err(),
                IMPORT_GONE
            );
            let requests = server.await.unwrap();
            (requests, paths)
        });
        let (requests, paths) = requests;
        let lines: Vec<&str> = requests
            .iter()
            .map(|request| request.lines().next().unwrap())
            .collect();
        assert_eq!(
            lines,
            [
                &format!("POST {FILES_PATH} HTTP/1.1"),
                &format!("GET /api/import/uploads/{ID} HTTP/1.1"),
                &format!("DELETE /api/import/uploads/{ID} HTTP/1.1"),
                &format!("GET /api/import/thread?source=chatgpt&id=a%20b%2Fc&upload={ID} HTTP/1.1"),
                "GET /api/import/agents?source=openclaw&path=~%2Fmy%20agent HTTP/1.1",
                "GET /api/import/agents?source=hermes HTTP/1.1",
                &format!("POST {FILES_PATH} HTTP/1.1"),
                &format!("GET /api/import/uploads/{ID} HTTP/1.1"),
            ]
        );
        assert_eq!(body(&requests[0]), serde_json::json!({ "paths": paths }));
        // `/files` refuses any request that carries an `Origin`: a native
        // client sends none, on any import call.
        for request in &requests {
            assert!(
                !request.to_ascii_lowercase().contains("\r\norigin:"),
                "{request}"
            );
        }
    }

    #[test]
    fn paths_match_the_web_shapes() {
        assert_eq!(
            thread_path(ImportSource::Grok, ID, "g1").as_deref(),
            Some(format!("/api/import/thread?source=grok&id=g1&upload={ID}").as_str())
        );
        assert_eq!(
            agents_path(ImportSource::Hermes, Some("  ")),
            "/api/import/agents?source=hermes"
        );
        for bad in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcde",
            "../../api/vault/0123456789abcdef",
            "0123456789abcdef&upload=0123456789abcdef",
        ] {
            assert_eq!(thread_path(ImportSource::ChatGpt, bad, "x"), None, "{bad}");
            assert_eq!(job_path(bad), None, "{bad}");
        }
    }
}
