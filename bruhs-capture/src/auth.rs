//! Per-launch bearer token. At startup the daemon writes 32 random bytes
//! (hex) to an owner-only file; every route except `/health` requires
//! `Authorization: Bearer <token>`. Native clients (bruhs-server, the desktop
//! app) read the file; browsers never see it and go through bruhs-server.

use crate::paths::DataRoot;
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::RngCore;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Liveness probe, answered without a token (clients poll it before the
/// token file exists).
const PUBLIC_PATH: &str = "/health";

#[derive(Clone)]
pub struct DaemonToken(Arc<str>);

impl std::fmt::Debug for DaemonToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DaemonToken(..)")
    }
}

impl DaemonToken {
    /// 32 bytes from the thread-local CSPRNG, hex encoded.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Self(hex.into())
    }

    #[cfg(test)]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison against a presented credential.
    fn matches(&self, presented: &[u8]) -> bool {
        let expected = self.0.as_bytes();
        if presented.len() != expected.len() {
            return false;
        }
        let difference = expected
            .iter()
            .zip(presented)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        std::hint::black_box(difference) == 0
    }

    /// Write the token for clients: the file is replaced atomically,
    /// owner-only (0600), and its directory prepared by
    /// [`DataRoot::ensure_token_dir`] (0700 when the daemon owns it).
    pub fn write_to(&self, path: &Path, data: &DataRoot) -> std::io::Result<()> {
        data.ensure_token_dir(path)?;
        crate::paths::write_private_atomic(path, self.0.as_bytes())
    }
}

/// Where this daemon writes its token: `BRUHS_CAPTURE_TOKEN_FILE`, else the
/// default under the data root (the same file clients look for).
pub fn token_path(data: &DataRoot) -> PathBuf {
    std::env::var_os(bruhs_models::capture_auth::TOKEN_FILE_ENV)
        .filter(|path| !path.is_empty())
        .map_or_else(|| data.token_path(), PathBuf::from)
}

/// Reject any request without the bearer token (except `/health`).
pub async fn require_token(
    State(token): State<DaemonToken>,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path() == PUBLIC_PATH {
        return next.run(request).await;
    }
    let mut headers = request.headers().get_all(header::AUTHORIZATION).iter();
    let presented = headers
        .next()
        .filter(|_| headers.next().is_none())
        .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "));
    if presented.is_some_and(|credential| token.matches(credential)) {
        return next.run(request).await;
    }
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex_of_the_client_shape() {
        let first = DaemonToken::generate();
        let second = DaemonToken::generate();
        assert_ne!(first.as_str(), second.as_str());
        assert_eq!(
            bruhs_models::capture_auth::parse_token(first.as_str()),
            Some(first.as_str())
        );
        assert!(!format!("{first:?}").contains(first.as_str()));
    }

    #[test]
    fn comparison_requires_the_exact_token() {
        let token = DaemonToken::generate();
        assert!(token.matches(token.as_str().as_bytes()));
        let mut wrong = token.as_str().as_bytes().to_vec();
        wrong[63] ^= 1;
        assert!(!token.matches(&wrong));
        assert!(!token.matches(&wrong[..63]));
        assert!(!token.matches(b""));
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_owner_only_and_replaced_on_relaunch() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = DataRoot::at(root.path());
        let path = data.token_path();
        let first = DaemonToken::generate();
        first.write_to(&path, &data).unwrap();
        let second = DaemonToken::generate();
        second.write_to(&path, &data).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), second.as_str());
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    /// `BRUHS_CAPTURE_TOKEN_FILE=~/token` must not chmod `$HOME`, and
    /// `/tmp/token` must not fail on a directory the user cannot chmod: an
    /// existing directory outside the daemon's own keeps its mode.
    #[cfg(unix)]
    #[test]
    fn token_file_in_an_existing_foreign_directory_leaves_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = DataRoot::at(root.path().join("app"));
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = home.join("token");
        let token = DaemonToken::generate();
        token.write_to(&path, &data).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), token.as_str());
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&home), 0o755, "an existing directory is left alone");

        // Missing directories on the way to the file are created owner-only.
        let nested = home.join("new").join("dir").join("token");
        token.write_to(&nested, &data).unwrap();
        assert_eq!(mode(&home.join("new")), 0o700);
        assert_eq!(mode(&home.join("new").join("dir")), 0o700);
        assert_eq!(mode(&home), 0o755);
    }

    /// The daemon's own capture directory, left world-readable by an earlier
    /// build, is still tightened.
    #[cfg(unix)]
    #[test]
    fn default_token_directory_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = DataRoot::at(root.path());
        let path = data.token_path();
        let dir = path.parent().unwrap();
        std::fs::create_dir(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        DaemonToken::generate().write_to(&path, &data).unwrap();
        let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
