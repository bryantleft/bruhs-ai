use crate::state::CaptureState;
use axum::{extract::State, http::StatusCode, Json};
use bruhs_models::{ActivityMonitorCommand, ActivityMonitorStatus};
use std::sync::Arc;

pub async fn status(State(state): State<Arc<CaptureState>>) -> Json<ActivityMonitorStatus> {
    Json(state.monitor.offer())
}

pub async fn command(
    State(state): State<Arc<CaptureState>>,
    Json(command): Json<ActivityMonitorCommand>,
) -> Result<Json<ActivityMonitorStatus>, (StatusCode, &'static str)> {
    let disabling = matches!(command, ActivityMonitorCommand::Disable { .. });
    let status = state
        .monitor
        .apply(command)
        .map_err(|error| (StatusCode::CONFLICT, error))?;
    if disabling {
        state
            .monitor
            .wait_stopped()
            .await
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))?;
    }
    Ok(Json(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn production_router_requires_consent_and_rejects_foreign_or_stale_commands() {
        // Production router, but deliberately no hardware worker.
        let (_root, state) = crate::state::isolated_state();
        let state = Arc::new(state);
        let (app, authorized) = crate::test_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/activity-monitor", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = authorized;
        let initial: ActivityMonitorStatus =
            serde_json::from_str(&client.get(&url).send().await.unwrap().text().await.unwrap())
                .unwrap();
        assert!(!initial.enabled);
        assert!(state.monitor.acquire().is_none());
        let enable = ActivityMonitorCommand::Enable {
            epoch: initial.epoch.clone(),
            token: uuid::Uuid::new_v4().to_string(),
        };
        let body = serde_json::to_string(&enable).unwrap();
        assert_eq!(
            client
                .post(&url)
                .header("origin", "https://evil.test")
                .header("content-type", "application/json")
                .body(body.clone())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert!(!state.monitor.status().enabled);
        assert_eq!(
            client
                .post(&url)
                .header("content-type", "application/json")
                .body("{\"operation\":\"enable\"}")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            // bruhs-server's proxy is a native client: no Origin header.
            client
                .post(&url)
                .header("content-type", "application/json")
                .body(body.clone())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert!(state.monitor.status().enabled);
        let guard = state.monitor.acquire().unwrap();
        let disable = serde_json::to_string(&ActivityMonitorCommand::Disable {
            epoch: initial.epoch,
        })
        .unwrap();
        let request = client
            .post(&url)
            .header("content-type", "application/json")
            .body(disable);
        let stopping = tokio::spawn(async move { request.send().await.unwrap() });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while state.monitor.status().enabled {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !stopping.is_finished(),
            "disable must await the active synthetic probe"
        );
        assert!(!guard.0.authorized());
        drop(guard);
        assert_eq!(stopping.await.unwrap().status(), StatusCode::OK);
        assert_eq!(
            client
                .post(&url)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!state.monitor.status().enabled);
        server.abort();
        let _ = server.await;
    }
}
