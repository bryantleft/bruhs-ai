//! Compile-time defaults + runtime env overrides for the Google OAuth
//! client. In production, users set `BRUHS_GOOGLE_CLIENT_ID` and
//! `BRUHS_GOOGLE_CLIENT_SECRET` themselves (they provide their own Google
//! Cloud project, per the README setup steps). In tests, we override the
//! token endpoint to point at a wiremock server.

pub const REDIRECT_URI: &str = "http://127.0.0.1:3001/api/calendar/google/oauth/callback";
pub const AUTH_ENDPOINT_DEFAULT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT_DEFAULT: &str = "https://oauth2.googleapis.com/token";
pub const USERINFO_ENDPOINT_DEFAULT: &str = "https://openidconnect.googleapis.com/v1/userinfo";
pub const REVOKE_ENDPOINT_DEFAULT: &str = "https://oauth2.googleapis.com/revoke";

pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar.events.readonly",
    "https://www.googleapis.com/auth/calendar.readonly",
    "openid",
    "email",
];

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub revoke_endpoint: String,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("BRUHS_GOOGLE_CLIENT_ID env var is not set — Google OAuth not configured")]
    MissingClientId,
    #[error("BRUHS_GOOGLE_CLIENT_SECRET env var is not set — Google OAuth not configured")]
    MissingClientSecret,
}

impl OAuthConfig {
    /// Read client_id + client_secret from env. Endpoints default to the
    /// real Google URLs; tests override via the explicit constructor below.
    pub fn from_env() -> Result<Self, ConfigError> {
        let client_id =
            std::env::var("BRUHS_GOOGLE_CLIENT_ID").map_err(|_| ConfigError::MissingClientId)?;
        let client_secret = std::env::var("BRUHS_GOOGLE_CLIENT_SECRET")
            .map_err(|_| ConfigError::MissingClientSecret)?;
        Ok(Self {
            client_id,
            client_secret,
            redirect_uri: REDIRECT_URI.into(),
            auth_endpoint: AUTH_ENDPOINT_DEFAULT.into(),
            token_endpoint: TOKEN_ENDPOINT_DEFAULT.into(),
            userinfo_endpoint: USERINFO_ENDPOINT_DEFAULT.into(),
            revoke_endpoint: REVOKE_ENDPOINT_DEFAULT.into(),
        })
    }

    /// Test-only constructor that points all endpoints at a wiremock host.
    #[cfg(test)]
    pub fn for_test(base_url: &str) -> Self {
        Self {
            client_id: "test-client-id".into(),
            client_secret: "test-client-secret".into(),
            redirect_uri: REDIRECT_URI.into(),
            auth_endpoint: format!("{base_url}/auth"),
            token_endpoint: format!("{base_url}/token"),
            userinfo_endpoint: format!("{base_url}/userinfo"),
            revoke_endpoint: format!("{base_url}/revoke"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_messages_mention_the_env_var() {
        let s = format!("{}", ConfigError::MissingClientId);
        assert!(s.contains("BRUHS_GOOGLE_CLIENT_ID"));
        let s = format!("{}", ConfigError::MissingClientSecret);
        assert!(s.contains("BRUHS_GOOGLE_CLIENT_SECRET"));
    }

    #[test]
    fn for_test_substitutes_base_url() {
        let cfg = OAuthConfig::for_test("http://127.0.0.1:8765");
        assert_eq!(cfg.token_endpoint, "http://127.0.0.1:8765/token");
        assert_eq!(cfg.userinfo_endpoint, "http://127.0.0.1:8765/userinfo");
    }
}
