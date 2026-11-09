//! Token storage. Tokens live in the OS credential store via the `keyring`
//! crate: the login keychain on macOS, the Credential Manager on Windows and
//! the Secret Service (gnome-keyring / KWallet) on Linux. The backends are
//! enabled through keyring's cargo features; without them keyring silently
//! uses an in-memory mock and nothing persists.
//!
//! Keychain calls block (and can wait on an OS prompt), so [`TokenStore`]
//! runs every operation on tokio's blocking pool. Tests build the store with
//! [`TokenStore::in_memory`], which never touches the real keychain.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

const SERVICE: &str = "bruhs-ai";
const USERNAME: &str = "google-calendar";

/// Buffer (in ms) before `expires_at_ms` at which we proactively refresh.
const REFRESH_BUFFER_MS: i64 = 60_000;

/// Public, no-secrets-leak-via-Debug token bundle.
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    pub email: Option<String>,
    pub obtained_at_ms: i64,
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("email", &self.email)
            .field("obtained_at_ms", &self.obtained_at_ms)
            .finish()
    }
}

impl TokenSet {
    pub fn is_expiring_soon(&self, now_ms: i64) -> bool {
        (self.expires_at_ms - now_ms) <= REFRESH_BUFFER_MS
    }
}

/// Result type for keychain operations.
pub type TokenResult<T> = Result<T, TokenError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TokenError {
    #[error("no tokens stored")]
    NotFound,
    #[error("keychain error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("token serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("keychain task did not finish: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// Where tokens are kept.
#[derive(Clone)]
enum Backend {
    /// The OS credential store, opened per operation.
    Os,
    /// One shared entry (the in-memory mock in tests).
    Entry(Arc<keyring::Entry>),
}

/// Handle to the stored Google token bundle. Cheap to clone.
#[derive(Clone)]
pub struct TokenStore {
    backend: Backend,
}

impl std::fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match self.backend {
            Backend::Os => "os",
            Backend::Entry(_) => "entry",
        };
        f.debug_struct("TokenStore")
            .field("backend", &backend)
            .finish()
    }
}

impl TokenStore {
    /// The OS credential store (`bruhs-ai` / `google-calendar`).
    pub fn os() -> Self {
        Self {
            backend: Backend::Os,
        }
    }

    /// A private in-memory store backed by keyring's mock credential. Every
    /// clone shares the same entry, so `put` followed by `get` round-trips.
    pub fn in_memory() -> Self {
        let credential = keyring::mock::MockCredential::default();
        Self {
            backend: Backend::Entry(Arc::new(keyring::Entry::new_with_credential(Box::new(
                credential,
            )))),
        }
    }

    /// Run `op` against the entry on the blocking pool.
    async fn with_entry<T, F>(&self, op: F) -> TokenResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&keyring::Entry) -> TokenResult<T> + Send + 'static,
    {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || match backend {
            Backend::Os => op(&keyring::Entry::new(SERVICE, USERNAME)?),
            Backend::Entry(entry) => op(&entry),
        })
        .await?
    }

    /// Persist a `TokenSet`. Overwrites any existing entry.
    pub async fn put(&self, tokens: &TokenSet) -> TokenResult<()> {
        let payload = serde_json::to_string(tokens)?;
        self.with_entry(move |entry| Ok(entry.set_password(&payload)?))
            .await
    }

    /// Load the stored `TokenSet`; `NotFound` before the first connect.
    pub async fn get(&self) -> TokenResult<TokenSet> {
        self.with_entry(|entry| match entry.get_password() {
            Ok(payload) => Ok(serde_json::from_str(&payload)?),
            Err(keyring::Error::NoEntry) => Err(TokenError::NotFound),
            Err(e) => Err(TokenError::Keyring(e)),
        })
        .await
    }

    /// Delete the stored tokens. Ok when there were none.
    pub async fn delete(&self) -> TokenResult<()> {
        self.with_entry(|entry| match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(TokenError::Keyring(e)),
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_expiring_soon_within_buffer() {
        let t = TokenSet {
            access_token: "x".into(),
            refresh_token: "r".into(),
            expires_at_ms: 100_000,
            email: None,
            obtained_at_ms: 0,
        };
        // 100s - 40s = 60s left → at the buffer boundary, should refresh
        assert!(t.is_expiring_soon(40_000));
        // 100s - 1s = 99s left → still fresh
        assert!(!t.is_expiring_soon(1_000));
    }

    fn sample() -> TokenSet {
        TokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms: 1_700_000_000_000,
            email: Some("a@b.c".into()),
            obtained_at_ms: 1_699_999_000_000,
        }
    }

    #[tokio::test]
    async fn in_memory_store_round_trips_across_clones() {
        let store = TokenStore::in_memory();
        assert!(matches!(store.get().await, Err(TokenError::NotFound)));
        store.put(&sample()).await.unwrap();
        let other = store.clone();
        let back = other.get().await.unwrap();
        assert_eq!(back.refresh_token, "rt");
        other.delete().await.unwrap();
        assert!(matches!(store.get().await, Err(TokenError::NotFound)));
        // Deleting again is not an error.
        store.delete().await.unwrap();
    }

    #[tokio::test]
    async fn separate_in_memory_stores_are_isolated() {
        let a = TokenStore::in_memory();
        let b = TokenStore::in_memory();
        a.put(&sample()).await.unwrap();
        assert!(matches!(b.get().await, Err(TokenError::NotFound)));
    }

    #[test]
    fn tokenset_serde_roundtrip() {
        let t = TokenSet {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at_ms: 1_700_000_000_000,
            email: Some("a@b.c".into()),
            obtained_at_ms: 1_699_999_000_000,
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: TokenSet = serde_json::from_str(&json).unwrap();
        assert_eq!(t.access_token, back.access_token);
        assert_eq!(t.refresh_token, back.refresh_token);
        assert_eq!(t.expires_at_ms, back.expires_at_ms);
        assert_eq!(t.email, back.email);
    }
}
