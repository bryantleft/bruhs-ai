use super::{ConnectionError as Error, Tokens};
use bruhs_models::connections::ServiceProvider;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone)]
pub(super) struct Config {
    pub provider: ServiceProvider,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth: String,
    pub token: String,
    pub identity: String,
    pub scopes: Vec<String>,
}

impl Config {
    pub fn from_env(provider: ServiceProvider) -> Option<Self> {
        let prefix = format!("BRUHS_CONNECTION_{}", provider.id().to_uppercase());
        let client_id = std::env::var(format!("{prefix}_CLIENT_ID"))
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let client_secret = std::env::var(format!("{prefix}_CLIENT_SECRET"))
            .ok()
            .filter(|s| !s.is_empty());
        let (auth, token, identity, scopes) = match provider {
            ServiceProvider::Google => (
                "https://accounts.google.com/o/oauth2/v2/auth".into(),
                "https://oauth2.googleapis.com/token".into(),
                "https://openidconnect.googleapis.com/v1/userinfo",
                vec![
                    "openid",
                    "email",
                    "https://www.googleapis.com/auth/calendar.events.readonly",
                ],
            ),
            ServiceProvider::Microsoft => {
                let tenant = std::env::var(format!("{prefix}_TENANT"))
                    .unwrap_or_else(|_| "organizations".into());
                if tenant.is_empty()
                    || !tenant
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
                {
                    return None;
                }
                (
                    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
                    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
                    "https://graph.microsoft.com/oidc/userinfo",
                    vec![
                        "openid",
                        "profile",
                        "email",
                        "offline_access",
                        "https://graph.microsoft.com/Calendars.ReadBasic",
                    ],
                )
            }
            ServiceProvider::Github => (
                "https://github.com/login/oauth/authorize".into(),
                "https://github.com/login/oauth/access_token".into(),
                "https://api.github.com/user",
                vec!["read:user", "read:org", "offline_access"],
            ),
        };
        // GitHub requires client authentication even when PKCE is used.
        if provider == ServiceProvider::Github && client_secret.is_none() {
            return None;
        }
        Some(Self {
            provider,
            client_id,
            client_secret,
            auth,
            token,
            identity: identity.into(),
            scopes: scopes.into_iter().map(String::from).collect(),
        })
    }

    pub fn registration(&self) -> String {
        use sha2::{Digest as _, Sha256};
        let mut hash = Sha256::new();
        for item in [&self.client_id, &self.auth, &self.token, &self.identity] {
            hash.update(item);
            hash.update([0]);
        }
        format!("{:x}", hash.finalize())
    }

    pub fn auth_url(&self, redirect: &str, state: &str, verifier: &str) -> Result<String, Error> {
        let mut url = url::Url::parse(&self.auth).map_err(|_| Error::InvalidResponse)?;
        let challenge = crate::google_calendar::pkce::challenge_for_verifier(verifier);
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect),
            ("response_type", "code"),
            ("state", state),
            ("scope", self.scopes.join(" ").as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        if self.provider == ServiceProvider::Google {
            url.query_pairs_mut().append_pair("access_type", "offline");
        }
        // An unset prompt lets the provider reuse the user's browser session.
        Ok(url.into())
    }
}

fn client() -> Result<&'static reqwest::Client, Error> {
    static CLIENT: OnceLock<Result<reqwest::Client, reqwest::Error>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("bruhs-ai")
                .redirect(reqwest::redirect::Policy::none())
                // Same total request policy as the existing Calendar HTTP client.
                .timeout(std::time::Duration::from_secs(30))
                .build()
        })
        .as_ref()
        .map_err(|_| Error::Network)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    token_type: Option<String>,
}

async fn token_request(config: &Config, form: &[(&str, &str)]) -> Result<TokenResponse, Error> {
    let mut form = form.to_vec();
    form.push(("client_id", &config.client_id));
    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret));
    }
    let response = client()?
        .post(&config.token)
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    let status = response.status();
    let body =
        zeroize::Zeroizing::new(response.bytes().await.map_err(|_| Error::Network)?.to_vec());
    // GitHub can return OAuth errors with a successful HTTP status.
    let value: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| Error::InvalidResponse)?;
    if value.get("error").is_some() || !status.is_success() {
        return Err(
            if matches!(
                value["error"].as_str(),
                Some(
                    "invalid_grant"
                        | "bad_verification_code"
                        | "expired_token"
                        | "bad_refresh_token"
                )
            ) {
                Error::SignIn
            } else {
                Error::InvalidResponse
            },
        );
    }
    let parsed: TokenResponse =
        serde_json::from_value(value).map_err(|_| Error::InvalidResponse)?;
    if config.provider != ServiceProvider::Github && parsed.expires_in.is_none() {
        return Err(Error::InvalidResponse);
    }
    if parsed.access_token.is_empty()
        || parsed.access_token.contains('\0')
        || parsed.expires_in.is_some_and(|v| v <= 0)
        || parsed
            .token_type
            .as_deref()
            .is_some_and(|t| !t.eq_ignore_ascii_case("bearer"))
    {
        return Err(Error::InvalidResponse);
    }
    Ok(parsed)
}

fn expiry(seconds: Option<i64>) -> Result<Option<i64>, Error> {
    seconds
        .map(|seconds| {
            seconds
                .checked_mul(1000)
                .and_then(|delta| chrono::Utc::now().timestamp_millis().checked_add(delta))
                .ok_or(Error::InvalidResponse)
        })
        .transpose()
}

pub(super) async fn exchange(
    config: &Config,
    code: &str,
    redirect: &str,
    verifier: &str,
    previous: Option<&Tokens>,
) -> Result<Tokens, Error> {
    let parsed = token_request(
        config,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    let identity = client()?
        .get(&config.identity)
        .bearer_auth(&parsed.access_token)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !identity.status().is_success() {
        return Err(Error::InvalidResponse);
    }
    let value: serde_json::Value = identity.json().await.map_err(|_| Error::InvalidResponse)?;
    let (account_id, account) = match config.provider {
        ServiceProvider::Google | ServiceProvider::Microsoft => (
            value["sub"].as_str().map(String::from),
            value["email"]
                .as_str()
                .or_else(|| value["name"].as_str())
                .map(String::from),
        ),
        ServiceProvider::Github => (
            value["id"].as_u64().map(|id| id.to_string()),
            value["login"].as_str().map(String::from),
        ),
    };
    let account_id = account_id
        .filter(|s| !s.is_empty())
        .ok_or(Error::InvalidResponse)?;
    let account = account
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .unwrap_or_else(|| account_id.clone());
    let registration = config.registration();
    let previous = previous.filter(|p| p.registration == registration);
    if previous.is_some_and(|tokens| tokens.account_id != account_id) {
        return Err(Error::AccountMismatch);
    }
    let refresh_token = parsed
        .refresh_token
        .or_else(|| previous.and_then(|p| p.refresh_token.clone()));
    Ok(Tokens {
        id: uuid::Uuid::new_v4().to_string(),
        access_token: parsed.access_token,
        refresh_token,
        expires_at_ms: expiry(parsed.expires_in)?,
        account_id,
        account,
        registration,
        scopes: parsed.scope.map_or_else(
            || config.scopes.clone(),
            |scope| {
                scope
                    .split([' ', ','])
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            },
        ),
    })
}

pub(super) async fn refresh(config: &Config, previous: &Tokens) -> Result<Tokens, Error> {
    let refresh = previous.refresh_token.as_deref().ok_or(Error::SignIn)?;
    let parsed = token_request(
        config,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
    )
    .await?;
    Ok(Tokens {
        id: previous.id.clone(),
        access_token: parsed.access_token,
        refresh_token: parsed
            .refresh_token
            .or_else(|| previous.refresh_token.clone()),
        expires_at_ms: expiry(parsed.expires_in)?,
        account_id: previous.account_id.clone(),
        account: previous.account.clone(),
        registration: previous.registration.clone(),
        scopes: parsed.scope.map_or_else(
            || previous.scopes.clone(),
            |scope| {
                scope
                    .split([' ', ','])
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            },
        ),
    })
}
