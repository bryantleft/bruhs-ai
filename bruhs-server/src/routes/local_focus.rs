//! Native local-context endpoint. Browser requests cannot read this private timeline.
use crate::error::AppError;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
use bruhs_core::AppState;
use bruhs_models::LocalFocusObservation;
use std::sync::Arc;

fn authorize(headers: &HeaderMap) -> Result<(), AppError> {
    let local = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|value| !value.contains('@'))
        .and_then(|s| s.parse::<axum::http::uri::Authority>().ok())
        .is_some_and(|a| matches!(a.host(), "127.0.0.1" | "[::1]") && a.port_u16().is_some());
    if !local
        || headers.get_all(header::HOST).iter().count() != 1
        || headers.contains_key(header::ORIGIN)
        || headers.contains_key("sec-fetch-site")
        || headers
            .get("x-bruhs-local-context")
            .is_none_or(|v| v != "1")
    {
        return Err(AppError::Forbidden(
            "Local context requires the native loopback client.".into(),
        ));
    }
    Ok(())
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<
    (
        [(header::HeaderName, &'static str); 1],
        Json<Vec<LocalFocusObservation>>,
    ),
    AppError,
> {
    authorize(&headers)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(state.meetings().list_local_focus(&id).await?),
    ))
}

pub async fn append(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(sample): Json<LocalFocusObservation>,
) -> Result<StatusCode, AppError> {
    authorize(&headers)?;
    if sample.meeting_id != id {
        return Err(AppError::BadRequest("Local focus meeting mismatch.".into()));
    }
    state.meetings().append_local_focus(sample).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_browser_and_rebinding_requests() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:1234".parse().unwrap());
        headers.insert("x-bruhs-local-context", "1".parse().unwrap());
        assert!(authorize(&headers).is_ok());
        for origin in ["null", "https://evil.test", "http://localhost:1234"] {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
            assert!(authorize(&headers).is_err());
        }
        headers.remove(header::ORIGIN);
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        assert!(authorize(&headers).is_err());
        headers.remove("sec-fetch-site");
        for host in [
            "evil.test:1234",
            "localhost.evil.test:1234",
            "evil@127.0.0.1:1234",
            "127.0.0.1:99999",
        ] {
            headers.insert(header::HOST, host.parse().unwrap());
            assert!(authorize(&headers).is_err(), "{host}");
        }
    }

    #[tokio::test]
    async fn local_focus_http_roundtrip_is_private_and_does_not_start_capture() {
        let state = Arc::new(AppState::new());
        let meeting = bruhs_models::Meeting::new("Synthetic context only", 1.0);
        state
            .meetings()
            .create_meeting(meeting.clone())
            .await
            .unwrap();
        let app = axum::Router::new()
            .route(
                "/api/meetings/{id}/local-focus",
                axum::routing::get(list).post(append),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/meetings/{}/local-focus",
            listener.local_addr().unwrap(),
            meeting.id
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        let sample = LocalFocusObservation {
            id: "synthetic".into(),
            meeting_id: meeting.id.clone(),
            observed_at_ms: 1,
            focus: bruhs_models::LocalFocus::Unavailable {},
        };
        for origin in ["null", "https://evil.test", "http://localhost:8088"] {
            let response = client
                .post(&url)
                .header("x-bruhs-local-context", "1")
                .header("origin", origin)
                .json(&sample)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert!(state
            .meetings()
            .list_local_focus(&meeting.id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            client
                .post(&url)
                .header("x-bruhs-local-context", "1")
                .json(&sample)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
        let response = client
            .get(&url)
            .header("x-bruhs-local-context", "1")
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.json::<Vec<LocalFocusObservation>>().await.unwrap(),
            vec![sample]
        );
        assert_eq!(
            client.get(&url).send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
        let _ = server.await;
    }
}
