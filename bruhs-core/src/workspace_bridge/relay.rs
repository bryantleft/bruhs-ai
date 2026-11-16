//! The web side of the connection: bruhs-server relays each request to the
//! tab that enabled the session (`bruhs_models::workspace_bridge::relay`).
//! The server writes the session's private connection file; `bruhs-mcp`
//! reads it and calls the server's loopback relay with the token.
use super::{wire, BridgeRequest, BridgeResponse, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};
use bruhs_models::workspace_bridge::relay::{CALL_PATH, REPLY_TIMEOUT_MS, SECRET_HEX_LEN};
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

/// The client waits a little longer than the server waits for the tab, so a
/// slow tab reaches the client as the server's timeout, not its own.
const CLIENT_TIMEOUT: Duration = Duration::from_millis(REPLY_TIMEOUT_MS + 5_000);

/// 32 random bytes from the OS, hex-encoded: session tokens and stream IDs.
pub fn new_secret() -> io::Result<String> {
    let mut random = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut random)
        .map_err(io::Error::other)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Compares two secrets without stopping at the first differing byte.
pub fn same_secret(supplied: &str, expected: &str) -> bool {
    wire::same_token(supplied, expected)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayConnection {
    /// bruhs-server's loopback origin, such as `http://127.0.0.1:3001`.
    url: String,
    token: String,
}

/// Accepts only a plain HTTP origin on a loopback literal with an explicit
/// port, so a tampered file cannot send the token anywhere else.
fn loopback_origin(origin: &str) -> io::Result<()> {
    let url = url::Url::parse(origin).map_err(|_| wire::invalid_connection())?;
    let loopback = matches!(
        url.host(),
        Some(url::Host::Ipv4(address)) if address.is_loopback()
    ) || matches!(url.host(), Some(url::Host::Ipv6(address)) if address.is_loopback())
        || matches!(url.host(), Some(url::Host::Domain("localhost")));
    if url.scheme() != "http"
        || !loopback
        || url.port().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || origin.ends_with('/')
    {
        return Err(wire::invalid_connection());
    }
    Ok(())
}

/// A web session's private connection file, in a fresh 0700 directory and
/// itself 0600. Dropping it removes both; the token is worthless once the
/// session ends anyway.
pub struct RelayConnectionFile {
    directory: PathBuf,
    path: PathBuf,
}

impl RelayConnectionFile {
    pub fn create(server_origin: &str, token: &str) -> io::Result<Self> {
        loopback_origin(server_origin)?;
        let directory = wire::private_directory()?;
        let path = directory.join("connection.json");
        let connection = RelayConnection {
            url: server_origin.into(),
            token: token.into(),
        };
        if let Err(error) = wire::write_connection(&path, &connection) {
            wire::remove_session_files(&directory);
            return Err(error);
        }
        Ok(Self { directory, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RelayConnectionFile {
    fn drop(&mut self) {
        wire::remove_session_files(&self.directory);
    }
}

#[derive(Clone)]
pub(super) struct Client {
    connection: RelayConnection,
    http: reqwest::Client,
}

impl Client {
    pub(super) fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let connection: RelayConnection =
            serde_json::from_slice(bytes).map_err(|_| wire::invalid_connection())?;
        if connection.token.len() != SECRET_HEX_LEN
            || !connection
                .token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(wire::invalid_connection());
        }
        loopback_origin(&connection.url)?;
        // No proxy and no redirects: the token goes to the named loopback
        // server and nowhere else.
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(CLIENT_TIMEOUT)
            .build()
            .map_err(io::Error::other)?;
        Ok(Self { connection, http })
    }

    /// One request. The server's answer is a [`BridgeResponse`] (the app's,
    /// or its own refusal for a revoked token or a busy tab); anything else,
    /// such as a timeout or a tab that went away, is an error and claims
    /// nothing about what the app did.
    pub(super) async fn call(&self, request: BridgeRequest) -> io::Result<BridgeResponse> {
        let body = serde_json::to_vec(&request)?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Workspace frame exceeds its limit.",
            ));
        }
        let mut response = self
            .http
            .post(format!("{}{CALL_PATH}", self.connection.url))
            .bearer_auth(&self.connection.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Workspace response exceeds its limit.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let unexpected = || {
            io::Error::other(format!(
                "The workspace relay answered {status} without a workspace response."
            ))
        };
        let reply: BridgeResponse = serde_json::from_slice(&bytes).map_err(|_| unexpected())?;
        match reply {
            BridgeResponse::Ok(_) if !status.is_success() => Err(unexpected()),
            reply => Ok(reply),
        }
    }
}

fn transport_error(error: reqwest::Error) -> io::Error {
    if error.is_timeout() {
        super::timed_out()
    } else {
        io::Error::other(error)
    }
}

#[cfg(test)]
mod tests {
    use super::super::BridgeClient;
    use super::*;
    use axum::{
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use bruhs_models::AgentDomain;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn connection_files_are_private_and_hold_a_loopback_origin() {
        let token = new_secret().unwrap();
        assert_eq!(token.len(), SECRET_HEX_LEN);
        let file = RelayConnectionFile::create("http://127.0.0.1:3001", &token).unwrap();
        let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let directory = file.path().parent().unwrap().to_owned();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(BridgeClient::from_file(file.path()).is_ok());
        drop(file);
        assert!(!directory.exists(), "dropping the session removes its file");
        for origin in [
            "https://127.0.0.1:3001",
            "http://example.com:3001",
            "http://127.0.0.1",
            "http://127.0.0.1:3001/api",
            "http://user@127.0.0.1:3001",
            "http://127.0.0.1:3001/",
        ] {
            assert!(
                RelayConnectionFile::create(origin, &token).is_err(),
                "{origin}"
            );
        }
        assert!(RelayConnectionFile::create("http://localhost:3001", &token).is_ok());
        assert!(RelayConnectionFile::create("http://[::1]:3001", &token).is_ok());
    }

    #[test]
    fn tampered_connection_files_are_refused() {
        let token = new_secret().unwrap();
        for body in [
            serde_json::json!({"url":"http://10.0.0.1:3001","token":token}),
            serde_json::json!({"url":"http://127.0.0.1:3001","token":"short"}),
            serde_json::json!({"url":"http://127.0.0.1:3001","token":token,"extra":true}),
            serde_json::json!({"url":"http://127.0.0.1:3001","token":"z".repeat(64)}),
        ] {
            assert!(Client::from_bytes(&serde_json::to_vec(&body).unwrap()).is_err());
        }
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        origin
    }

    #[tokio::test]
    async fn requests_carry_the_token_and_relay_answers_and_refusals() {
        let token = new_secret().unwrap();
        let expected = format!("Bearer {token}");
        let app = Router::new().route(
            CALL_PATH,
            post(move |headers: HeaderMap, body: String| {
                let expected = expected.clone();
                async move {
                    let request: BridgeRequest = serde_json::from_str(&body).unwrap();
                    if headers["authorization"] != expected.as_str() {
                        return (
                            StatusCode::UNAUTHORIZED,
                            axum::Json(BridgeResponse::Error(
                                "Workspace connection is not authorized.".into(),
                            )),
                        );
                    }
                    (
                        StatusCode::OK,
                        axum::Json(BridgeResponse::Ok(
                            serde_json::json!({"echo": serde_json::to_value(request).unwrap()}),
                        )),
                    )
                }
            }),
        );
        let origin = serve(app).await;
        let file = RelayConnectionFile::create(&origin, &token).unwrap();
        let client = BridgeClient::from_file(file.path()).unwrap();
        let reply = client
            .call(BridgeRequest::Read {
                domain: AgentDomain::Tasks,
                query: "needle".into(),
                offset: 3,
            })
            .await
            .unwrap();
        let BridgeResponse::Ok(value) = reply else {
            panic!("ok reply")
        };
        assert_eq!(value["echo"]["kind"], "read");
        assert_eq!(value["echo"]["offset"], 3);
        let other = RelayConnectionFile::create(&origin, &new_secret().unwrap()).unwrap();
        let refused = BridgeClient::from_file(other.path())
            .unwrap()
            .call(BridgeRequest::Status {
                workflow_id: "w".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            refused,
            BridgeResponse::Error("Workspace connection is not authorized.".into())
        );
    }

    #[tokio::test]
    async fn timeouts_and_unexpected_answers_claim_nothing() {
        let app = Router::new().route(
            CALL_PATH,
            post(|| async { (StatusCode::GATEWAY_TIMEOUT, "timed out") }),
        );
        let origin = serve(app).await;
        let file = RelayConnectionFile::create(&origin, &new_secret().unwrap()).unwrap();
        let client = BridgeClient::from_file(file.path()).unwrap();
        assert!(client
            .call(BridgeRequest::Status {
                workflow_id: "w".into()
            })
            .await
            .is_err());
        let ok_on_error = Router::new().route(
            CALL_PATH,
            post(|| async {
                (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(BridgeResponse::Ok(serde_json::json!({}))),
                )
            }),
        );
        let origin = serve(ok_on_error).await;
        let file = RelayConnectionFile::create(&origin, &new_secret().unwrap()).unwrap();
        assert!(BridgeClient::from_file(file.path())
            .unwrap()
            .call(BridgeRequest::Status {
                workflow_id: "w".into()
            })
            .await
            .is_err());
        let huge = Router::new().route(
            CALL_PATH,
            post(|| async { "x".repeat(MAX_RESPONSE_BYTES + 1) }),
        );
        let origin = serve(huge).await;
        let file = RelayConnectionFile::create(&origin, &new_secret().unwrap()).unwrap();
        assert!(BridgeClient::from_file(file.path())
            .unwrap()
            .call(BridgeRequest::Status {
                workflow_id: "w".into()
            })
            .await
            .is_err());
    }
}
