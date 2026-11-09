//! The one way the SPA talks JSON to bruhs-server: [`get_json`],
//! [`send_json`] and [`send`] build the request, check the status and decode
//! the body, and every failure is an [`ApiError`] that says which of those
//! steps went wrong.

use gloo_net::http::{Method, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::BASE_URL;

/// Longest server error message shown to the user; anything longer is
/// treated as an unreadable body rather than a message.
const MAX_DETAIL_LEN: usize = 512;

/// Why a call to bruhs-server failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// No response arrived: the server is down or unreachable, or the
    /// browser blocked the request.
    #[error("Could not reach the server: {0}")]
    Network(String),
    /// The server answered with a non-success status. `detail` is its
    /// human-readable message (the `error` field of bruhs-server's JSON
    /// error body, or a short plain-text body), if it sent one.
    #[error("{}", http_message(*status, detail.as_deref()))]
    Http { status: u16, detail: Option<String> },
    /// The response arrived but its body was not what this client expects.
    /// The reason is for logs only: it can quote the untrusted body.
    #[error("The server sent a reply this app could not read.")]
    Decode(String),
    /// The caller cancelled the request.
    #[error("Stopped")]
    Aborted,
    /// No answer within the call's time limit; the request was cancelled.
    #[error("The server did not answer within {} seconds.", after_ms / 1000)]
    Timeout { after_ms: u32 },
}

fn http_message(status: u16, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => detail.to_owned(),
        None => format!("The server answered HTTP {status}."),
    }
}

impl ApiError {
    /// An HTTP error from a response's status and body text.
    ///
    /// The body is untrusted: only bruhs-server's `{"error": "…"}` message or
    /// a short plain-text body becomes the detail. HTML (a proxy error page),
    /// other JSON, empty and oversized bodies never reach the UI.
    pub fn http(status: u16, body: &str) -> Self {
        let body = body.trim();
        let detail = match serde_json::from_str::<serde_json::Value>(body) {
            Ok(value) => value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .map(str::to_owned),
            Err(_) if body.starts_with('<') => None,
            Err(_) => Some(body.to_owned()),
        }
        .filter(|detail| !detail.is_empty() && detail.len() <= MAX_DETAIL_LEN);
        Self::Http { status, detail }
    }

    /// The HTTP status, when the server answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Whether the server answered with `status`.
    pub fn is_status(&self, status: u16) -> bool {
        self.status() == Some(status)
    }

    /// A failure from `fetch` itself: an abort, or no response at all.
    pub(crate) fn from_transport(name: &str, message: &str) -> Self {
        if name == "AbortError" {
            Self::Aborted
        } else if message.is_empty() {
            Self::Network(name.to_owned())
        } else {
            Self::Network(message.to_owned())
        }
    }
}

impl From<gloo_net::Error> for ApiError {
    fn from(error: gloo_net::Error) -> Self {
        match error {
            gloo_net::Error::JsError(error) => Self::from_transport(&error.name, &error.message),
            gloo_net::Error::SerdeError(error) => Self::Decode(error.to_string()),
            gloo_net::Error::GlooError(message) => Self::Network(message),
        }
    }
}

/// `Ok(None)` for a 404, so callers can tell "not there" from a failure.
pub fn found<T>(result: Result<T, ApiError>) -> Result<Option<T>, ApiError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_status(404) => Ok(None),
        Err(error) => Err(error),
    }
}

/// The absolute URL of a server route (`/api/…`).
pub fn url(route: &str) -> String {
    format!("{BASE_URL}{route}")
}

/// Send a request built for `route` and return the response if its status
/// is a success.
pub async fn send_request(request: gloo_net::http::Request) -> Result<Response, ApiError> {
    let response = request.send().await?;
    if response.ok() {
        return Ok(response);
    }
    let status = response.status();
    // The body only refines the message; the status alone is the error when
    // the body cannot be read.
    let body = response.text().await.unwrap_or_default();
    Err(ApiError::http(status, &body))
}

/// Decode a JSON response body.
pub async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
    let text = response.text().await?;
    serde_json::from_str(&text).map_err(|error| ApiError::Decode(error.to_string()))
}

fn builder(method: Method, route: &str) -> RequestBuilder {
    RequestBuilder::new(&url(route)).method(method)
}

/// `method route` with an optional JSON body; succeeds on any 2xx and
/// ignores the response body.
pub async fn send<B: Serialize + ?Sized>(
    method: Method,
    route: &str,
    body: Option<&B>,
) -> Result<Response, ApiError> {
    let builder = builder(method, route);
    let request = match body {
        Some(body) => builder.json(body)?,
        None => builder.build()?,
    };
    send_request(request).await
}

/// `method route` with an optional JSON body, decoding the JSON reply.
pub async fn send_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    method: Method,
    route: &str,
    body: Option<&B>,
) -> Result<T, ApiError> {
    decode(send(method, route, body).await?).await
}

/// `GET route`, decoding the JSON reply.
pub async fn get_json<T: DeserializeOwned>(route: &str) -> Result<T, ApiError> {
    send_json::<(), T>(Method::GET, route, None).await
}

/// One path segment of a route, percent-encoded.
pub fn segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(error: &ApiError) -> Option<&str> {
        match error {
            ApiError::Http { detail, .. } => detail.as_deref(),
            _ => None,
        }
    }

    #[test]
    fn server_error_messages_become_the_detail() {
        let error = ApiError::http(409, r#"{"error":"Thread is busy"}"#);
        assert_eq!(
            error,
            ApiError::Http {
                status: 409,
                detail: Some("Thread is busy".into())
            }
        );
        assert_eq!(error.to_string(), "Thread is busy");
        assert!(error.is_status(409));
        let error = ApiError::http(400, r#"{"success":false,"error":"bad id"}"#);
        assert_eq!(detail(&error), Some("bad id"));
        // axum's plain-text extractor rejections are short and readable.
        let error = ApiError::http(
            415,
            "Expected request with `Content-Type: application/json`",
        );
        assert_eq!(
            detail(&error),
            Some("Expected request with `Content-Type: application/json`")
        );
    }

    #[test]
    fn untrusted_bodies_never_become_the_detail() {
        for body in [
            "".to_string(),
            "   ".to_string(),
            "<html>Proxy failure</html>".to_string(),
            r#"{"error":7}"#.to_string(),
            r#"{"message":"nope"}"#.to_string(),
            r#"{"error":"  "}"#.to_string(),
            serde_json::json!({ "error": "x".repeat(513) }).to_string(),
            "y".repeat(513),
        ] {
            let error = ApiError::http(503, &body);
            assert_eq!(detail(&error), None, "{body:?}");
            assert_eq!(error.to_string(), "The server answered HTTP 503.");
        }
    }

    #[test]
    fn a_user_abort_is_not_a_network_failure() {
        assert_eq!(
            ApiError::from_transport("AbortError", "The user aborted a request."),
            ApiError::Aborted
        );
        // The message mentioning "abort" is not what decides it.
        assert_eq!(
            ApiError::from_transport("TypeError", "connection aborted"),
            ApiError::Network("connection aborted".into())
        );
        assert_eq!(
            ApiError::from_transport("TypeError", ""),
            ApiError::Network("TypeError".into())
        );
    }

    #[test]
    fn not_found_is_none_and_other_errors_stay_errors() {
        assert_eq!(found(Ok::<_, ApiError>(1)), Ok(Some(1)));
        assert_eq!(found::<i32>(Err(ApiError::http(404, ""))), Ok(None));
        assert_eq!(
            found::<i32>(Err(ApiError::http(500, ""))),
            Err(ApiError::http(500, ""))
        );
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(segment("abc-123_x.y~"), "abc-123_x.y~");
        assert_eq!(segment("a/b?c#d e"), "a%2Fb%3Fc%23d%20e");
        assert_eq!(segment("é"), "%C3%A9");
    }
}
