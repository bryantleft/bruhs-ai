//! Subscription windows and account models from the OAuth profile used by agy ACP
//! (supporting both OS-keyring sign-ins and file-backed `acp_token.json`).
use super::{grouping::Identity, AccountUsage};
use bruhs_models::{agent::AgentProvider, agent_extensions::QuotaWindow};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

pub(super) mod windows;
const DAILY: &str = "https://daily-cloudcode-pa.googleapis.com";
const PROD: &str = "https://cloudcode-pa.googleapis.com";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const IDENTITY_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const LIMIT: usize = 2 * 1024 * 1024;
const KEYRING_TARGETS: [(&str, &str); 3] = [
    ("gemini", "antigravity-acp"),
    ("antigravity-acp", "acp_token"),
    ("antigravity-acp", "oauth"),
];
fn failed() -> io::Error {
    io::Error::other("Could not read Antigravity subscription limits. Refresh to try again.")
}
pub(super) fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    format!("antigravity/hub/2.24.0 (aidev_client; os_type={os}; arch={arch}; cl=963137146)")
}
pub(super) fn google_account(id: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("google-account\0{id}").as_bytes())
    )
}
#[derive(Deserialize)]
struct Credential {
    #[serde(alias = "clientId")]
    client_id: String,
    #[serde(alias = "clientSecret")]
    client_secret: String,
    #[serde(alias = "refreshToken")]
    refresh_token: String,
    #[serde(default, alias = "projectId")]
    project_id: Option<String>,
}
impl Drop for Credential {
    fn drop(&mut self) {
        self.client_secret.zeroize();
        self.refresh_token.zeroize();
    }
}
struct Profile {
    credential: Credential,
    business: bool,
}
fn file(path: &Path) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failed()),
    };
    let mut data = Zeroizing::new(Vec::new());
    f.take(LIMIT as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| failed())?;
    if data.len() > LIMIT {
        return Err(failed());
    }
    Ok(Some(data))
}
fn parse_credential(bytes: &[u8]) -> io::Result<Credential> {
    let c: Credential = serde_json::from_slice(bytes).map_err(|_| failed())?;
    if [&c.refresh_token, &c.client_id, &c.client_secret]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        return Err(failed());
    }
    Ok(c)
}
fn read_os_keyring(service: &str, account: &str) -> io::Result<Option<Zeroizing<String>>> {
    match keyring::Entry::new(service, account).and_then(|entry| entry.get_password()) {
        Ok(secret) => Ok(Some(Zeroizing::new(secret))),
        Err(keyring::Error::NoEntry) => Ok(None),
        // Locked/unavailable OS keyrings should not prevent file fallback or crash refresh.
        Err(_) => Ok(None),
    }
}
fn open_profile_with_keyring(
    home: &Path,
    force_file_storage: bool,
    read_keyring: impl Fn(&str, &str) -> io::Result<Option<Zeroizing<String>>>,
) -> io::Result<Option<Profile>> {
    let dir = home.join("antigravity-acp");
    let settings: Value = match file(&dir.join("settings.json"))? {
        Some(s) => serde_json::from_slice(&s).map_err(|_| failed())?,
        None => Value::Null,
    };
    let business = match settings.pointer("/auth/type") {
        None => false,
        Some(v) if v == "oauth-personal" => false,
        Some(v) if v == "oauth-business" => true,
        _ => return Ok(None),
    };
    if !force_file_storage {
        for (service, account) in KEYRING_TARGETS {
            if let Some(secret) = read_keyring(service, account)? {
                let credential = parse_credential(secret.as_bytes())?;
                return Ok(Some(Profile {
                    credential,
                    business,
                }));
            }
        }
    }
    let Some(data) = file(&dir.join("acp_token.json"))? else {
        return Ok(None);
    };
    let credential = parse_credential(&data)?;
    Ok(Some(Profile {
        credential,
        business,
    }))
}
#[cfg(test)]
fn open_profile(home: &Path) -> io::Result<Option<Profile>> {
    open_profile_with_keyring(home, true, |_, _| Ok(None))
}
async fn profile() -> io::Result<Option<Profile>> {
    let vars = tokio::task::spawn_blocking(crate::secret_sources::load)
        .await
        .map_err(|_| failed())??;
    let setting = async |name: &str| {
        vars.provider_variable("antigravity", name)
            .await
            .map_err(|_| failed())
    };
    if matches!(
        setting("AGY_ADC_AUTH").await?.as_deref(),
        Some("1" | "true")
    ) {
        return Ok(None);
    }
    let force_file_storage = matches!(
        setting("AGY_ACP_FORCE_FILE_STORAGE").await?.as_deref(),
        Some("1" | "true")
    );
    let home = setting("GEMINI_HOME")
        .await?
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|p| p.join(".gemini")))
        .ok_or_else(failed)?;
    if !home.is_absolute() {
        return Err(failed());
    }
    tokio::task::spawn_blocking(move || {
        open_profile_with_keyring(&home, force_file_storage, read_os_keyring)
    })
    .await
    .map_err(|_| failed())?
}
async fn response(req: RequestBuilder) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut res = req.send().await.map_err(|_| failed())?;
    if !res.status().is_success() {
        return Err(failed());
    }
    let mut data = Zeroizing::new(Vec::new());
    while let Some(chunk) = res.chunk().await.map_err(|_| failed())? {
        if data.len() + chunk.len() > LIMIT {
            return Err(failed());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
}
impl Drop for Token {
    fn drop(&mut self) {
        self.access_token.zeroize();
    }
}
struct Session {
    profile_key: [u8; 32],
    access: Zeroizing<String>,
    account: String,
    expires: Instant,
}
type Cache = tokio::sync::Mutex<Option<Session>>;
static CACHE: Cache = Cache::const_new(None);
struct Endpoints<'a> {
    token: &'a str,
    identity: &'a str,
    daily: &'a str,
    prod: &'a str,
}
const GOOGLE: Endpoints<'static> = Endpoints {
    token: TOKEN_URL,
    identity: IDENTITY_URL,
    daily: DAILY,
    prod: PROD,
};

async fn session(
    client: &Client,
    c: &Credential,
    endpoints: &Endpoints<'_>,
    cache: &Cache,
) -> io::Result<(Zeroizing<String>, String)> {
    let mut hash = Sha256::new();
    hash.update(&c.client_id);
    hash.update([0]);
    hash.update(&c.refresh_token);
    hash.update([0]);
    hash.update(c.project_id.as_deref().unwrap_or_default());
    let profile_key: [u8; 32] = hash.finalize().into();
    let mut slot = cache.lock().await;
    if let Some(s) = slot.as_ref().filter(|s| {
        s.profile_key == profile_key
            && s.expires.saturating_duration_since(Instant::now()) > Duration::from_secs(60)
    }) {
        return Ok((s.access.clone(), s.account.clone()));
    }
    *slot = None;
    let data = response(client.post(endpoints.token).form(&[
        ("grant_type", "refresh_token"),
        ("client_id", &c.client_id),
        ("client_secret", &c.client_secret),
        ("refresh_token", &c.refresh_token),
    ]))
    .await?;
    let mut token: Token = serde_json::from_slice(&data).map_err(|_| failed())?;
    if token.expires_in == 0 || token.access_token.is_empty() {
        return Err(failed());
    }
    let access = Zeroizing::new(std::mem::take(&mut token.access_token));
    #[derive(Deserialize)]
    struct User {
        id: String,
    }
    let account = response(client.get(endpoints.identity).bearer_auth(&*access))
        .await
        .ok()
        .and_then(|v| serde_json::from_slice::<User>(&v).ok())
        .filter(|u| !u.id.is_empty())
        .map(|u| google_account(&u.id))
        .unwrap_or_else(|| profile_key.iter().map(|b| format!("{b:02x}")).collect());
    if let Some(expires) = Instant::now().checked_add(Duration::from_secs(token.expires_in)) {
        *slot = Some(Session {
            profile_key,
            access: access.clone(),
            account: account.clone(),
            expires,
        });
    }
    Ok((access, account))
}
async fn read_profile(
    profile: &Profile,
    endpoints: &Endpoints<'_>,
    cache: &Cache,
) -> io::Result<(String, Vec<QuotaWindow>)> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| failed())?;
    let c = &profile.credential;
    let (access, account) = session(&client, c, endpoints, cache).await?;
    let project = c
        .project_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .map_or_else(|| json!({}), |s| json!({"project":s}));
    let (first, second) = if profile.business {
        (endpoints.prod, endpoints.daily)
    } else {
        (endpoints.daily, endpoints.prod)
    };
    for (operation, parse) in [
        (
            "retrieveUserQuotaSummary",
            windows::summary as fn(&Value, &str, i64) -> Vec<QuotaWindow>,
        ),
        ("fetchAvailableModels", windows::models),
    ] {
        let fetch = async |base: &str| {
            let at = chrono::Utc::now().timestamp_millis();
            let req = client
                .post(format!("{base}/v1internal:{operation}"))
                .header(reqwest::header::USER_AGENT, user_agent())
                .bearer_auth(&*access)
                .json(&project);
            let bytes = response(req).await.ok()?;
            Some(parse(
                &serde_json::from_slice(&bytes).ok()?,
                "Antigravity",
                at,
            ))
        };
        let (a, b) = tokio::join!(fetch(first), fetch(second));
        let mut rows = a.unwrap_or_default();
        for row in b.unwrap_or_default() {
            if !rows.iter().any(|w| w.window_id == row.window_id) {
                rows.push(row);
            }
        }
        if !rows.is_empty() {
            return Ok((account, rows));
        }
    }
    // Refresh should recover if Google invalidated a still-unexpired access token.
    let mut slot = cache.lock().await;
    if slot.as_ref().is_some_and(|s| s.access == access) {
        *slot = None;
    }
    Err(failed())
}
pub(crate) async fn read() -> Option<AccountUsage> {
    if crate::agent_extensions::availability::require_enabled(AgentProvider::Antigravity).is_err() {
        *CACHE.lock().await = None;
        return None;
    }
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let Some(p) = profile().await? else {
            *CACHE.lock().await = None;
            return Ok(None);
        };
        read_profile(&p, &GOOGLE, &CACHE).await.map(Some)
    })
    .await
    .unwrap_or_else(|_| Err(failed()));
    let (fingerprint, windows) = match result {
        Ok(None) => return None,
        Ok(Some((id, rows))) => (Some(id), Ok(rows)),
        Err(_) => (None, Err(failed())),
    };
    Some(AccountUsage {
        id: "antigravity".into(),
        label: "Antigravity".into(),
        provider: "Antigravity",
        identity: Identity {
            fingerprint,
            ..Default::default()
        },
        checked_at: chrono::Utc::now().timestamp_millis(),
        credits: None,
        plan: None,
        external: None,
        zero_spend_cap: false,
        windows,
    })
}

async fn read_models_profile(
    profile: &Profile,
    endpoints: &Endpoints<'_>,
    cache: &Cache,
) -> io::Result<Value> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| failed())?;
    let c = &profile.credential;
    let (access, _) = session(&client, c, endpoints, cache).await?;
    let project = c
        .project_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .map_or_else(|| json!({}), |s| json!({"project":s}));
    let (first, second) = if profile.business {
        (endpoints.prod, endpoints.daily)
    } else {
        (endpoints.daily, endpoints.prod)
    };
    let fetch = async |base: &str| {
        let req = client
            .post(format!("{base}/v1internal:fetchAvailableModels"))
            .header(reqwest::header::USER_AGENT, user_agent())
            .bearer_auth(&*access)
            .json(&project);
        let bytes = response(req).await.ok()?;
        serde_json::from_slice::<Value>(&bytes).ok()
    };
    let (a, b) = tokio::join!(fetch(first), fetch(second));
    let mut merged = serde_json::Map::new();
    for payload in [a, b].into_iter().flatten() {
        if let Some(models) = payload["models"].as_object() {
            for (id, value) in models {
                merged.entry(id.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    if merged.is_empty() {
        return Err(failed());
    }
    Ok(json!({ "models": merged }))
}

/// Fetch available Antigravity models for the local signed-in OAuth account
/// (used as a fallback when `agy acp` does not list models).
pub(crate) async fn fetch_available_models() -> io::Result<Option<Value>> {
    if crate::agent_extensions::availability::require_enabled(AgentProvider::Antigravity).is_err() {
        return Ok(None);
    }
    let Some(p) = profile().await? else {
        return Ok(None);
    };
    read_models_profile(&p, &GOOGLE, &CACHE).await.map(Some)
}

#[cfg(test)]
mod tests;
