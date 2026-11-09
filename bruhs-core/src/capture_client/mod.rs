//! HTTP client for the bruhs-capture daemon. Used by bruhs-server to
//! detect the daemon, forward start/stop, and bridge the SSE stream.

use bruhs_models::{CentroidsFile, OcrObservation, TranscriptSegment};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Upper bound for one daemon request. Generous: voice enrollment runs a
/// model on the request path.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// The daemon is on loopback; a connect that takes longer means it is not
/// there.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Health probes answer immediately when the daemon is up.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Where the capture daemon listens and where it writes its per-launch
/// bearer token. Every request except `/health` must carry that token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpoint {
    base_url: String,
    token_file: Option<PathBuf>,
}

impl DaemonEndpoint {
    pub fn new(base_url: impl Into<String>, token_file: Option<PathBuf>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token_file,
        }
    }

    /// `BRUHS_CAPTURE_URL` (default `http://127.0.0.1:3002`) and
    /// `BRUHS_CAPTURE_TOKEN_FILE` (default: the daemon's app-support file).
    pub fn from_env() -> Self {
        let base_url = std::env::var("BRUHS_CAPTURE_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:3002".into());
        Self::new(base_url, token_file_from_env())
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The daemon URL for `segments`, each percent-encoded as one path
    /// segment (so an id like `../capture/stop` cannot change the route).
    pub fn url(&self, segments: &[&str]) -> Result<reqwest::Url, CaptureClientError> {
        let mut url =
            reqwest::Url::parse(&self.base_url).map_err(|_| CaptureClientError::InvalidUrl)?;
        url.path_segments_mut()
            .map_err(|()| CaptureClientError::InvalidUrl)?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// The token the running daemon wrote, re-read on every call because a
    /// restarted daemon writes a new one. `None` if the file is missing or
    /// malformed (the daemon then answers 401).
    pub async fn token(&self) -> Option<String> {
        let path = self.token_file.as_ref()?;
        match tokio::fs::read_to_string(path).await {
            Ok(contents) => {
                let token = bruhs_models::capture_auth::parse_token(&contents);
                if token.is_none() {
                    tracing::warn!("capture daemon token file is malformed: {}", path.display());
                }
                token.map(str::to_owned)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::warn!("capture daemon token unreadable: {error}");
                None
            }
        }
    }

    /// Attach `Authorization: Bearer <token>` when a token is available.
    pub async fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.token().await {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

/// `BRUHS_CAPTURE_TOKEN_FILE`, else the daemon's default token file.
pub fn token_file_from_env() -> Option<PathBuf> {
    std::env::var_os(bruhs_models::capture_auth::TOKEN_FILE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(default_token_file)
}

/// `<local data dir>/capture/daemon-token`, where the daemon writes it.
pub fn default_token_file() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai").map(|dirs| {
        dirs.data_local_dir()
            .join(bruhs_models::capture_auth::TOKEN_DIR)
            .join(bruhs_models::capture_auth::TOKEN_FILE)
    })
}

/// The process-wide daemon endpoint, read from the environment once.
pub fn endpoint() -> &'static DaemonEndpoint {
    static ENDPOINT: OnceLock<DaemonEndpoint> = OnceLock::new();
    ENDPOINT.get_or_init(DaemonEndpoint::from_env)
}

/// Base URL of the capture daemon (`BRUHS_CAPTURE_URL`, default
/// `http://127.0.0.1:3002`).
pub fn daemon_base_url() -> &'static str {
    endpoint().base_url()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub recording: bool,
    pub meeting_id: Option<String>,
    pub started_at_ms: Option<f64>,
    #[serde(default)]
    pub system_audio_permission: String,
    #[serde(default)]
    pub system_audio_capturing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartRequest {
    pub meeting_id: String,
    pub started_at_ms: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartResponse {
    pub meeting_id: String,
    pub wav_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StopResponse {
    pub meeting_id: String,
    pub ended_at_ms: f64,
    pub wav_path: String,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CaptureClientError {
    #[error("capture daemon not running")]
    NotRunning,
    #[error("http client unavailable: {0}")]
    ClientInit(#[source] Arc<reqwest::Error>),
    #[error("network: {0}")]
    Network(#[source] reqwest::Error),
    #[error("daemon {status}: {body}")]
    Status { status: u16, body: String },
    /// The daemon's reply was not the JSON expected.
    #[error("parse: {0}")]
    Parse(#[source] serde_json::Error),
    /// The meeting id cannot be sent to the daemon.
    #[error("parse: Invalid capture meeting identity.")]
    InvalidMeetingId,
    /// The transcript reply is larger than the client accepts.
    #[error("parse: Transcript response exceeds its size limit.")]
    TooLarge,
    /// The transcript reply was not the JSON expected.
    #[error("parse: Transcript response is malformed.")]
    MalformedTranscript(#[source] serde_json::Error),
    /// Decoding the transcript reply did not finish.
    #[error("parse: Transcript response could not be decoded.")]
    DecodeTask(#[source] tokio::task::JoinError),
    /// `BRUHS_CAPTURE_URL` is not a usable base URL.
    #[error("capture daemon URL is invalid")]
    InvalidUrl,
    /// The daemon's reply is larger than the client accepts.
    #[error("daemon reply exceeds its size limit")]
    ReplyTooLarge,
}

type SharedClient = OnceLock<Result<reqwest::Client, Arc<reqwest::Error>>>;

fn build(
    slot: &'static SharedClient,
    timeout: Duration,
) -> Result<&'static reqwest::Client, CaptureClientError> {
    slot.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(CONNECT_TIMEOUT.min(timeout))
            .build()
            .map_err(Arc::new)
    })
    .as_ref()
    .map_err(|e| CaptureClientError::ClientInit(Arc::clone(e)))
}

/// Shared client for general daemon requests. Built once per process.
fn client() -> Result<&'static reqwest::Client, CaptureClientError> {
    static CLIENT: SharedClient = OnceLock::new();
    build(&CLIENT, REQUEST_TIMEOUT)
}

/// Separate client with a short timeout used only for the health-check probe.
fn probe_client() -> Result<&'static reqwest::Client, CaptureClientError> {
    static PROBE_CLIENT: SharedClient = OnceLock::new();
    build(&PROBE_CLIENT, PROBE_TIMEOUT)
}

// ---------------------------------------------------------------------------
// Shared transport helpers — every public proxy below collapses through
// these, so adding a new daemon endpoint is 2 lines, not 25.
// ---------------------------------------------------------------------------

/// Map a reqwest network error to either NotRunning (when the daemon isn't
/// reachable) or Network (other transport failures). Centralizes the
/// connect/timeout-vs-other classification.
fn classify_transport(e: reqwest::Error) -> CaptureClientError {
    if e.is_connect() || e.is_timeout() {
        CaptureClientError::NotRunning
    } else {
        CaptureClientError::Network(e)
    }
}

/// Read the response status + body. Returns Status{status,body} for non-2xx
/// responses; otherwise yields the raw body string for parsing.
async fn read_body_or_status_err(resp: reqwest::Response) -> Result<String, CaptureClientError> {
    let status = resp.status();
    let body = resp.text().await.map_err(CaptureClientError::Network)?;
    if !status.is_success() {
        return Err(CaptureClientError::Status {
            status: status.as_u16(),
            body,
        });
    }
    Ok(body)
}

/// GET the daemon route `segments` and parse the JSON body as `T`.
async fn daemon_get_json<T: serde::de::DeserializeOwned>(
    segments: &[&str],
) -> Result<T, CaptureClientError> {
    daemon_get_json_from(endpoint(), segments).await
}

/// [`daemon_get_json`] against `daemon`.
async fn daemon_get_json_from<T: serde::de::DeserializeOwned>(
    daemon: &DaemonEndpoint,
    segments: &[&str],
) -> Result<T, CaptureClientError> {
    let request = client()?.get(daemon.url(segments)?);
    let resp = daemon
        .authorize(request)
        .await
        .send()
        .await
        .map_err(classify_transport)?;
    let body = read_body_or_status_err(resp).await?;
    serde_json::from_str(&body).map_err(CaptureClientError::Parse)
}

/// POST `request` (already carrying any body) and parse the JSON reply as `T`.
async fn daemon_post<T: serde::de::DeserializeOwned>(
    segments: &[&str],
    body: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> Result<T, CaptureClientError> {
    let daemon = endpoint();
    let request = body(client()?.post(daemon.url(segments)?));
    let resp = daemon
        .authorize(request)
        .await
        .send()
        .await
        .map_err(classify_transport)?;
    let body = read_body_or_status_err(resp).await?;
    serde_json::from_str(&body).map_err(CaptureClientError::Parse)
}

// ---------------------------------------------------------------------------
// Public API — each method delegates to the helpers above.
// ---------------------------------------------------------------------------

pub async fn is_running() -> bool {
    let Ok(probe) = probe_client() else {
        return false;
    };
    matches!(
        probe.get(format!("{}/health", daemon_base_url())).send().await,
        Ok(r) if r.status().is_success()
    )
}

pub async fn fetch_status() -> Result<DaemonStatus, CaptureClientError> {
    fetch_status_from(endpoint()).await
}

/// [`fetch_status`] from `daemon`.
pub async fn fetch_status_from(
    daemon: &DaemonEndpoint,
) -> Result<DaemonStatus, CaptureClientError> {
    daemon_get_json_from(daemon, &["status"]).await
}

pub async fn start_capture(req: &StartRequest) -> Result<StartResponse, CaptureClientError> {
    daemon_post(&["capture", "start"], |request| request.json(req)).await
}

pub async fn stop_capture() -> Result<StopResponse, CaptureClientError> {
    daemon_post(&["capture", "stop"], |request| request).await
}

pub async fn list_segments(meeting_id: &str) -> Result<Vec<TranscriptSegment>, CaptureClientError> {
    #[derive(Deserialize)]
    struct R {
        segments: Vec<TranscriptSegment>,
    }
    if meeting_id.is_empty()
        || meeting_id.len() > 128
        || !meeting_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CaptureClientError::InvalidMeetingId);
    }
    // Keep capture failure bounded and do not deserialize a large transcript on
    // an async executor worker. No unbounded generic JSON response helper here.
    let daemon = endpoint();
    let request = client()?
        .get(daemon.url(&["segments", meeting_id])?)
        .timeout(std::time::Duration::from_secs(3));
    let mut response = daemon
        .authorize(request)
        .await
        .send()
        .await
        .map_err(classify_transport)?;
    if !response.status().is_success() {
        return Err(CaptureClientError::Status {
            status: response.status().as_u16(),
            body: "Transcript unavailable.".into(),
        });
    }
    const LIMIT: usize = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(classify_transport)? {
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return Err(CaptureClientError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    tokio::task::spawn_blocking(move || {
        serde_json::from_slice::<R>(&bytes)
            .map(|reply| reply.segments)
            .map_err(CaptureClientError::MalformedTranscript)
    })
    .await
    .map_err(CaptureClientError::DecodeTask)?
}

pub async fn list_centroids(meeting_id: &str) -> Result<CentroidsFile, CaptureClientError> {
    daemon_get_json(&["centroids", meeting_id]).await
}

/// Fetch OCR observations from the capture daemon for a given meeting.
/// Returns an empty `Vec` when the file is absent (video OCR was disabled
/// or not yet complete). Non-fatal: callers should `unwrap_or_default()`.
pub async fn list_ocr_observations(
    meeting_id: &str,
) -> Result<Vec<OcrObservation>, CaptureClientError> {
    daemon_get_json(&["ocr", meeting_id]).await
}

/// POST /enroll/{person_id} — forward a WAV body to the daemon to extract
/// a voice embedding. Returns the L2-normalized embedding vector on success.
pub async fn enroll_voice(
    person_id: &str,
    wav_bytes: Vec<u8>,
) -> Result<Vec<f32>, CaptureClientError> {
    #[derive(Deserialize)]
    struct R {
        embedding: Vec<f32>,
    }
    let r: R = daemon_post(&["enroll", person_id], |request| {
        request.header("Content-Type", "audio/wav").body(wav_bytes)
    })
    .await?;
    Ok(r.embedding)
}

/// A daemon route bruhs-server forwards for the browser, which never holds
/// the daemon token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxiedRoute {
    /// `POST /capture/start`
    Start,
    /// `POST /capture/stop`
    Stop,
    /// `GET` / `POST /activity-monitor`
    ActivityMonitor,
}

impl ProxiedRoute {
    fn segments(self) -> &'static [&'static str] {
        match self {
            Self::Start => &["capture", "start"],
            Self::Stop => &["capture", "stop"],
            Self::ActivityMonitor => &["activity-monitor"],
        }
    }
}

/// The daemon's reply to a proxied request, passed back unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonReply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Largest daemon reply the proxy relays.
pub const PROXY_REPLY_LIMIT: usize = 64 * 1024;

/// Forward one browser request to the daemon with the bearer token attached.
/// `body` is `(content type, bytes)` for requests that carry one.
pub async fn proxy(
    daemon: &DaemonEndpoint,
    method: reqwest::Method,
    route: ProxiedRoute,
    body: Option<(String, Vec<u8>)>,
) -> Result<DaemonReply, CaptureClientError> {
    let mut request = client()?.request(method, daemon.url(route.segments())?);
    if let Some((content_type, bytes)) = body {
        request = request.header("Content-Type", content_type).body(bytes);
    }
    let mut response = daemon
        .authorize(request)
        .await
        .send()
        .await
        .map_err(classify_transport)?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(classify_transport)? {
        if bytes.len().saturating_add(chunk.len()) > PROXY_REPLY_LIMIT {
            return Err(CaptureClientError::ReplyTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(DaemonReply {
        status,
        content_type,
        body: bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = CaptureClientError::NotRunning;
        assert_eq!(format!("{e}"), "capture daemon not running");
    }

    /// Daemon requests used to have no timeout, so a wedged daemon hung the
    /// calling handler forever. Paused time jumps to the first timeout.
    #[tokio::test(start_paused = true)]
    async fn shared_client_times_out_stalled_requests() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        static SLOT: SharedClient = OnceLock::new();
        let client = build(&SLOT, REQUEST_TIMEOUT).unwrap();
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_delay(REQUEST_TIMEOUT * 10))
            .mount(&server)
            .await;
        let err = client.get(server.uri()).send().await.unwrap_err();
        assert!(err.is_timeout(), "{err}");
        assert!(matches!(
            classify_transport(err),
            CaptureClientError::NotRunning
        ));
    }

    fn token_file(dir: &tempfile::TempDir, contents: &str) -> PathBuf {
        let path = dir.path().join("daemon-token");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn ids_cannot_change_the_daemon_route() {
        let daemon = DaemonEndpoint::new("http://127.0.0.1:3002/", None);
        assert_eq!(
            daemon.url(&["enroll", "../capture/stop"]).unwrap().as_str(),
            "http://127.0.0.1:3002/enroll/..%2Fcapture%2Fstop"
        );
        assert_eq!(
            daemon.url(&["capture", "start"]).unwrap().as_str(),
            "http://127.0.0.1:3002/capture/start"
        );
        assert!(matches!(
            DaemonEndpoint::new("not a url", None).url(&["status"]),
            Err(CaptureClientError::InvalidUrl)
        ));
    }

    #[tokio::test]
    async fn token_is_read_fresh_and_must_have_the_daemon_shape() {
        let dir = tempfile::tempdir().unwrap();
        let first = "a".repeat(64);
        let path = token_file(&dir, &format!("{first}\n"));
        let daemon = DaemonEndpoint::new("http://127.0.0.1:1", Some(path.clone()));
        assert_eq!(daemon.token().await.as_deref(), Some(first.as_str()));
        // A restarted daemon writes a new token; the next request uses it.
        let second = "b".repeat(64);
        std::fs::write(&path, &second).unwrap();
        assert_eq!(daemon.token().await.as_deref(), Some(second.as_str()));
        std::fs::write(&path, "not-a-token").unwrap();
        assert_eq!(daemon.token().await, None);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(daemon.token().await, None);
        assert_eq!(DaemonEndpoint::new("http://x", None).token().await, None);
    }

    #[tokio::test]
    async fn proxy_attaches_the_token_and_relays_the_reply_unchanged() {
        use wiremock::matchers::{body_string, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let token = "c".repeat(64);
        Mock::given(method("POST"))
            .and(path("/capture/start"))
            .and(header("authorization", format!("Bearer {token}").as_str()))
            .and(header("content-type", "application/json"))
            .and(body_string(r#"{"meeting_id":"m"}"#))
            .respond_with(
                ResponseTemplate::new(409)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("already recording"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let daemon = DaemonEndpoint::new(server.uri(), Some(token_file(&dir, &token)));
        let reply = proxy(
            &daemon,
            reqwest::Method::POST,
            ProxiedRoute::Start,
            Some(("application/json".into(), br#"{"meeting_id":"m"}"#.to_vec())),
        )
        .await
        .unwrap();
        assert_eq!(
            reply,
            DaemonReply {
                status: 409,
                content_type: Some("text/plain".into()),
                body: b"already recording".to_vec(),
            }
        );
    }

    #[tokio::test]
    async fn oversized_proxy_reply_is_refused() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b'x'; PROXY_REPLY_LIMIT + 1]),
            )
            .mount(&server)
            .await;
        let daemon = DaemonEndpoint::new(server.uri(), None);
        let error = proxy(
            &daemon,
            reqwest::Method::GET,
            ProxiedRoute::ActivityMonitor,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, CaptureClientError::ReplyTooLarge));
    }

    #[tokio::test]
    async fn is_running_returns_false_when_daemon_down() {
        // Assumes the daemon isn't running in CI / dev test envs.
        let _ = is_running().await;
    }
}
