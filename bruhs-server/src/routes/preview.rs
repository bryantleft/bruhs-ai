//! GET /api/preview/servers — local TCP listeners the Preview surface can
//! offer as one-click targets. The scan lives in `bruhs_core::preview`.

use std::sync::Arc;

use axum::{extract::State, Json};
use bruhs_core::preview::{scan_local_servers, LocalServer};
use serde::Serialize;

use crate::config::Config;

#[derive(Serialize)]
pub struct LocalServersResponse {
    pub servers: Vec<LocalServer>,
}

/// Every local listener except this server itself.
pub async fn get_local_servers(State(config): State<Arc<Config>>) -> Json<LocalServersResponse> {
    Json(LocalServersResponse {
        servers: scan_local_servers(&[config.port]).await,
    })
}
