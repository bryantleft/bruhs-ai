//! HTTP client for bruhs-server, the same API surface `bruhs-web/src/api.rs`
//! calls for meetings, people, Google Calendar, the capture daemon and the
//! two SSE streams. The desktop runs bruhs-core in-process for everything
//! else; these features go through the server because the capture daemon
//! posts into it and it owns the meeting / people store and the fan-out.
//!
//! Every call runs on the desktop's tokio runtime and fails with a
//! [`ServerError`] that says whether the server was unreachable, answered
//! with a failure status, or answered with something unexpected.

use std::sync::Arc;
use std::time::Duration;

use bruhs_models::{
    Calendar, GoogleCalendarSyncState, Meeting, MeetingSummary, PersonProfile, TranscriptSegment,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;
use tokio_stream::StreamExt;

/// Web `BASE_URL` default; `BRUHS_SERVER_URL` overrides at runtime.
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:3001";
/// Capture daemon default; `BRUHS_CAPTURE_URL` overrides. The desktop talks
/// to the daemon directly with the bearer token from the daemon's token file
/// (the web goes through the server's `/api/capture/*` proxy instead).
pub const DEFAULT_CAPTURE_URL: &str = "http://127.0.0.1:3002";

/// Web `meetings_stream.rs` reconnect backoff: 1s doubling to 30s.
pub const SSE_BACKOFF_INITIAL_MS: u64 = 1_000;
pub const SSE_BACKOFF_MAX_MS: u64 = 30_000;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Bytes of a failure response kept for the log.
const ERROR_BODY_LOG_BYTES: usize = 4 * 1024;
const SUMMARY_REQUEST_TIMEOUT: Duration = Duration::from_secs(135);
const MEETING_READBACK_TIMEOUT: Duration = Duration::from_secs(15);

use bruhs_models::api::{CancelSummaryGenerationBody, StopMeetingBody, SummarizeRequest};
pub use bruhs_models::api::{
    CaptureStartBody, CaptureStatus, CreateMeetingBody, StartOAuthResponse, Thread,
};
use bruhs_models::time::EpochMs;

/// A transcript read plus the chunk ids the server will not restore.
pub struct TranscriptRefresh {
    pub segments: Vec<TranscriptSegment>,
    pub removed: Vec<String>,
}

/// Why a request to bruhs-server (or the capture daemon) failed.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// No HTTP answer at all: refused, reset, timed out, unresolvable. The
    /// only failure that means the server is offline.
    #[error("{request} failed: {source}")]
    Transport {
        request: String,
        #[source]
        source: reqwest::Error,
    },
    /// The server answered with a failure status. The body is kept for the
    /// log only; it is never shown to the user.
    #[error("{request} → {}", .code.as_u16())]
    Status {
        request: String,
        code: reqwest::StatusCode,
        body: String,
    },
    /// The server answered success with a body that is not what was asked
    /// for.
    #[error("{request}: unexpected response")]
    Decode {
        request: String,
        #[source]
        source: reqwest::Error,
    },
    /// Refused before anything was sent.
    #[error("{0}")]
    Invalid(&'static str),
    /// The HTTP client could not be created, so nothing can be sent.
    #[error("the HTTP client is unavailable: {0}")]
    Client(Arc<reqwest::Error>),
}

impl ServerError {
    /// The server (or daemon) could not be reached.
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Transport { .. })
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Status { code, .. } if *code == reqwest::StatusCode::NOT_FOUND)
    }

    /// The server answered 502: something behind it (the capture daemon)
    /// answered but could not be used.
    pub fn is_bad_gateway(&self) -> bool {
        matches!(self, Self::Status { code, .. } if *code == reqwest::StatusCode::BAD_GATEWAY)
    }
}

#[derive(Clone)]
pub struct ServerClient {
    base: String,
    capture: bruhs_core::capture_client::DaemonEndpoint,
    /// `Err` when the client could not be built; every request then fails
    /// with [`ServerError::Client`].
    http: Result<reqwest::Client, Arc<reqwest::Error>>,
}

impl ServerClient {
    /// Private metadata may only reach a literal loopback address. Do not follow
    /// redirects or environment proxies, including on readback.
    pub fn local_context_url(&self, id: &str) -> Result<reqwest::Url, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || url.port().is_none()
        {
            return Err(ServerError::Invalid("Local app context requires an HTTP server on 127.0.0.1 or [::1] with an explicit port."));
        }
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid server URL"))?
            .pop_if_empty()
            .extend(["api", "meetings", id, "local-focus"]);
        Ok(url)
    }

    async fn local_context_request(
        &self,
        id: &str,
        sample: Option<&bruhs_models::LocalFocusObservation>,
    ) -> Result<reqwest::Response, ServerError> {
        const REQUEST: &str = "Local context request";
        let url = self.local_context_url(id)?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ServerError::Client(Arc::new(error)))?;
        let request = if let Some(sample) = sample {
            client.post(url).json(sample)
        } else {
            client.get(url)
        };
        // Private metadata: the request names a fixed label, never the URL,
        // and a failure keeps no response body.
        let response = request
            .header("x-bruhs-local-context", "1")
            .send()
            .await
            .map_err(|error| ServerError::Transport {
                request: REQUEST.into(),
                source: error.without_url(),
            })?;
        if !response.status().is_success() {
            return Err(ServerError::Status {
                request: REQUEST.into(),
                code: response.status(),
                body: String::new(),
            });
        }
        Ok(response)
    }

    pub async fn append_local_focus(
        &self,
        sample: &bruhs_models::LocalFocusObservation,
    ) -> Result<(), ServerError> {
        self.local_context_request(&sample.meeting_id, Some(sample))
            .await?;
        Ok(())
    }

    pub async fn list_local_focus(
        &self,
        id: &str,
    ) -> Result<Vec<bruhs_models::LocalFocusObservation>, ServerError> {
        self.local_context_request(id, None)
            .await?
            .json()
            .await
            .map_err(|error| ServerError::Decode {
                request: "Local context request".into(),
                source: error.without_url(),
            })
    }
    /// Reads `BRUHS_SERVER_URL` / `BRUHS_CAPTURE_URL`, trailing slashes trimmed.
    ///
    /// # Errors
    ///
    /// [`ServerError::Client`] when the HTTP client cannot be built.
    pub fn from_env() -> Result<Self, ServerError> {
        let base = std::env::var("BRUHS_SERVER_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string());
        let capture_base = std::env::var("BRUHS_CAPTURE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CAPTURE_URL.to_string());
        Ok(Self::new(base, capture_base)?
            .with_capture_token_file(bruhs_core::capture_client::token_file_from_env()))
    }

    /// A client for the server at `base` and the capture daemon at
    /// `capture_base`. Every request has a timeout.
    ///
    /// # Errors
    ///
    /// [`ServerError::Client`] when the HTTP client cannot be built.
    pub fn new(
        base: impl Into<String>,
        capture_base: impl Into<String>,
    ) -> Result<Self, ServerError> {
        let http = Self::build_http().map_err(|error| ServerError::Client(Arc::new(error)))?;
        Ok(Self::with_http(base, capture_base, Ok(http)))
    }

    /// A client whose every request fails with [`ServerError::Client`]: the
    /// app keeps running (meetings show as unavailable) when `from_env`
    /// could not build one.
    pub fn unavailable(error: &ServerError) -> Self {
        let cause = match error {
            ServerError::Client(cause) => Arc::clone(cause),
            // Only a build failure leads here; anything else is reported as
            // a fresh build attempt's error.
            _ => match Self::build_http() {
                Err(cause) => Arc::new(cause),
                Ok(http) => {
                    return Self::with_http(DEFAULT_SERVER_URL, DEFAULT_CAPTURE_URL, Ok(http))
                }
            },
        };
        Self::with_http(DEFAULT_SERVER_URL, DEFAULT_CAPTURE_URL, Err(cause))
    }

    fn build_http() -> Result<reqwest::Client, reqwest::Error> {
        reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()
    }

    fn with_http(
        base: impl Into<String>,
        capture_base: impl Into<String>,
        http: Result<reqwest::Client, Arc<reqwest::Error>>,
    ) -> Self {
        Self {
            base: normalize_base(&base.into()),
            capture: bruhs_core::capture_client::DaemonEndpoint::new(
                normalize_base(&capture_base.into()),
                None,
            ),
            http,
        }
    }

    fn http(&self) -> Result<&reqwest::Client, ServerError> {
        self.http
            .as_ref()
            .map_err(|error| ServerError::Client(Arc::clone(error)))
    }

    /// Where to read the capture daemon's bearer token (none: send no token).
    pub fn with_capture_token_file(mut self, token_file: Option<std::path::PathBuf>) -> Self {
        self.capture = bruhs_core::capture_client::DaemonEndpoint::new(
            self.capture.base_url().to_owned(),
            token_file,
        );
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// The response, or why there is none worth reading: no answer at all
    /// ([`ServerError::Transport`]) or a failure status
    /// ([`ServerError::Status`], body kept for the log).
    async fn check(
        resp: Result<reqwest::Response, reqwest::Error>,
        request: &str,
    ) -> Result<reqwest::Response, ServerError> {
        let resp = resp.map_err(|source| ServerError::Transport {
            request: request.to_string(),
            source,
        })?;
        let code = resp.status();
        if code.is_success() {
            return Ok(resp);
        }
        let body = match resp.text().await {
            Ok(mut body) => {
                bruhs_models::logic::text::truncate_utf8(&mut body, ERROR_BODY_LOG_BYTES);
                body
            }
            Err(error) => format!("(body unreadable: {error})"),
        };
        tracing::debug!("{request} → {}: {body}", code.as_u16());
        Err(ServerError::Status {
            request: request.to_string(),
            code,
            body,
        })
    }

    async fn decode<T: DeserializeOwned>(
        resp: reqwest::Response,
        request: &str,
    ) -> Result<T, ServerError> {
        resp.json::<T>()
            .await
            .map_err(|source| ServerError::Decode {
                request: request.to_string(),
                source,
            })
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ServerError> {
        let request = format!("GET {path}");
        let resp = Self::check(self.http()?.get(self.url(path)).send().await, &request).await?;
        Self::decode(resp, &request).await
    }

    async fn send_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ServerError> {
        self.send_json_with_timeout(method, path, body, REQUEST_TIMEOUT)
            .await
    }

    async fn send_json_with_timeout<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
        timeout: Duration,
    ) -> Result<T, ServerError> {
        let label = format!("{method} {path}");
        let mut request = self
            .http()?
            .request(method, self.url(path))
            .timeout(timeout);
        if let Some(body) = body {
            request = request.json(body);
        }
        let resp = Self::check(request.send().await, &label).await?;
        Self::decode(resp, &label).await
    }

    async fn send_empty<B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<(), ServerError> {
        let label = format!("{method} {path}");
        let mut request = self.http()?.request(method, self.url(path));
        if let Some(body) = body {
            request = request.json(body);
        }
        Self::check(request.send().await, &label).await.map(|_| ())
    }

    /// `path` under `/api/meetings/`, each segment percent-encoded.
    fn meetings_url(&self, segments: &[&str]) -> Result<reqwest::Url, ServerError> {
        let mut url = reqwest::Url::parse(&self.url("/api/meetings/"))
            .map_err(|_| ServerError::Invalid("Invalid server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid server URL"))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// `Ok(None)` when the server answers 404.
    async fn found<T>(result: Result<T, ServerError>) -> Result<Option<T>, ServerError> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error),
        }
    }

    // ── Meetings ────────────────────────────────────────────────────────

    pub async fn fetch_meetings(&self) -> Result<Vec<Meeting>, ServerError> {
        self.get_json("/api/meetings").await
    }

    /// `Ok(None)` on 404 so callers can tell missing from failed.
    pub async fn get_meeting(&self, id: &str) -> Result<Option<Meeting>, ServerError> {
        let path = format!("/api/meetings/{id}");
        let request = format!("GET {path}");
        let resp = self
            .http()?
            .get(self.url(&path))
            .timeout(MEETING_READBACK_TIMEOUT)
            .send()
            .await;
        let meeting = match Self::check(resp, &request).await {
            Ok(resp) => Self::decode(resp, &request).await,
            Err(error) => Err(error),
        };
        Self::found(meeting).await
    }

    pub async fn create_meeting(&self, body: CreateMeetingBody) -> Result<Meeting, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/meetings", Some(&body))
            .await
    }

    /// Discover an unresolved request whose client kept no identity. `Err` for an
    /// unknown meeting or transport failure; never treat that as "none pending".
    pub async fn pending_summary_generation(
        &self,
        id: &str,
    ) -> Result<Option<bruhs_models::SummaryGenerationReceipt>, ServerError> {
        const REQUEST: &str = "GET unresolved generation";
        let url = self.meetings_url(&[id, "generations"])?;
        let response = self
            .http()?
            .get(url)
            .timeout(MEETING_READBACK_TIMEOUT)
            .send()
            .await;
        let response = Self::check(response, REQUEST).await?;
        Self::decode(response, REQUEST).await
    }

    /// Fence the exact saved request, including one the server never received.
    /// `None` means the meeting is unknown there; the saved request must be retained.
    pub async fn cancel_summary_generation(
        &self,
        request: &bruhs_models::SummaryGenerationRequest,
    ) -> Result<Option<bruhs_models::SummaryGenerationReceipt>, ServerError> {
        const REQUEST: &str = "DELETE generation request";
        let url = self.meetings_url(&[&request.meeting_id, "generations", &request.request_id])?;
        let response = self
            .http()?
            .delete(url)
            .timeout(MEETING_READBACK_TIMEOUT)
            .json(&CancelSummaryGenerationBody {
                extract_observations: request.extract_observations,
                replace_observations: request.replace_observations,
            })
            .send()
            .await;
        let receipt = match Self::check(response, REQUEST).await {
            Ok(response) => Self::decode(response, REQUEST).await,
            Err(error) => Err(error),
        };
        Self::found(receipt).await
    }

    /// A missing receipt does not prove a delayed generation request cannot arrive.
    pub async fn summary_generation(
        &self,
        id: &str,
        request_id: &str,
    ) -> Result<Option<bruhs_models::SummaryGenerationReceipt>, ServerError> {
        const REQUEST: &str = "GET generation receipt";
        let url = self.meetings_url(&[id, "generations", request_id])?;
        let response = self
            .http()?
            .get(url)
            .timeout(MEETING_READBACK_TIMEOUT)
            .send()
            .await;
        let receipt = match Self::check(response, REQUEST).await {
            Ok(response) => Self::decode(response, REQUEST).await,
            Err(error) => Err(error),
        };
        Self::found(receipt).await
    }

    pub async fn stop_meeting(&self, id: &str, ended_at_ms: f64) -> Result<Meeting, ServerError> {
        let ended_at_ms = EpochMs::from_f64_floor(ended_at_ms)
            .map_err(|_| ServerError::Invalid("The meeting's end time is not a valid time."))?;
        self.send_json(
            reqwest::Method::POST,
            &format!("/api/meetings/{id}/stop"),
            Some(&StopMeetingBody { ended_at_ms }),
        )
        .await
    }

    pub async fn update_speaker_label(
        &self,
        meeting_id: &str,
        speaker_id: &str,
        label: String,
    ) -> Result<Meeting, ServerError> {
        self.send_json(
            reqwest::Method::PATCH,
            &format!("/api/meetings/{meeting_id}/speakers/{speaker_id}"),
            Some(&serde_json::json!({ "label": label })),
        )
        .await
    }

    pub async fn link_speaker_to_person(
        &self,
        meeting_id: &str,
        speaker_id: &str,
        person_id: &str,
    ) -> Result<Meeting, ServerError> {
        self.send_json::<Meeting, ()>(
            reqwest::Method::POST,
            &format!("/api/meetings/{meeting_id}/speakers/{speaker_id}/link/{person_id}"),
            None,
        )
        .await
    }

    /// `POST /api/meetings/{id}/summarize`; `replace` requests atomic replacement
    /// of this meeting's observations with the generated summary.
    /// `request_id` must come from the acknowledged local intent checkpoint.
    pub async fn summarize_meeting(
        &self,
        id: &str,
        request_id: &str,
        extract_observations: bool,
        replace: bool,
    ) -> Result<MeetingSummary, ServerError> {
        self.send_json_with_timeout(
            reqwest::Method::POST,
            &format!("/api/meetings/{id}/summarize"),
            Some(&SummarizeRequest {
                request_id: Some(request_id.to_string()),
                extract_observations,
                replace_observations: replace,
            }),
            SUMMARY_REQUEST_TIMEOUT,
        )
        .await
    }

    pub async fn set_meeting_skip_observations(
        &self,
        meeting_id: &str,
        skip: bool,
    ) -> Result<Meeting, ServerError> {
        self.send_json_with_timeout(
            reqwest::Method::PATCH,
            &format!("/api/meetings/{meeting_id}/skip_observations"),
            Some(&serde_json::json!({ "skip": skip })),
            MEETING_READBACK_TIMEOUT,
        )
        .await
    }

    pub async fn list_segments(
        &self,
        meeting_id: &str,
    ) -> Result<Vec<TranscriptSegment>, ServerError> {
        self.get_json(&format!("/api/meetings/{meeting_id}/segments"))
            .await
    }

    pub async fn remove_transcript_segment(
        &self,
        meeting_id: &str,
        segment_id: &str,
    ) -> Result<(), ServerError> {
        let url = self.meetings_url(&[meeting_id, "segments", segment_id])?;
        let request = format!("DELETE {url}");
        Self::check(self.http()?.delete(url).send().await, &request)
            .await
            .map(|_| ())
    }

    pub async fn removed_segment_ids(&self, meeting_id: &str) -> Result<Vec<String>, ServerError> {
        let url = self.meetings_url(&[meeting_id, "segment-removals"])?;
        let request = format!("GET {url}");
        let resp = Self::check(self.http()?.get(url).send().await, &request).await?;
        Self::decode(resp, &request).await
    }

    /// Segments and the ids capture must not put back. Both have to succeed.
    pub async fn refresh_transcript(
        &self,
        meeting_id: &str,
    ) -> Result<TranscriptRefresh, ServerError> {
        let (segments, removed) = tokio::join!(
            self.list_segments(meeting_id),
            self.removed_segment_ids(meeting_id),
        );
        Ok(TranscriptRefresh {
            segments: segments?,
            removed: removed?,
        })
    }

    pub async fn create_or_get_meeting_chat(
        &self,
        meeting_id: &str,
    ) -> Result<Thread, ServerError> {
        self.send_json::<Thread, ()>(
            reqwest::Method::POST,
            &format!("/api/meetings/{meeting_id}/chat"),
            None,
        )
        .await
    }

    // ── People ──────────────────────────────────────────────────────────

    pub async fn list_people(&self) -> Result<Vec<PersonProfile>, ServerError> {
        self.get_json("/api/people").await
    }

    pub async fn resolve_person_merges(
        &self,
        ids: &[String],
    ) -> Result<Vec<bruhs_models::PersonMergeResolution>, ServerError> {
        if ids.len() > 64 || ids.iter().any(|id| id.trim().is_empty()) {
            return Err(ServerError::Invalid("Invalid contact merge lookup batch."));
        }
        let resolved: Vec<bruhs_models::PersonMergeResolution> = self
            .send_json(
                reqwest::Method::POST,
                "/api/people/resolve-merges",
                Some(&serde_json::json!({"ids":ids})),
            )
            .await?;
        let mut seen = std::collections::HashSet::new();
        if resolved.len() > ids.len()
            || resolved.iter().any(|item| {
                !ids.contains(&item.source_id)
                    || !seen.insert(&item.source_id)
                    || item.source_id == item.target.id
                    || item.target.id.trim().is_empty()
                    || item.target.display_name.trim().is_empty()
                    || item
                        .target
                        .voice_embedding
                        .as_ref()
                        .is_some_and(|values| values.iter().any(|v| !v.is_finite()))
            })
        {
            return Err(ServerError::Invalid(
                "Contact merge lookup returned invalid identities; local records were retained.",
            ));
        }
        Ok(resolved)
    }

    pub async fn create_person(
        &self,
        person: &PersonProfile,
    ) -> Result<PersonProfile, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/people", Some(person))
            .await
    }

    pub async fn rename_person(&self, id: &str, name: &str) -> Result<PersonProfile, ServerError> {
        self.send_json(
            reqwest::Method::PATCH,
            &format!("/api/people/{id}/rename"),
            Some(&serde_json::json!({ "display_name": name })),
        )
        .await
    }

    pub async fn merge_people(
        &self,
        source_id: &str,
        target_id: &str,
    ) -> Result<PersonProfile, ServerError> {
        self.send_json::<PersonProfile, ()>(
            reqwest::Method::POST,
            &format!("/api/people/{source_id}/merge_into/{target_id}"),
            None,
        )
        .await
    }

    pub async fn set_person_tracked(
        &self,
        id: &str,
        tracked: bool,
    ) -> Result<PersonProfile, ServerError> {
        self.send_json(
            reqwest::Method::PATCH,
            &format!("/api/people/{id}/tracked"),
            Some(&serde_json::json!({ "tracked": tracked })),
        )
        .await
    }

    /// `POST /api/people/{id}/enroll` with a 16-bit mono WAV body.
    pub async fn enroll_voice(&self, id: &str, wav: Vec<u8>) -> Result<PersonProfile, ServerError> {
        let path = format!("/api/people/{id}/enroll");
        let request = format!("POST {path}");
        let resp = Self::check(
            self.http()?
                .post(self.url(&path))
                .header("Content-Type", "audio/wav")
                .body(wav)
                .send()
                .await,
            &request,
        )
        .await?;
        Self::decode(resp, &request).await
    }

    // ── Google Calendar ─────────────────────────────────────────────────

    pub async fn fetch_google_sync_state(&self) -> Result<GoogleCalendarSyncState, ServerError> {
        self.get_json("/api/calendar/google/state").await
    }

    pub async fn start_google_oauth(&self) -> Result<StartOAuthResponse, ServerError> {
        self.send_json::<StartOAuthResponse, ()>(
            reqwest::Method::POST,
            "/api/calendar/google/oauth/start",
            None,
        )
        .await
    }

    pub async fn disconnect_google(&self) -> Result<(), ServerError> {
        self.send_empty::<()>(
            reqwest::Method::POST,
            "/api/calendar/google/disconnect",
            None,
        )
        .await
    }

    pub async fn force_google_sync(&self) -> Result<(), ServerError> {
        self.send_empty::<()>(reqwest::Method::POST, "/api/calendar/google/sync", None)
            .await
    }

    pub async fn fetch_google_calendars(&self) -> Result<Vec<Calendar>, ServerError> {
        self.get_json("/api/calendar/google/calendars").await
    }

    pub async fn patch_google_calendar(
        &self,
        id: &str,
        sync_enabled: bool,
    ) -> Result<Calendar, ServerError> {
        self.send_json(
            reqwest::Method::PATCH,
            &format!("/api/calendar/google/calendars/{id}"),
            Some(&serde_json::json!({ "sync_enabled": sync_enabled })),
        )
        .await
    }

    // ── Capture daemon ──────────────────────────────────────────────────

    pub async fn fetch_capture_status(&self) -> Result<CaptureStatus, ServerError> {
        self.get_json("/api/capture/status").await
    }

    pub async fn activity_monitor(
        &self,
        command: Option<&bruhs_models::ActivityMonitorCommand>,
    ) -> Result<bruhs_models::ActivityMonitorStatus, ServerError> {
        const REQUEST: &str = "Microphone activity permission";
        let url = format!("{}/activity-monitor", self.capture.base_url());
        let http = self.http()?;
        let request = if let Some(command) = command {
            http.post(url).json(command)
        } else {
            http.get(url)
        };
        let request = self.capture.authorize(request).await;
        let resp = Self::check(
            request
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await,
            REQUEST,
        )
        .await?;
        Self::decode(resp, REQUEST).await
    }

    /// The daemon is spoken to directly for start/stop, with its token.
    pub async fn capture_start(&self, body: CaptureStartBody) -> Result<(), ServerError> {
        let url = format!("{}/capture/start", self.capture.base_url());
        let request = self
            .capture
            .authorize(self.http()?.post(url).json(&body))
            .await;
        Self::check(request.send().await, "POST /capture/start")
            .await
            .map(|_| ())
    }

    pub async fn capture_stop(&self) -> Result<(), ServerError> {
        let url = format!("{}/capture/stop", self.capture.base_url());
        let request = self.capture.authorize(self.http()?.post(url)).await;
        Self::check(request.send().await, "POST /capture/stop")
            .await
            .map(|_| ())
    }

    // ── Server-sent events ──────────────────────────────────────────────

    /// Read `path` as an SSE stream forever, sending every decoded `data:`
    /// payload to `tx`; reconnects with the web's backoff. Before each retry
    /// `tx` receives [`SseMessage::Disconnected`] (for the "reconnecting…"
    /// notice); a payload that does not decode is logged and, once per
    /// connection, reported as [`SseMessage::Undecodable`] so the UI can say
    /// its data may be stale. Runs until `tx` is closed.
    pub async fn run_sse<E>(self: Arc<Self>, path: String, tx: UnboundedSender<SseMessage<E>>)
    where
        E: DeserializeOwned + Send + 'static,
    {
        let mut backoff_ms = SSE_BACKOFF_INITIAL_MS;
        loop {
            if tx.is_closed() {
                return;
            }
            match self.http() {
                Ok(http) => {
                    let request = http
                        .get(self.url(&path))
                        .timeout(Duration::from_secs(24 * 60 * 60))
                        .header("Accept", "text/event-stream");
                    match request.send().await {
                        Ok(resp) if resp.status().is_success() => {
                            let delivered = Self::read_sse(resp, &path, &tx, &mut backoff_ms).await;
                            if delivered.is_err() {
                                return;
                            }
                        }
                        Ok(resp) => {
                            tracing::warn!("GET {path} → {}", resp.status().as_u16());
                        }
                        Err(error) => tracing::debug!("GET {path} failed: {error}"),
                    }
                }
                Err(error) => tracing::warn!("GET {path}: {error}"),
            }
            if tx.is_closed() {
                return;
            }
            let delay = backoff_ms;
            backoff_ms = (backoff_ms * 2).min(SSE_BACKOFF_MAX_MS);
            if tx.send(SseMessage::Disconnected).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }

    /// Forward one connection's events until it ends. `Err` once `tx` is
    /// closed (nobody is listening any more).
    async fn read_sse<E: DeserializeOwned>(
        resp: reqwest::Response,
        path: &str,
        tx: &UnboundedSender<SseMessage<E>>,
        backoff_ms: &mut u64,
    ) -> Result<(), ()> {
        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::default();
        let mut reported_undecodable = false;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    tracing::debug!("GET {path}: stream ended: {error}");
                    return Ok(());
                }
            };
            let payloads = match parser.push(&chunk) {
                Ok(payloads) => payloads,
                Err(error) => {
                    // Reconnect rather than buffer without bound.
                    tracing::warn!("GET {path}: {error}; reconnecting");
                    return Ok(());
                }
            };
            for payload in payloads {
                *backoff_ms = SSE_BACKOFF_INITIAL_MS;
                match serde_json::from_str::<E>(&payload) {
                    Ok(event) => tx.send(SseMessage::Event(event)).map_err(|_| ())?,
                    Err(error) => {
                        let mut sample = payload;
                        bruhs_models::logic::text::truncate_utf8(&mut sample, 256);
                        tracing::warn!("GET {path}: undecodable event ({error}): {sample}");
                        if !reported_undecodable {
                            reported_undecodable = true;
                            tx.send(SseMessage::Undecodable).map_err(|_| ())?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// One message from an SSE task.
pub enum SseMessage<E> {
    Event(E),
    /// The stream dropped; the task reconnects with backoff.
    Disconnected,
    /// The server sent an event this build cannot read (first one per
    /// connection); what the UI shows may be out of date.
    Undecodable,
}

/// Longest SSE line or event the parser holds; a stream that sends more
/// without a line break is dropped and reconnected.
pub const SSE_MAX_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error(
    "event stream sent more than {} bytes without completing an event",
    SSE_MAX_EVENT_BYTES
)]
pub struct SseOverflow;

/// Incremental `text/event-stream` decoder: yields the `data:` payload of
/// each event (multi-line data joined with `\n`), ignoring comments,
/// `event:`/`id:`/`retry:` fields and heartbeats.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    scanned: usize,
    data_lines: Vec<String>,
    data_bytes: usize,
}

impl SseParser {
    /// Feed `chunk`; returns the events it completed.
    ///
    /// # Errors
    ///
    /// [`SseOverflow`] once a partial line or an event's data passes
    /// [`SSE_MAX_EVENT_BYTES`]; the parser should then be discarded.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, SseOverflow> {
        // Network chunks can split a UTF-8 code point. Decode only complete lines.
        self.buffer.extend_from_slice(chunk);
        let mut out = Vec::new();
        let mut consumed = 0;
        while let Some(offset) = self.buffer[self.scanned..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let newline = self.scanned + offset;
            let line = String::from_utf8_lossy(&self.buffer[consumed..newline])
                .trim_end_matches('\r')
                .to_string();
            self.scanned = newline + 1;
            consumed = self.scanned;
            if line.is_empty() {
                if !self.data_lines.is_empty() {
                    out.push(self.data_lines.join("\n"));
                    self.data_lines.clear();
                    self.data_bytes = 0;
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let data = rest.strip_prefix(' ').unwrap_or(rest);
                self.data_bytes += data.len();
                if self.data_bytes > SSE_MAX_EVENT_BYTES {
                    return Err(SseOverflow);
                }
                self.data_lines.push(data.to_string());
            }
        }
        // Compact once per chunk and don't rescan the retained partial line.
        self.buffer.drain(..consumed);
        self.scanned = self.buffer.len();
        if self.buffer.len() > SSE_MAX_EVENT_BYTES {
            return Err(SseOverflow);
        }
        Ok(out)
    }
}

fn normalize_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    /// The desktop talks to the capture daemon directly, so it must send the
    /// daemon's per-launch bearer token, read from the token file.
    #[test]
    fn capture_requests_carry_the_daemon_token() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let token_file =
            std::env::temp_dir().join(format!("bruhs-desktop-token-{}", uuid::Uuid::new_v4()));
        let token = "d".repeat(64);
        std::fs::write(&token_file, &token).unwrap();
        let heads = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let daemon = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let mut heads = Vec::new();
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut buffer = vec![0; 16 * 1024];
                    let read = socket.read(&mut buffer).await.unwrap();
                    heads.push(String::from_utf8_lossy(&buffer[..read]).to_lowercase());
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                        )
                        .await
                        .unwrap();
                }
                heads
            });
            let client = super::ServerClient::new("http://127.0.0.1:9", &daemon)
                .unwrap()
                .with_capture_token_file(Some(token_file.clone()));
            client
                .capture_start(super::CaptureStartBody {
                    meeting_id: "m-1".parse().unwrap(),
                    started_at_ms: bruhs_models::time::EpochMs::new(1).unwrap(),
                    model: None,
                    live_model: None,
                    subspeaker_enabled: None,
                    cross_meeting_linking_auto: None,
                    cross_meeting_linking_threshold: None,
                    max_speakers: None,
                    video_ocr_enabled: None,
                })
                .await
                .unwrap();
            client.capture_stop().await.unwrap();
            server.await.unwrap()
        });
        std::fs::remove_file(&token_file).unwrap();
        assert!(heads[0].starts_with("post /capture/start "), "{}", heads[0]);
        assert!(heads[1].starts_with("post /capture/stop "), "{}", heads[1]);
        for head in heads {
            assert!(
                head.contains(&format!("authorization: bearer {token}")),
                "{head}"
            );
        }
    }

    #[test]
    fn local_focus_never_uses_remote_dns_credentials_or_base_paths() {
        for base in [
            "https://remote.test",
            "http://localhost:3001",
            "http://127.0.0.1:3001/private",
            "http://user@127.0.0.1:3001",
            "http://127.0.0.1:3001?token=x",
        ] {
            let client = super::ServerClient::new(base, "http://127.0.0.1:9").unwrap();
            assert!(client.local_context_url("meeting").is_err(), "{base}");
        }
        for base in ["http://127.0.0.1:3001", "http://[::1]:3001"] {
            let client = super::ServerClient::new(base, "http://127.0.0.1:9").unwrap();
            assert_eq!(
                client.local_context_url("private/id").unwrap().path(),
                "/api/meetings/private%2Fid/local-focus"
            );
        }
    }
    use super::{normalize_base, ServerClient, ServerError, SseParser, SSE_MAX_EVENT_BYTES};

    /// Serve one canned HTTP reply per connection, `replies` times.
    async fn serve(replies: Vec<(&'static str, &'static str)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            for (status, body) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 16 * 1024];
                let _read = socket.read(&mut buffer).await.unwrap();
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        base
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn a_failure_status_is_typed_and_never_shows_the_body() {
        runtime().block_on(async {
            let base = serve(vec![(
                "500 Internal Server Error",
                "secret stack trace /Users/me/private",
            )])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.fetch_meetings().await.unwrap_err();
            assert!(!error.is_transport());
            assert!(matches!(
                &error,
                ServerError::Status { code, body, .. }
                    if code.as_u16() == 500 && body.contains("stack trace")
            ));
            assert_eq!(error.to_string(), "GET /api/meetings → 500");
        });
    }

    /// `/api/capture/status` is 502 when the daemon refused the server's
    /// token; the hub tells that apart from a stopped daemon.
    #[test]
    fn a_capture_status_bad_gateway_is_recognized() {
        runtime().block_on(async {
            let base = serve(vec![(
                "502 Bad Gateway",
                r#"{"error":"capture daemon rejected the server's token"}"#,
            )])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.fetch_capture_status().await.unwrap_err();
            assert!(error.is_bad_gateway(), "{error:?}");
            assert!(!error.is_transport());
            assert!(!error.is_not_found());
        });
    }

    #[test]
    fn not_found_means_missing_not_failed() {
        runtime().block_on(async {
            let base = serve(vec![("404 Not Found", "{}"), ("404 Not Found", "{}")]).await;
            let client = ServerClient::new(&base, &base).unwrap();
            assert!(client.get_meeting("gone").await.unwrap().is_none());
            assert!(client
                .summary_generation("gone", "request")
                .await
                .unwrap()
                .is_none());
        });
    }

    #[test]
    fn an_unexpected_body_is_a_decode_error_and_no_answer_is_transport() {
        runtime().block_on(async {
            let base = serve(vec![("200 OK", "{\"not\": \"a list\"}")]).await;
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.fetch_meetings().await.unwrap_err();
            assert!(matches!(error, ServerError::Decode { .. }), "{error:?}");

            let closed = {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                format!("http://{}", listener.local_addr().unwrap())
            };
            let client = ServerClient::new(&closed, &closed).unwrap();
            let error = client.fetch_meetings().await.unwrap_err();
            assert!(error.is_transport(), "{error:?}");
        });
    }

    #[test]
    fn an_unavailable_client_fails_every_request_without_sending() {
        let error = ServerError::Client(std::sync::Arc::new(
            runtime().block_on(reqwest::get("not a url")).unwrap_err(),
        ));
        let client = ServerClient::unavailable(&error);
        let result = runtime().block_on(client.fetch_meetings());
        assert!(matches!(result, Err(ServerError::Client(_))));
    }

    #[test]
    fn undecodable_stream_events_are_reported_once_per_connection() {
        use super::SseMessage;
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Event {
            kind: String,
        }
        runtime().block_on(async {
            let base = serve(vec![(
                "200 OK",
                "data: {\"kind\":\"a\"}\n\ndata: not json\n\ndata: [1]\n\ndata: {\"kind\":\"b\"}\n\n",
            )])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let resp = client.http().unwrap().get(&base).send().await.unwrap();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SseMessage<Event>>();
            let mut backoff = 0;
            ServerClient::read_sse(resp, "/stream", &tx, &mut backoff)
                .await
                .unwrap();
            drop(tx);
            let mut seen = Vec::new();
            while let Some(message) = rx.recv().await {
                seen.push(match message {
                    SseMessage::Event(event) => event.kind,
                    SseMessage::Undecodable => "undecodable".into(),
                    SseMessage::Disconnected => "disconnected".into(),
                });
            }
            assert_eq!(seen, ["a", "undecodable", "b"]);
        });
    }

    #[test]
    fn sse_parser_refuses_an_unbounded_line_or_event() {
        let mut parser = SseParser::default();
        let long_line = vec![b'x'; SSE_MAX_EVENT_BYTES + 1];
        assert!(parser.push(&long_line).is_err());

        let mut parser = SseParser::default();
        let chunk = format!("data: {}\n", "y".repeat(SSE_MAX_EVENT_BYTES / 2 + 1));
        assert!(parser.push(chunk.as_bytes()).is_ok());
        assert!(
            parser.push(chunk.as_bytes()).is_err(),
            "one event's data lines add up"
        );
    }

    #[test]
    fn sse_parser_splits_events_and_joins_multiline_data() {
        let mut parser = SseParser::default();
        let first = parser
            .push(
                b": heartbeat\ndata: {\"kind\":\"a\"}\n\nevent: x\ndata: 1\ndata: 2\n\ndata: partial",
            )
            .unwrap();
        assert_eq!(
            first,
            vec!["{\"kind\":\"a\"}".to_string(), "1\n2".to_string()]
        );
        let second = parser.push(b" tail\r\n\r\n").unwrap();
        assert_eq!(second, vec!["partial tail".to_string()]);
        assert!(parser.push(b"\n\n").unwrap().is_empty());
    }

    #[test]
    fn base_urls_drop_trailing_slashes() {
        assert_eq!(normalize_base("http://x:1/ "), "http://x:1");
        assert_eq!(normalize_base("http://x:1"), "http://x:1");
    }

    #[test]
    fn sse_parser_preserves_unicode_at_every_network_split() {
        let frame = "data: {\"text\":\"café 🦀 — 你好\"}\r\n\r\n";
        for split in 0..=frame.len() {
            let mut parser = SseParser::default();
            let mut events = parser.push(&frame.as_bytes()[..split]).unwrap();
            events.extend(parser.push(&frame.as_bytes()[split..]).unwrap());
            assert_eq!(
                events,
                ["{\"text\":\"café 🦀 — 你好\"}"],
                "split at byte {split}"
            );
        }
    }

    #[test]
    fn sse_parser_preserves_multiline_unicode_received_one_byte_at_a_time() {
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        for byte in ": heartbeat\ndata: 日本語\ndata: 👩🏽‍💻\n\n".as_bytes() {
            events.extend(parser.push(std::slice::from_ref(byte)).unwrap());
        }
        assert_eq!(events, ["日本語\n👩🏽‍💻"]);
    }
}
