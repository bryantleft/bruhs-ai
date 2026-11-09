//! The one reqwest client every Google request goes through.
//!
//! It has a total timeout and a connect timeout, so a stalled request ends
//! with an error instead of stopping the sync loop forever. Building it can
//! fail (e.g. TLS backend initialisation); that failure is kept and returned
//! from every call rather than panicking.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::oauth::OAuthError;

/// Upper bound for one Google API request, including reading the body.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound for establishing the TCP + TLS connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn client() -> Result<&'static reqwest::Client, OAuthError> {
    static CLIENT: OnceLock<Result<reqwest::Client, Arc<reqwest::Error>>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
                .build()
                .map_err(Arc::new)
        })
        .as_ref()
        .map_err(|e| OAuthError::ClientInit(Arc::clone(e)))
}

#[cfg(test)]
mod tests {
    use super::super::client::CalendarClient;
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A stalled Google request used to hang the sync loop forever. With the
    /// clock paused, tokio jumps straight to whichever timeout fires first.
    #[tokio::test(start_paused = true)]
    async fn stalled_requests_time_out() {
        let server = MockServer::start().await;
        Mock::given(path("/users/me/calendarList"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "items": [] }))
                    .set_delay(REQUEST_TIMEOUT * 10),
            )
            .mount(&server)
            .await;
        let err = CalendarClient::with_base(server.uri())
            .list_calendars("at")
            .await
            .unwrap_err();
        match err {
            OAuthError::Network(e) => assert!(e.is_timeout(), "{e}"),
            other => panic!("expected a timeout, got {other:?}"),
        }
    }
}
