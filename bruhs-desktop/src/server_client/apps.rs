//! The Apps routes (D21) for Settings, Apps: the same calls as the web's
//! `api/apps.rs`. A token leaves only in the write-only connect and replace
//! bodies and never comes back: every answer is metadata. Disconnect is
//! the vault's delete (`delete_vault_entry`), which also takes the account
//! out of every bruh's grants. A native client sends no `Origin`, which the
//! owner-only routes allow.
use super::vault::vault_entry_path;
use super::{ServerClient, ServerError};
use bruhs_models::apps::{AppAccount, AppConnect, AppReplace, AppTestResult, AppsListing};
use bruhs_models::crew::AppGrant;
use bruhs_models::reactive::ReactiveCommand;
use bruhs_models::vault::VAULT_CALL_TIMEOUT_MS;
use serde::Serialize;
use std::time::Duration;

const APPS_PATH: &str = "/api/apps";
pub(crate) const ACCOUNTS_PATH: &str = "/api/apps/accounts";
pub(crate) const COMMANDS_PATH: &str = "/api/reactive/commands";
/// The vault's bound, as on the web: a test call or a keychain prompt can
/// hold the server, and the page's buttons must come back.
const APPS_TIMEOUT: Duration = Duration::from_millis(VAULT_CALL_TIMEOUT_MS as u64);

/// `/api/apps/accounts/{id}`, the id percent-encoded as one path segment
/// by the vault's rule.
pub(crate) fn app_account_path(id: &str) -> String {
    let path = vault_entry_path(id);
    let segment = path.strip_prefix("/api/vault/").unwrap_or(&path);
    format!("{ACCOUNTS_PATH}/{segment}")
}

/// `/api/apps/accounts/{id}/test`.
pub(crate) fn app_test_path(id: &str) -> String {
    format!("{}/test", app_account_path(id))
}

impl ServerClient {
    /// The reply to one Apps call, bounded like the vault's, or why there
    /// is none worth reading.
    async fn apps_call<B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<(reqwest::Response, String), ServerError> {
        let label = format!("{method} {path}");
        let mut request = self
            .http()?
            .request(method, self.url(path))
            .timeout(APPS_TIMEOUT);
        if let Some(body) = body {
            request = request.json(body);
        }
        Ok((Self::check(request.send().await, &label).await?, label))
    }

    async fn apps_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ServerError> {
        let (response, label) = self.apps_call(method, path, body).await?;
        Self::decode(response, &label).await
    }

    /// Every app account and mailbox, value free, with each bruh's grants
    /// and the server's protocol. An older server answers 404.
    pub async fn apps_listing(&self) -> Result<AppsListing, ServerError> {
        self.apps_json::<_, ()>(reqwest::Method::GET, APPS_PATH, None)
            .await
    }

    /// Tests the token, then saves the account. A failed test saves
    /// nothing and answers 422 with its copy.
    pub async fn connect_app(&self, connect: &AppConnect) -> Result<AppAccount, ServerError> {
        self.apps_json(reqwest::Method::POST, ACCOUNTS_PATH, Some(connect))
            .await
    }

    /// A new token for the same account, once it passes the test call.
    pub async fn replace_app_token(
        &self,
        id: &str,
        replace: &AppReplace,
    ) -> Result<AppAccount, ServerError> {
        self.apps_json(reqwest::Method::PUT, &app_account_path(id), Some(replace))
            .await
    }

    /// The saved token's test call. A refused token is a 200 with
    /// `ok: false`, so read `ok`, not the status.
    pub async fn test_app_account(&self, id: &str) -> Result<AppTestResult, ServerError> {
        self.apps_json::<_, ()>(reqwest::Method::POST, &app_test_path(id), None)
            .await
    }

    /// Turns one bruh's app or mail grant on or off (`SetAppGrant`, D9).
    /// Only for a service where `apps_ready`. The answer's snapshot is left
    /// unread: the page reloads the listing.
    pub async fn set_app_grant(
        &self,
        agent_id: &str,
        grant: &AppGrant,
        granted: bool,
    ) -> Result<(), ServerError> {
        let command = ReactiveCommand::SetAppGrant {
            agent_id: agent_id.to_owned(),
            grant: grant.clone(),
            granted,
        };
        self.apps_call(reqwest::Method::POST, COMMANDS_PATH, Some(&command))
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::{app_account_path, app_test_path, ServerClient};
    use bruhs_models::apps::AppConnect;
    use bruhs_models::crew::{AppGrant, CREW_PROTOCOL};
    use bruhs_models::vault::{CredentialScope, VAULT_FAILED};

    /// Serve one canned reply per connection and hand back each request.
    async fn serve(
        replies: Vec<(&'static str, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0; 16 * 1024];
                let read = socket.read(&mut buffer).await.unwrap();
                requests.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            requests
        });
        (base, server)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn app_paths_escape_ids() {
        assert_eq!(app_account_path("v1 a"), "/api/apps/accounts/v1%20a");
        assert_eq!(app_account_path("a/b?c"), "/api/apps/accounts/a%2Fb%3Fc");
        assert_eq!(app_test_path("x"), "/api/apps/accounts/x/test");
    }

    #[test]
    fn an_older_server_answers_not_found() {
        runtime().block_on(async {
            let (base, _server) = serve(vec![("404 Not Found", String::new())]).await;
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.apps_listing().await.unwrap_err();
            assert!(error.is_not_found(), "{error:?}");
        });
    }

    /// Each call sends its route; a refused token's test is a 200 with
    /// `ok: false`; the token never appears in an error.
    #[test]
    fn calls_send_their_routes_and_never_echo_the_token() {
        let canary = format!("canary-{}", uuid::Uuid::new_v4());
        let listing = serde_json::json!({ "accounts": [], "protocol": CREW_PROTOCOL }).to_string();
        let refused =
            serde_json::json!({ "ok": false, "message": "GitHub refused the token." }).to_string();
        let requests = runtime().block_on(async {
            let (base, server) = serve(vec![
                ("200 OK", listing),
                ("500 Internal Server Error", format!("echo {canary}")),
                ("200 OK", refused),
                ("200 OK", "{}".into()),
            ])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let listing = client.apps_listing().await.unwrap();
            assert_eq!(listing.protocol, CREW_PROTOCOL);
            let connect = AppConnect {
                app: "github".into(),
                name: None,
                scope: CredentialScope::Global,
                params: std::collections::BTreeMap::new(),
                token: canary.clone(),
                key: None,
            };
            let error = client.connect_app(&connect).await.unwrap_err();
            drop(connect);
            assert_eq!(error.vault_message(), VAULT_FAILED);
            assert!(!error.vault_message().contains(&canary));
            assert!(!error.to_string().contains(&canary));
            let tested = client.test_app_account("v1-a").await.unwrap();
            assert!(!tested.ok);
            let grant = AppGrant::Token {
                entry_id: "v1-a".into(),
            };
            client.set_app_grant("ash", &grant, true).await.unwrap();
            server.await.unwrap()
        });
        assert!(requests[0].starts_with("GET /api/apps HTTP/1.1"));
        assert!(requests[1].starts_with("POST /api/apps/accounts HTTP/1.1"));
        assert!(!requests[1].to_lowercase().contains("\r\norigin:"));
        assert!(requests[2].starts_with("POST /api/apps/accounts/v1-a/test HTTP/1.1"));
        assert!(requests[3].starts_with("POST /api/reactive/commands HTTP/1.1"));
        assert!(
            requests[3].contains(r#""action":"set_app_grant""#),
            "{}",
            requests[3]
        );
    }
}
