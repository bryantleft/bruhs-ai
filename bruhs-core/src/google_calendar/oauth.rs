//! OAuth2 helpers: build the consent URL, exchange a code for tokens,
//! refresh an expired access token, fetch the connected user's email.
//!
//! Errors are bubbled up as a single `OAuthError` to keep the handler
//! surface in `mod.rs` simple. The error preserves enough detail to
//! distinguish "network failed" from "Google said no" but does NOT
//! preserve secrets — tokens never appear in error strings.

use serde::Deserialize;
use std::sync::Arc;

use super::config::{OAuthConfig, SCOPES};
use super::http::client;
use super::pkce::challenge_for_verifier;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OAuthError {
    #[error("http client unavailable: {0}")]
    ClientInit(#[source] Arc<reqwest::Error>),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("google returned {status}: {body}")]
    Status { status: u16, body: String },
    /// A Google reply (`what`: "token", "events.list", …) was not the JSON
    /// expected.
    #[error("invalid response from google: {what} JSON: {source}")]
    InvalidJson {
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
    /// The token exchange succeeded but granted no refresh token.
    #[error("invalid response from google: missing refresh_token")]
    MissingRefreshToken,
}

impl OAuthError {
    /// Maps a JSON decode failure of the `what` reply.
    pub(crate) fn json(what: &'static str) -> impl FnOnce(serde_json::Error) -> Self {
        move |source| Self::InvalidJson { what, source }
    }
}

/// Build the authorization URL the user opens in their browser.
pub fn build_auth_url(cfg: &OAuthConfig, state: &str, verifier: &str) -> String {
    let challenge = challenge_for_verifier(verifier);
    let scope = SCOPES.join(" ");
    let pairs = [
        ("client_id", cfg.client_id.as_str()),
        ("redirect_uri", cfg.redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", scope.as_str()),
        ("state", state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish();
    format!("{}?{}", cfg.auth_endpoint, query)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}

#[derive(Debug)]
pub struct ExchangedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_secs: i64,
}

/// Exchange an authorization `code` for a token bundle. Requires the
/// `verifier` from the matching PKCE pending entry.
pub async fn exchange_code_for_tokens(
    cfg: &OAuthConfig,
    code: &str,
    verifier: &str,
) -> Result<ExchangedTokens, OAuthError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", cfg.redirect_uri.as_str()),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("code_verifier", verifier),
    ];
    let resp = client()?
        .post(&cfg.token_endpoint)
        .form(&form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(OAuthError::Status {
            status: status.as_u16(),
            body,
        });
    }
    let parsed: TokenResponse = serde_json::from_str(&body).map_err(OAuthError::json("token"))?;
    let refresh_token = parsed
        .refresh_token
        .ok_or(OAuthError::MissingRefreshToken)?;
    Ok(ExchangedTokens {
        access_token: parsed.access_token,
        refresh_token,
        expires_in_secs: parsed.expires_in,
    })
}

#[derive(Debug)]
pub struct RefreshedTokens {
    pub access_token: String,
    pub expires_in_secs: i64,
}

/// Use the refresh_token to mint a new access_token. Google may rotate
/// the refresh_token in rare cases; this function does NOT surface that —
/// callers should keep the original refresh_token.
pub async fn refresh_access_token(
    cfg: &OAuthConfig,
    refresh_token: &str,
) -> Result<RefreshedTokens, OAuthError> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
    ];
    let resp = client()?
        .post(&cfg.token_endpoint)
        .form(&form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(OAuthError::Status {
            status: status.as_u16(),
            body,
        });
    }
    let parsed: TokenResponse = serde_json::from_str(&body).map_err(OAuthError::json("refresh"))?;
    Ok(RefreshedTokens {
        access_token: parsed.access_token,
        expires_in_secs: parsed.expires_in,
    })
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    email: Option<String>,
}

pub async fn fetch_userinfo_email(
    cfg: &OAuthConfig,
    access_token: &str,
) -> Result<Option<String>, OAuthError> {
    let resp = client()?
        .get(&cfg.userinfo_endpoint)
        .bearer_auth(access_token)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(OAuthError::Status {
            status: status.as_u16(),
            body,
        });
    }
    let parsed: UserInfo = serde_json::from_str(&body).map_err(OAuthError::json("userinfo"))?;
    Ok(parsed.email)
}

/// Best-effort revoke. We don't bubble errors — callers will clear local
/// state regardless.
pub async fn revoke_refresh_token(cfg: &OAuthConfig, refresh_token: &str) {
    let result = match client() {
        Ok(client) => client
            .post(&cfg.revoke_endpoint)
            .form(&[("token", refresh_token)])
            .send()
            .await
            .map(drop)
            .map_err(OAuthError::from),
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        tracing::warn!("google token revoke failed (continuing): {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn build_auth_url_includes_required_params() {
        let cfg = OAuthConfig::for_test("http://example");
        let url = build_auth_url(&cfg, "state-xyz", "verifier-abc");
        assert!(url.starts_with("http://example/auth?"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("state=state-xyz"));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("scope=https"));
    }

    #[tokio::test]
    async fn exchange_code_for_tokens_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-1",
                "refresh_token": "rt-1",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;

        let cfg = OAuthConfig::for_test(&server.uri());
        let got = exchange_code_for_tokens(&cfg, "the-code", "the-verifier")
            .await
            .unwrap();
        assert_eq!(got.access_token, "at-1");
        assert_eq!(got.refresh_token, "rt-1");
        assert_eq!(got.expires_in_secs, 3600);
    }

    #[tokio::test]
    async fn exchange_returns_status_error_on_bad_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"invalid_grant"}"#),
            )
            .mount(&server)
            .await;

        let cfg = OAuthConfig::for_test(&server.uri());
        let err = exchange_code_for_tokens(&cfg, "x", "y").await.unwrap_err();
        match err {
            OAuthError::Status { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("invalid_grant"));
            }
            _ => panic!("expected Status error, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn exchange_rejects_response_missing_refresh_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-1",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;
        let cfg = OAuthConfig::for_test(&server.uri());
        let err = exchange_code_for_tokens(&cfg, "x", "y").await.unwrap_err();
        assert!(matches!(err, OAuthError::MissingRefreshToken));
    }

    #[tokio::test]
    async fn refresh_access_token_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-refreshed",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;
        let cfg = OAuthConfig::for_test(&server.uri());
        let got = refresh_access_token(&cfg, "rt-1").await.unwrap();
        assert_eq!(got.access_token, "at-refreshed");
        assert_eq!(got.expires_in_secs, 3600);
    }

    #[tokio::test]
    async fn fetch_userinfo_returns_email() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "email": "you@example.com",
                "sub": "abc"
            })))
            .mount(&server)
            .await;
        let cfg = OAuthConfig::for_test(&server.uri());
        let email = fetch_userinfo_email(&cfg, "at").await.unwrap();
        assert_eq!(email, Some("you@example.com".to_string()));
    }
}
