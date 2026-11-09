//! The one error type every handler returns. Routes that can fail return
//! `Result<_, AppError>`, so the signature documents both the success type
//! and the failure modes, and every failure reaches the client as
//! `{"error": "<message>"}` with a status that says whose fault it was.
//!
//! Server-side failures (5xx) keep the error that caused them as their
//! [`std::error::Error::source`]; the response logs the whole chain once, so
//! a cause is never discarded on the way to a status code.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

/// The error that caused a server-side failure.
pub type ErrorSource = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 400: the client sent a malformed or incomplete request.
    #[error("{0}")]
    BadRequest(String),
    /// 403: the request resolved to something outside what the server may
    /// touch (a path outside the home directory).
    #[error("{0}")]
    Forbidden(String),
    /// 404: the resource does not exist.
    #[error("{0}")]
    NotFound(String),
    /// 409: the resource's current state conflicts with this request.
    #[error("{0}")]
    Conflict(String),
    /// 500: the server failed. `message` is what the client sees.
    #[error("{message}")]
    Internal {
        message: String,
        #[source]
        source: Option<ErrorSource>,
    },
    /// 502: a service the server depends on (the capture daemon) failed.
    #[error("{message}")]
    BadGateway {
        message: String,
        #[source]
        source: Option<ErrorSource>,
    },
    /// 503: authoritative data or a dependency is unavailable right now.
    #[error("{0}")]
    Unavailable(String),
    /// 504: a dependency (a provider CLI) did not answer in time.
    #[error("{0}")]
    GatewayTimeout(String),
}

impl AppError {
    /// 500 caused by `source`; the client sees the source's message.
    pub fn internal(source: impl Into<ErrorSource>) -> Self {
        let source = source.into();
        Self::Internal {
            message: source.to_string(),
            source: Some(source),
        }
    }

    /// 500 caused by `source`, described as `context: <source>`.
    pub fn internal_in(context: &str, source: impl Into<ErrorSource>) -> Self {
        let source = source.into();
        Self::Internal {
            message: format!("{context}: {source}"),
            source: Some(source),
        }
    }

    /// 500 with no underlying error (an invariant the server broke).
    pub fn internal_msg(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            source: None,
        }
    }

    /// 502 caused by `source`.
    pub fn bad_gateway(message: impl Into<String>, source: impl Into<ErrorSource>) -> Self {
        Self::BadGateway {
            message: message.into(),
            source: Some(source.into()),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadGateway { .. } => StatusCode::BAD_GATEWAY,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::GatewayTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
        }
    }

    /// Log a server-side failure once, with its whole cause chain. 503 is
    /// an expected state (capture not running) that clients poll through,
    /// so it is a warning; client errors are the client's to report.
    fn report(&self) {
        let status = self.status();
        if matches!(self, Self::Unavailable(_)) {
            tracing::warn!(status = status.as_u16(), "request failed: {self}");
        } else if status.is_server_error() {
            tracing::error!(
                status = status.as_u16(),
                cause = %cause_chain(self),
                "request failed: {self}"
            );
        }
    }

    /// The response body, with `extra` fields merged in.
    fn into_response_with(self, extra: Option<(&str, Value)>) -> Response {
        self.report();
        let status = self.status();
        let mut body = json!({ "error": self.to_string() });
        if let Some((key, value)) = extra {
            body[key] = value;
        }
        (status, Json(body)).into_response()
    }
}

/// `source → source → …` of `error`, or `-` when it has none.
fn cause_chain(error: &dyn std::error::Error) -> String {
    let mut causes = Vec::new();
    let mut next = error.source();
    while let Some(cause) = next {
        causes.push(cause.to_string());
        next = cause.source();
    }
    if causes.is_empty() {
        "-".into()
    } else {
        causes.join(" → ")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        self.into_response_with(None)
    }
}

/// An [`AppError`] for the routes whose clients read
/// `{"success": false, "error": …}` (login and branch checkout); the status
/// and logging are the same.
#[derive(Debug)]
pub struct WithSuccessFlag(pub AppError);

impl From<AppError> for WithSuccessFlag {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

impl IntoResponse for WithSuccessFlag {
    fn into_response(self) -> Response {
        self.0
            .into_response_with(Some(("success", Value::Bool(false))))
    }
}

impl From<bruhs_core::SteerError> for AppError {
    fn from(error: bruhs_core::SteerError) -> Self {
        match error {
            bruhs_core::SteerError::NoTurn => AppError::Conflict(error.to_string()),
            _ => AppError::internal(error),
        }
    }
}

impl From<bruhs_core::SendMessageError> for AppError {
    fn from(e: bruhs_core::SendMessageError) -> Self {
        match e {
            bruhs_core::SendMessageError::ThreadBusy(_) => AppError::Conflict(e.to_string()),
            _ => AppError::internal(e),
        }
    }
}

impl From<bruhs_core::MeetingStoreError> for AppError {
    fn from(e: bruhs_core::MeetingStoreError) -> Self {
        use bruhs_core::MeetingStoreError as E;
        match e {
            E::MeetingNotFound { .. } | E::PersonNotFound { .. } | E::SpeakerNotFound { .. } => {
                AppError::NotFound(e.to_string())
            }
            E::InvalidInput(msg) => AppError::BadRequest(msg),
            E::Conflict(msg) => AppError::Conflict(msg),
            _ => AppError::internal(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn errors_keep_the_error_body_shape() {
        let response = AppError::NotFound("meeting m".into()).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body(response).await, json!({ "error": "meeting m" }));

        let flagged = WithSuccessFlag(AppError::BadRequest("Invalid path".into())).into_response();
        assert_eq!(flagged.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body(flagged).await,
            json!({ "success": false, "error": "Invalid path" })
        );
    }

    /// The cause of a 500 is kept for the log instead of being discarded.
    #[test]
    fn internal_errors_keep_their_cause() {
        let error = AppError::internal_in("list meetings", std::io::Error::other("disk gone"));
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.to_string(), "list meetings: disk gone");
        assert_eq!(cause_chain(&error), "disk gone");
        assert_eq!(cause_chain(&AppError::Conflict("busy".into())), "-");
    }

    #[test]
    fn store_errors_map_to_their_status() {
        use bruhs_core::MeetingStoreError as E;
        for (error, status) in [
            (E::MeetingNotFound { id: "m".into() }, StatusCode::NOT_FOUND),
            (E::InvalidInput("bad".into()), StatusCode::BAD_REQUEST),
            (E::Conflict("merged".into()), StatusCode::CONFLICT),
            (
                E::PersistenceFailed("io".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            assert_eq!(AppError::from(error).status(), status);
        }
    }
}
