//! Server endpoints for managed relay / offline webhook mailbox, Simlock device pools,
//! durable device access policies, and WSL backend management.
use crate::error::AppError;
use axum::{
    extract::{Extension, Path},
    Json,
};
use bruhs_core::{
    relay_mailbox::{RelayMailbox, RelayMailboxError},
    simlock::{SimlockError, SimlockPool},
    wsl::{WslError, WslManager},
};
use bruhs_models::environment_parity::{
    DevicePolicyUpdate, FlushWebhooksResponse, HeldWebhookSummary, HoldWebhookRequest, RelayConfig,
    RelaySnapshot, SimlockHostSpec, SimlockLease, SimlockLeaseRequest, SimlockPoolSnapshot,
    SimlockProbeResult, WslAction, WslBackendConfig, WslBackendStatus,
};
use serde::Deserialize;
use uuid::Uuid;

fn now_ms() -> i64 {
    bruhs_models::EpochMs::now().as_millis()
}

fn relay_err(err: RelayMailboxError) -> AppError {
    match err {
        RelayMailboxError::Invalid(msg) => AppError::BadRequest(msg),
        RelayMailboxError::Refused(msg) => AppError::Forbidden(msg),
        other => AppError::internal(other),
    }
}

fn simlock_err(err: SimlockError) -> AppError {
    match err {
        SimlockError::Invalid(msg) => AppError::BadRequest(msg),
        SimlockError::Refused(msg) => AppError::Forbidden(msg),
        SimlockError::NotFound(msg) => AppError::NotFound(msg),
        SimlockError::Conflict(msg) => AppError::Conflict(msg),
        other => AppError::internal(other),
    }
}

fn wsl_err(err: WslError) -> AppError {
    match err {
        WslError::Invalid(msg) => AppError::BadRequest(msg),
        WslError::Unsupported(msg) => AppError::Unavailable(msg),
        WslError::Command(msg) => AppError::BadGateway {
            message: msg,
            source: None,
        },
        other => AppError::internal(other),
    }
}

pub async fn relay_snapshot(Extension(mailbox): Extension<RelayMailbox>) -> Json<RelaySnapshot> {
    Json(mailbox.snapshot(now_ms()).await)
}

pub async fn relay_update(
    Extension(mailbox): Extension<RelayMailbox>,
    Json(config): Json<RelayConfig>,
) -> Result<Json<RelaySnapshot>, AppError> {
    mailbox
        .update_config(config, now_ms())
        .await
        .map(Json)
        .map_err(relay_err)
}

pub async fn relay_hold_webhook(
    Extension(mailbox): Extension<RelayMailbox>,
    Json(request): Json<HoldWebhookRequest>,
) -> Result<Json<HeldWebhookSummary>, AppError> {
    mailbox
        .hold_webhook(request, now_ms())
        .await
        .map(Json)
        .map_err(relay_err)
}

pub async fn relay_flush_webhooks(
    Extension(mailbox): Extension<RelayMailbox>,
) -> Result<Json<FlushWebhooksResponse>, AppError> {
    let now = now_ms();
    let (ready, expired) = mailbox.drain_ready(now).await.map_err(relay_err)?;
    let flushed = ready.len();
    let remaining = mailbox.snapshot(now).await.held_webhook_count;
    Ok(Json(FlushWebhooksResponse {
        flushed,
        expired,
        failed: 0,
        remaining,
    }))
}

pub async fn simlock_snapshot(
    Extension(pool): Extension<SimlockPool>,
) -> Json<SimlockPoolSnapshot> {
    Json(pool.snapshot(now_ms()).await)
}

pub async fn simlock_set_policy(
    Extension(pool): Extension<SimlockPool>,
    Json(update): Json<DevicePolicyUpdate>,
) -> Result<Json<SimlockPoolSnapshot>, AppError> {
    pool.set_policy(update.policy, now_ms())
        .await
        .map(Json)
        .map_err(simlock_err)
}

pub async fn simlock_upsert_host(
    Extension(pool): Extension<SimlockPool>,
    Json(spec): Json<SimlockHostSpec>,
) -> Result<Json<SimlockPoolSnapshot>, AppError> {
    pool.upsert_host(spec, now_ms())
        .await
        .map(Json)
        .map_err(simlock_err)
}

pub async fn simlock_remove_host(
    Extension(pool): Extension<SimlockPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<SimlockPoolSnapshot>, AppError> {
    pool.remove_host(id, now_ms())
        .await
        .map(Json)
        .map_err(simlock_err)
}

pub async fn simlock_probe_host(
    Extension(pool): Extension<SimlockPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<SimlockProbeResult>, AppError> {
    pool.probe_host(id).await.map(Json).map_err(simlock_err)
}

pub async fn simlock_acquire_lease(
    Extension(pool): Extension<SimlockPool>,
    Json(request): Json<SimlockLeaseRequest>,
) -> Result<Json<SimlockLease>, AppError> {
    pool.acquire_lease(request, now_ms())
        .await
        .map(Json)
        .map_err(simlock_err)
}

pub async fn simlock_release_lease(
    Extension(pool): Extension<SimlockPool>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let released = pool.release_lease(id).await.map_err(simlock_err)?;
    Ok(Json(serde_json::json!({ "released": released })))
}

pub async fn wsl_status(Extension(wsl): Extension<WslManager>) -> Json<WslBackendStatus> {
    Json(wsl.status().await)
}

pub async fn wsl_update_config(
    Extension(wsl): Extension<WslManager>,
    Json(config): Json<WslBackendConfig>,
) -> Result<Json<WslBackendStatus>, AppError> {
    wsl.update_config(config).await.map(Json).map_err(wsl_err)
}

#[derive(Deserialize)]
pub struct WslControlRequest {
    pub action: WslAction,
    #[serde(default)]
    pub distro: Option<String>,
}

pub async fn wsl_control(
    Extension(wsl): Extension<WslManager>,
    Json(request): Json<WslControlRequest>,
) -> Result<Json<WslBackendStatus>, AppError> {
    wsl.control(request.action, request.distro)
        .await
        .map(Json)
        .map_err(wsl_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::StatusCode,
        routing::{delete, get, post, put},
        Router,
    };
    use bruhs_models::environment_parity::DeviceAccessPolicy;
    use bruhs_models::local_tools::DevicePlatform;

    async fn start_server(dir: &std::path::Path) -> (String, tokio::task::JoinHandle<()>) {
        let mailbox = RelayMailbox::open(dir.join("relay.json")).await.unwrap();
        let pool = SimlockPool::open(dir.join("simlock.json")).await.unwrap();
        let wsl = WslManager::open(dir.join("wsl.json")).await.unwrap();
        let app = Router::new()
            .route("/api/relay", get(relay_snapshot).put(relay_update))
            .route("/api/relay/webhooks/hold", post(relay_hold_webhook))
            .route("/api/relay/webhooks/flush", post(relay_flush_webhooks))
            .route("/api/local-tools/devices/simlock", get(simlock_snapshot))
            .route("/api/local-tools/devices/policy", put(simlock_set_policy))
            .route(
                "/api/local-tools/devices/simlock/hosts",
                post(simlock_upsert_host),
            )
            .route(
                "/api/local-tools/devices/simlock/hosts/{id}",
                delete(simlock_remove_host),
            )
            .route(
                "/api/local-tools/devices/simlock/hosts/{id}/probe",
                post(simlock_probe_host),
            )
            .route(
                "/api/local-tools/devices/simlock/leases",
                post(simlock_acquire_lease),
            )
            .route(
                "/api/local-tools/devices/simlock/leases/{id}",
                delete(simlock_release_lease),
            )
            .route(
                "/api/environment/wsl",
                get(wsl_status).put(wsl_update_config),
            )
            .route("/api/environment/wsl/control", post(wsl_control))
            .layer(Extension(mailbox))
            .layer(Extension(pool))
            .layer(Extension(wsl));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn relay_simlock_and_wsl_http_routes_work_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let (base, server) = start_server(dir.path()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        // 1. Relay config + offline webhook hold & flush
        let resp = client
            .put(format!("{base}/api/relay"))
            .json(&serde_json::json!({
                "enabled": true,
                "relay_url": "https://relay.bruhs.example",
                "environment_label": "Studio",
                "publish_agent_activity": true,
                "preview_tunnel": true,
                "require_device_approval": true,
                "hold_webhooks_while_offline": true,
                "max_hold_hours": 12
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = client
            .post(format!("{base}/api/relay/webhooks/hold"))
            .json(&serde_json::json!({
                "task_id": "t-1",
                "delivery_key": "gh-123",
                "uri": "/api/reactive/tasks/t-1/hook",
                "body": "{\"event\":\"push\"}"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let held: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(held["delivery_key"], "gh-123");

        let resp = client
            .post(format!("{base}/api/relay/webhooks/flush"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let flushed: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(flushed["flushed"], 1);
        assert_eq!(flushed["remaining"], 0);

        // 2. Simlock durable policy + host + live lease acquire & release
        let resp = client
            .put(format!("{base}/api/local-tools/devices/policy"))
            .json(&serde_json::json!({ "policy": DeviceAccessPolicy::Persistent }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let host_id = Uuid::new_v4();
        let resp = client
            .post(format!("{base}/api/local-tools/devices/simlock/hosts"))
            .json(&serde_json::json!({
                "id": host_id,
                "label": "Mac Mini Pool",
                "origin": "http://127.0.0.1:4700",
                "max_devices": 2,
                "lease_minutes": 30
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let snap: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(snap["hosts"].as_array().unwrap().len(), 1);

        let resp = client
            .post(format!("{base}/api/local-tools/devices/simlock/leases"))
            .json(&serde_json::json!({
                "host_id": host_id,
                "device_id": "sim-iphone-16",
                "platform": DevicePlatform::Ios,
                "thread_id": "thread-42"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let lease: serde_json::Value = resp.json().await.unwrap();
        let lease_id = lease["lease_id"].as_str().unwrap();

        let resp = client
            .delete(format!(
                "{base}/api/local-tools/devices/simlock/leases/{lease_id}"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let rel: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(rel["released"], true);

        // 3. WSL status & config update
        let resp = client
            .put(format!("{base}/api/environment/wsl"))
            .json(&serde_json::json!({
                "enabled": true,
                "preferred_distro": "Ubuntu-24.04",
                "stop_windows_backend": false
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let wsl: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(wsl["config"]["preferred_distro"], "Ubuntu-24.04");

        server.abort();
    }
}
