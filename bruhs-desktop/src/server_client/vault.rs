//! The vault routes (D22) for Settings, Vault: the same calls as the web's
//! `api/vault.rs`. The server owns every value, so the process that saves
//! one is the one that reads it later. Values leave only in the write-only
//! create and update bodies and never come back: every answer is metadata.
//! A native client sends no `Origin`, which the owner-only routes allow.
use super::{ServerClient, ServerError};
use bruhs_models::vault::{VaultCheck, VaultCreate, VaultEntry, VaultListing, VaultUpdate};
use serde::Serialize;
use std::time::Duration;

const VAULT_PATH: &str = "/api/vault";
/// A vault change or check can wait on a keychain or 1Password prompt: the
/// server bounds a read by 30 seconds and `op` by 25, so the page waits
/// longer than both before it gives up.
const VAULT_TIMEOUT: Duration = Duration::from_secs(75);
/// The longest server message shown; longer bodies aren't messages.
const MAX_MESSAGE_LEN: usize = 512;

/// `/api/vault/{id}`, the id percent-encoded as one path segment.
pub(crate) fn vault_entry_path(id: &str) -> String {
    use std::fmt::Write as _;
    let mut path = format!("{VAULT_PATH}/");
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            path.push(char::from(byte));
        } else {
            let _ = write!(path, "%{byte:02X}");
        }
    }
    path
}

/// `/api/vault/{id}/check`.
pub(crate) fn vault_check_path(id: &str) -> String {
    format!("{}/check", vault_entry_path(id))
}

impl ServerError {
    /// What the vault page shows for a failed call: the server's own
    /// sentence (fixed copy that never holds a value), or this error.
    pub(crate) fn vault_message(&self) -> String {
        let detail = match self {
            Self::Status { body, .. } => serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|detail| !detail.is_empty() && detail.len() <= MAX_MESSAGE_LEN)
                        .map(str::to_owned)
                }),
            _ => None,
        };
        detail.unwrap_or_else(|| self.to_string())
    }
}

impl ServerClient {
    /// The reply to one vault call, or why there is none worth reading.
    async fn vault_call<B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<(reqwest::Response, String), ServerError> {
        let label = format!("{method} {path}");
        let mut request = self
            .http()?
            .request(method, self.url(path))
            .timeout(VAULT_TIMEOUT);
        if let Some(body) = body {
            request = request.json(body);
        }
        Ok((Self::check(request.send().await, &label).await?, label))
    }

    async fn vault_json<T: serde::de::DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ServerError> {
        let (response, label) = self.vault_call(method, path, body).await?;
        Self::decode(response, &label).await
    }

    /// Every entry's metadata, the bruhs' names and whether `op` was found.
    /// An older server answers 404.
    pub async fn vault_listing(&self) -> Result<VaultListing, ServerError> {
        self.vault_json::<_, ()>(reqwest::Method::GET, VAULT_PATH, None)
            .await
    }

    pub async fn create_vault_entry(
        &self,
        create: &VaultCreate,
    ) -> Result<VaultEntry, ServerError> {
        self.vault_json(reqwest::Method::POST, VAULT_PATH, Some(create))
            .await
    }

    pub async fn update_vault_entry(
        &self,
        id: &str,
        update: &VaultUpdate,
    ) -> Result<VaultEntry, ServerError> {
        self.vault_json(reqwest::Method::PUT, &vault_entry_path(id), Some(update))
            .await
    }

    pub async fn delete_vault_entry(&self, id: &str) -> Result<(), ServerError> {
        self.vault_call::<()>(reqwest::Method::DELETE, &vault_entry_path(id), None)
            .await
            .map(drop)
    }

    /// Whether one read of the entry works. Never the value.
    pub async fn check_vault_entry(&self, id: &str) -> Result<VaultCheck, ServerError> {
        self.vault_json::<_, ()>(reqwest::Method::POST, &vault_check_path(id), None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{vault_check_path, vault_entry_path, ServerClient};
    use bruhs_models::vault::{CredentialScope, VaultCreate, VaultSecretInput, DUPLICATE_NAME};

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
    fn vault_paths_escape_ids() {
        assert_eq!(vault_entry_path("v1 a"), "/api/vault/v1%20a");
        assert_eq!(vault_entry_path("a/b?c"), "/api/vault/a%2Fb%3Fc");
        assert_eq!(vault_check_path("x"), "/api/vault/x/check");
    }

    #[test]
    fn an_older_server_answers_not_found() {
        runtime().block_on(async {
            let (base, _server) = serve(vec![("404 Not Found", String::new())]).await;
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.vault_listing().await.unwrap_err();
            assert!(error.is_not_found(), "{error:?}");
        });
    }

    #[test]
    fn calls_send_their_routes_and_show_only_the_servers_sentence() {
        let canary = format!("canary-{}", uuid::Uuid::new_v4());
        let refused = serde_json::json!({ "error": DUPLICATE_NAME }).to_string();
        let gone = r#"<html>proxy page</html>"#.to_string();
        let requests = runtime().block_on(async {
            let (base, server) = serve(vec![
                ("400 Bad Request", refused),
                ("204 No Content", String::new()),
                ("500 Internal Server Error", gone),
            ])
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let create = VaultCreate {
                name: "Deploy".into(),
                scope: CredentialScope::Global,
                secret: VaultSecretInput::Value {
                    value: Some(canary.clone()),
                },
            };
            let error = client.create_vault_entry(&create).await.unwrap_err();
            drop(create);
            assert_eq!(error.vault_message(), DUPLICATE_NAME);
            assert!(!format!("{error} {error:?}").contains(&canary));
            client.delete_vault_entry("v1-a b").await.unwrap();
            let error = client.check_vault_entry("v1-a").await.unwrap_err();
            assert_eq!(error.vault_message(), "POST /api/vault/v1-a/check → 500");
            server.await.unwrap()
        });
        assert!(
            requests[0].starts_with("POST /api/vault HTTP/1.1"),
            "{}",
            requests[0]
        );
        assert!(!requests[0].to_lowercase().contains("\r\norigin:"));
        assert!(requests[1].starts_with("DELETE /api/vault/v1-a%20b HTTP/1.1"));
        assert!(requests[2].starts_with("POST /api/vault/v1-a/check HTTP/1.1"));
    }
}
