//! The vault routes (D22) for Settings, Vault: the same calls as the web's
//! `api/vault.rs`. The server owns every value, so the process that saves
//! one is the one that reads it later. Values leave only in the write-only
//! create and update bodies and never come back: every answer is metadata.
//! A native client sends no `Origin`, which the owner-only routes allow.
use super::{ServerClient, ServerError};
use bruhs_models::vault::{
    vault_failure_message, VaultCheck, VaultCreate, VaultEntry, VaultFailure, VaultListing,
    VaultUpdate, VAULT_CALL_TIMEOUT_MS,
};
use serde::Serialize;
use std::time::Duration;

const VAULT_PATH: &str = "/api/vault";
/// The web's limit too ([`VAULT_CALL_TIMEOUT_MS`]): a keychain or
/// 1Password prompt can hold the server, so the page outwaits its bounds.
const VAULT_TIMEOUT: Duration = Duration::from_millis(VAULT_CALL_TIMEOUT_MS as u64);

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
    /// What the vault page shows for a failed call, by the rule both apps
    /// share (`bruhs_models::vault::vault_failure_message`): a refusal's or
    /// an unavailable store's own sentence from the server's
    /// `{"error": …}`, else fixed copy. No answer in time, a timeout
    /// included, says the service couldn't be reached.
    pub(crate) fn vault_message(&self) -> String {
        self.vault_failure(|failure| vault_failure_message(failure).into())
    }

    /// Whether a failed vault change may still have landed: it got no
    /// answer, a timeout included (rereview M5).
    pub(crate) fn vault_may_have_landed(&self) -> bool {
        self.vault_failure(|failure| failure.may_have_landed())
    }

    /// The failure by the rule both apps share, read by `with`.
    fn vault_failure<T>(&self, with: impl FnOnce(VaultFailure<'_>) -> T) -> T {
        let json = match self {
            Self::Status { body, .. } => serde_json::from_str::<serde_json::Value>(body).ok(),
            _ => None,
        };
        let failure = match self {
            Self::Transport { .. } => VaultFailure::NoAnswer,
            Self::Status { code, .. } => VaultFailure::Status {
                code: code.as_u16(),
                error: json
                    .as_ref()
                    .and_then(|value| value.get("error"))
                    .and_then(serde_json::Value::as_str),
            },
            Self::Decode { .. } | Self::Invalid(_) | Self::Client(_) => VaultFailure::Other,
        };
        with(failure)
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
    use bruhs_models::vault::{
        CredentialScope, VaultCreate, VaultSecretInput, DUPLICATE_NAME, VAULT_FAILED,
        VAULT_UNREACHABLE,
    };

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
            assert_eq!(error.vault_message(), VAULT_FAILED);
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

    #[test]
    fn only_the_vault_routes_fixed_refusals_are_shown() {
        let status = |code: u16, body: &str| super::ServerError::Status {
            request: "GET /api/vault".into(),
            code: reqwest::StatusCode::from_u16(code).unwrap(),
            body: body.into(),
        };
        let sentence = |text: &str| serde_json::json!({ "error": text }).to_string();
        assert_eq!(
            status(409, &sentence(DUPLICATE_NAME)).vault_message(),
            DUPLICATE_NAME
        );
        let keychain = "Could not access the OS credential store. Unlock it and retry.";
        assert_eq!(status(503, &sentence(keychain)).vault_message(), keychain);
        // A 500 can carry any internal text; it never reaches the page.
        assert_eq!(
            status(500, &sentence("snapshot failed: /private/path")).vault_message(),
            VAULT_FAILED
        );
        assert_eq!(
            status(413, "length limit exceeded").vault_message(),
            VAULT_FAILED
        );
        runtime().block_on(async {
            let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
            let error = client.vault_listing().await.unwrap_err();
            assert!(error.is_transport());
            assert_eq!(error.vault_message(), VAULT_UNREACHABLE);
            // Rereview M5: a change with no answer may still have landed.
            assert!(error.vault_may_have_landed());
        });
        // An answered failure changed nothing.
        assert!(!status(409, &sentence(DUPLICATE_NAME)).vault_may_have_landed());
        assert!(!status(503, "").vault_may_have_landed());
    }
}
