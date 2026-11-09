//! `GET /api/health` — who is on this port. Bruhs.app reads it before
//! adopting a server it did not start: only a `bruhs-server` counts, and one
//! from another build is given a moment to exit first.

use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Health {
    pub ok: bool,
    pub app: &'static str,
    pub version: &'static str,
    /// The commit this server was built from (empty outside a checkout).
    pub build: &'static str,
}

pub async fn get_health() -> Json<Health> {
    let release = env!("BRUHS_RELEASE_VERSION");
    Json(Health {
        ok: true,
        app: "bruhs-server",
        version: if release.is_empty() {
            env!("CARGO_PKG_VERSION")
        } else {
            release
        },
        build: env!("BRUHS_BUILD_COMMIT"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_names_the_server_and_its_build() {
        let Json(health) = get_health().await;
        assert!(health.ok);
        assert_eq!(health.app, "bruhs-server");
        assert_eq!(health.build, env!("BRUHS_BUILD_COMMIT"));
    }
}
