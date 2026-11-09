//! Public surface for Google Calendar sync.
//!
//! `GoogleCalendarHandle::from_env` builds an instance from env vars. When
//! BRUHS_GOOGLE_CLIENT_ID / BRUHS_GOOGLE_CLIENT_SECRET are unset the handle
//! still loads, but every OAuth action returns a clear "not configured"
//! error and the sync loop never starts.
//!
//! Constructing a handle does no I/O and spawns nothing; call
//! [`GoogleCalendarHandle::start_background`] from inside the runtime that
//! should own the sync loop.
//!
//! Methods (all idempotent enough to be safely retried):
//!   - `start_oauth() -> { auth_url, state }`
//!   - `complete_oauth(code, state) -> Result<(), HandleError>`
//!   - `disconnect() -> Result<(), HandleError>`
//!   - `state() -> GoogleCalendarSyncState`

pub mod client;
pub mod config;
pub mod cursor;
mod http;
pub mod mapping;
pub mod oauth;
pub mod paths;
pub mod pending;
pub mod pkce;
pub mod sync_engine;
pub mod tokens;

use bruhs_models::GoogleCalendarSyncState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use config::{ConfigError, OAuthConfig};
use cursor::{CalendarFiles, CursorError};
use pending::PendingPkceCache;
use tokens::{TokenError, TokenSet, TokenStore};

/// One-line status the UI consumes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StartOAuthResponse {
    pub auth_url: String,
    pub state: String,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HandleError {
    #[error(transparent)]
    NotConfigured(ConfigError),
    #[error(transparent)]
    OAuth(oauth::OAuthError),
    #[error(transparent)]
    Token(TokenError),
    #[error("invalid or expired OAuth state")]
    InvalidState,
    #[error(transparent)]
    Storage(#[from] CursorError),
}

pub struct GoogleCalendarHandle {
    config: Option<OAuthConfig>,
    pending: Arc<PendingPkceCache>,
    engine: sync_engine::SyncEngine,
    started: AtomicBool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl GoogleCalendarHandle {
    /// Build a handle from env using the OS keychain and the per-user state
    /// directory. Missing env vars leave it unconfigured (see module docs).
    pub fn from_env() -> Self {
        Self::new(
            OAuthConfig::from_env().ok(),
            TokenStore::os(),
            CalendarFiles::default_location(),
        )
    }

    /// Build a handle over explicit storage (tests use
    /// [`TokenStore::in_memory`] and a temp [`CalendarFiles`]).
    pub fn new(config: Option<OAuthConfig>, tokens: TokenStore, files: CalendarFiles) -> Self {
        Self {
            config,
            pending: Arc::new(PendingPkceCache::new()),
            engine: sync_engine::SyncEngine::new(tokens, files),
            started: AtomicBool::new(false),
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<bruhs_models::CalendarSyncEvent> {
        self.engine.subscribe()
    }

    /// Run a sync tick now.
    pub fn force_sync(&self) {
        self.engine.force_sync();
    }

    /// Run a sync tick now that re-lists every enabled calendar instead of
    /// continuing from stored sync tokens. Call this when a subscriber
    /// lagged and dropped events; they are replayed as upserts/deletes.
    pub fn force_full_sync(&self) {
        self.engine.force_full_sync();
    }

    /// Whether a full resync was requested and has not completed yet.
    pub fn full_sync_pending(&self) -> bool {
        self.engine.full_sync_pending()
    }

    /// Spawn the background sync loop on `runtime`. A no-op when OAuth is
    /// not configured or the loop is already running.
    pub fn start_background(&self, runtime: &tokio::runtime::Handle) {
        let Some(cfg) = self.config.clone() else {
            return;
        };
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let engine = self.engine.clone();
        runtime.spawn(async move {
            engine.run_forever(cfg, client::CalendarClient::new()).await;
        });
    }

    fn config(&self) -> Result<&OAuthConfig, HandleError> {
        self.config
            .as_ref()
            .ok_or(HandleError::NotConfigured(ConfigError::MissingClientId))
    }

    pub fn start_oauth(&self) -> Result<StartOAuthResponse, HandleError> {
        let cfg = self.config()?;
        let (verifier, _challenge) = pkce::generate_verifier_and_challenge();
        let state = pkce::generate_state();
        self.pending
            .insert(state.clone(), verifier.clone(), now_ms());
        let auth_url = oauth::build_auth_url(cfg, &state, &verifier);
        Ok(StartOAuthResponse { auth_url, state })
    }

    pub async fn complete_oauth(&self, code: &str, state: &str) -> Result<(), HandleError> {
        let cfg = self.config()?;
        let pending = self
            .pending
            .take(state, now_ms())
            .ok_or(HandleError::InvalidState)?;
        let exchanged = oauth::exchange_code_for_tokens(cfg, code, &pending.verifier)
            .await
            .map_err(HandleError::OAuth)?;
        // The email is only a display label; connecting works without it.
        let email = match oauth::fetch_userinfo_email(cfg, &exchanged.access_token).await {
            Ok(email) => email,
            Err(e) => {
                tracing::warn!("google userinfo lookup failed: {e}");
                None
            }
        };
        let now = now_ms();
        let tokens = TokenSet {
            access_token: exchanged.access_token,
            refresh_token: exchanged.refresh_token,
            expires_at_ms: now + exchanged.expires_in_secs * 1000,
            email,
            obtained_at_ms: now,
        };
        self.engine
            .tokens()
            .put(&tokens)
            .await
            .map_err(HandleError::Token)?;
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<(), HandleError> {
        // Best-effort revoke first. Continue even if the revoke fails so
        // local state still clears.
        if let Some(cfg) = self.config.as_ref() {
            match self.engine.tokens().get().await {
                Ok(stored) => oauth::revoke_refresh_token(cfg, &stored.refresh_token).await,
                Err(TokenError::NotFound) => {}
                Err(e) => tracing::warn!("could not read tokens to revoke: {e}"),
            }
        }
        self.engine
            .tokens()
            .delete()
            .await
            .map_err(HandleError::Token)?;
        // Status only: clients also poll `state()`.
        self.engine
            .emit(bruhs_models::CalendarSyncEvent::Disconnected);
        Ok(())
    }

    /// The stored calendar list. An unreadable list is an error, not `[]`.
    pub async fn list_calendars(&self) -> Result<Vec<bruhs_models::Calendar>, HandleError> {
        Ok(self.engine.files().load_calendars().await?)
    }

    /// Toggle one calendar's `sync_enabled` and trigger an immediate
    /// event-sync. Returns the updated calendar or `None` if the id
    /// wasn't found.
    pub async fn patch_calendar(
        &self,
        id: &str,
        sync_enabled: bool,
    ) -> Result<Option<bruhs_models::Calendar>, HandleError> {
        let updated = self
            .engine
            .files()
            .set_sync_enabled(id, sync_enabled)
            .await?;
        let Some(list) = updated else {
            return Ok(None);
        };
        let found = list.iter().find(|c| c.id == id).cloned();
        // Status only: the caller gets the updated calendar in the reply.
        self.engine
            .emit(bruhs_models::CalendarSyncEvent::CalendarsRefreshed { calendars: list });
        self.engine.force_sync();
        Ok(found)
    }

    pub async fn state(&self) -> GoogleCalendarSyncState {
        match self.engine.tokens().get().await {
            Ok(t) => {
                let last_synced_at = match self
                    .engine
                    .files()
                    .load_cursor(sync_engine::PRIMARY_CALENDAR)
                    .await
                {
                    Ok(cursor) => cursor.last_synced_at_ms,
                    Err(e) => {
                        tracing::warn!("could not read the primary sync cursor: {e}");
                        None
                    }
                };
                GoogleCalendarSyncState {
                    last_synced_at,
                    ..GoogleCalendarSyncState::connected(t.email)
                }
            }
            Err(TokenError::NotFound) => GoogleCalendarSyncState::default(),
            Err(e) => {
                tracing::warn!("could not read google tokens: {e}");
                GoogleCalendarSyncState::default()
            }
        }
    }
}

impl Default for GoogleCalendarHandle {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(config: Option<OAuthConfig>, dir: &std::path::Path) -> GoogleCalendarHandle {
        GoogleCalendarHandle::new(config, TokenStore::in_memory(), CalendarFiles::at(dir))
    }

    fn calendar(id: &str) -> bruhs_models::Calendar {
        bruhs_models::Calendar {
            id: id.into(),
            source: bruhs_models::EventSource::Google,
            provider_id: Some(id.into()),
            name: id.into(),
            color: None,
            sync_enabled: false,
            primary: false,
            last_synced_at: None,
        }
    }

    #[test]
    fn start_oauth_returns_url_and_state_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = handle(Some(OAuthConfig::for_test("http://example")), tmp.path());
        let resp = handle.start_oauth().unwrap();
        assert!(resp.auth_url.contains("client_id=test-client-id"));
        assert!(!resp.state.is_empty());
    }

    /// Building a handle (even a configured one) must not need a runtime:
    /// the desktop app constructs it before entering one.
    #[test]
    fn constructing_a_configured_handle_needs_no_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = handle(Some(OAuthConfig::for_test("http://example")), tmp.path());
        assert!(!handle.started.load(Ordering::SeqCst));
    }

    #[test]
    fn start_background_spawns_once_and_only_when_configured() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let unconfigured = handle(None, tmp.path());
        unconfigured.start_background(runtime.handle());
        assert!(!unconfigured.started.load(Ordering::SeqCst));

        let configured = handle(
            Some(OAuthConfig::for_test("http://127.0.0.1:9")),
            tmp.path(),
        );
        configured.start_background(runtime.handle());
        configured.start_background(runtime.handle());
        assert!(configured.started.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn state_reflects_stored_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = handle(None, tmp.path());
        assert!(!handle.state().await.is_connected());
        handle
            .engine
            .tokens()
            .put(&TokenSet {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at_ms: 0,
                email: Some("me@example.com".into()),
                obtained_at_ms: 0,
            })
            .await
            .unwrap();
        let state = handle.state().await;
        assert!(state.is_connected());
        assert_eq!(state.email(), Some("me@example.com"));

        handle.disconnect().await.unwrap();
        assert!(!handle.state().await.is_connected());
    }

    #[tokio::test]
    async fn patch_calendar_updates_and_reports_unknown_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = handle(None, tmp.path());
        handle
            .engine
            .files()
            .merge_calendars(|_| vec![calendar("one")])
            .await
            .unwrap();
        let mut rx = handle.subscribe();
        let patched = handle.patch_calendar("one", true).await.unwrap().unwrap();
        assert!(patched.sync_enabled);
        assert!(matches!(
            rx.try_recv(),
            Ok(bruhs_models::CalendarSyncEvent::CalendarsRefreshed { .. })
        ));
        assert!(handle
            .patch_calendar("missing", true)
            .await
            .unwrap()
            .is_none());
        assert!(handle.list_calendars().await.unwrap()[0].sync_enabled);
    }

    #[tokio::test]
    async fn corrupt_calendar_list_surfaces_as_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("calendars.json"), b"oops").unwrap();
        let handle = handle(None, tmp.path());
        assert!(matches!(
            handle.list_calendars().await,
            Err(HandleError::Storage(CursorError::Corrupt { .. }))
        ));
        assert!(matches!(
            handle.patch_calendar("one", true).await,
            Err(HandleError::Storage(CursorError::Corrupt { .. }))
        ));
    }

    #[tokio::test]
    async fn complete_oauth_rejects_unknown_state() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = handle(Some(OAuthConfig::for_test("http://example")), tmp.path());
        let err = handle
            .complete_oauth("any-code", "never-issued")
            .await
            .unwrap_err();
        assert!(matches!(err, HandleError::InvalidState));
    }
}
