use serde::Deserialize;

use crate::api::ApiError;

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct AuthStatus {
    #[serde(default)]
    pub authenticated: bool,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default, alias = "subscriptionType")]
    pub subscription_type: Option<String>,
    #[serde(default, alias = "authMethod")]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Fetch the auth status for a specific provider (`claude` or `codex`) from
/// the server. An unreachable server or unreadable reply reads as
/// unauthenticated (logged), which is what every caller shows.
pub async fn fetch_auth_status_for(provider: &str) -> AuthStatus {
    let route = format!("/api/agent/auth/{}", crate::api::segment(provider));
    crate::api::get_json(&route).await.unwrap_or_else(|error| {
        log::warn!("fetch_auth_status({provider}): {error}");
        AuthStatus::default()
    })
}

/// Kick off the interactive login flow for a provider on the server
/// (`claude auth login` / `codex login`). The server runs the CLI, which
/// typically prints a device-code URL / opens a browser. `Ok(())` only means
/// the CLI ran — callers poll `fetch_auth_status_for` afterward to observe
/// the result.
pub async fn trigger_login_for(provider: &str) -> Result<(), ApiError> {
    let route = format!("/api/agent/login/{}", crate::api::segment(provider));
    crate::api::send(gloo_net::http::Method::POST, &route, None::<&()>)
        .await
        .map(drop)
}
