//! Browser authorization and private service credentials, separate from static secrets.
mod login;
mod oauth;
mod storage;

use bruhs_models::connections::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};
use tokio::sync::watch;
use zeroize::{Zeroize, Zeroizing};

pub(crate) const PRIVATE_ENV: [&str; 4] = [
    "BRUHS_CONNECTION_GOOGLE_CLIENT_SECRET",
    "BRUHS_CONNECTION_MICROSOFT_CLIENT_SECRET",
    "BRUHS_CONNECTION_GITHUB_CLIENT_SECRET",
    "BRUHS_GOOGLE_CLIENT_SECRET",
];

pub(crate) fn protect_command(command: &mut tokio::process::Command) {
    for key in PRIVATE_ENV {
        command.env_remove(key);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error(
        "This service is not configured on this device. See the service connection setup guide."
    )]
    NotConfigured,
    #[error("The connection is no longer active. Connect again.")]
    Cancelled,
    #[error("Sign in again to reconnect this account.")]
    SignIn,
    #[error("Could not access the OS keychain or connection catalog. Unlock it and retry.")]
    Storage,
    #[error("The service could not be reached. Check your connection and retry.")]
    Network,
    #[error("The service returned an invalid authorization response. Connect again.")]
    InvalidResponse,
    #[error(
        "This is a different account. Disconnect the existing account before switching accounts."
    )]
    AccountMismatch,
    #[error("The service registration changed. Disconnect the previous account before connecting again.")]
    RegistrationChanged,
    #[error("The saved account identity is unavailable. Disconnect it before connecting again.")]
    IdentityUnavailable,
    #[error("Could not open the local authorization callback. Retry Connect.")]
    Listener,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Tokens {
    id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at_ms: Option<i64>,
    account_id: String,
    account: String,
    registration: String,
    scopes: Vec<String>,
}

impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(refresh) = &mut self.refresh_token {
            refresh.zeroize();
        }
    }
}

#[derive(Clone)]
pub struct Connections(Arc<Inner>);

struct Inner {
    configs: HashMap<ServiceProvider, oauth::Config>,
    store: storage::Store,
    attempts: Mutex<HashMap<String, Attempt>>,
}

pub(crate) struct ConnectionAccess {
    pub value: Zeroizing<String>,
    _lease: storage::Lease,
}

struct Attempt {
    provider: ServiceProvider,
    status: ConnectionAttempt,
    cancel: watch::Sender<bool>,
}

impl Default for Connections {
    fn default() -> Self {
        Self(Arc::new(Inner {
            configs: ServiceProvider::ALL
                .into_iter()
                .filter_map(|provider| oauth::Config::from_env(provider).map(|cfg| (provider, cfg)))
                .collect(),
            store: storage::Store::os(),
            attempts: Mutex::new(HashMap::new()),
        }))
    }
}

impl Connections {
    /// Isolated fixture: no real keychain or environment configuration.
    pub fn in_memory() -> Self {
        Self(Arc::new(Inner {
            configs: HashMap::new(),
            store: storage::Store::memory(),
            attempts: Mutex::new(HashMap::new()),
        }))
    }

    pub async fn list(&self) -> Result<Vec<ServiceConnection>, ConnectionError> {
        let (google, microsoft, github) = tokio::join!(
            self.connection(ServiceProvider::Google),
            self.connection(ServiceProvider::Microsoft),
            self.connection(ServiceProvider::Github)
        );
        Ok(vec![google?, microsoft?, github?])
    }

    pub async fn connection(
        &self,
        provider: ServiceProvider,
    ) -> Result<ServiceConnection, ConnectionError> {
        let config = self.0.configs.get(&provider);
        let saved = self
            .0
            .store
            .metadata(provider)
            .await?
            .filter(|m| m.state != ConnectionState::Disconnected);
        Ok(ServiceConnection {
            id: saved.as_ref().map(|m| m.id.clone()),
            provider,
            configured: config.is_some(),
            account: saved.as_ref().map(|m| m.account.clone()),
            scopes: saved.as_ref().map_or_else(
                || config.map_or_else(Vec::new, |c| c.scopes.clone()),
                |m| m.scopes.clone(),
            ),
            state: saved.map_or(ConnectionState::Disconnected, |m| {
                if config.is_some_and(|c| c.registration() != m.registration)
                    || (!m.refresh_available
                        && m.expires_at_ms
                            .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis()))
                {
                    ConnectionState::NeedsSignIn
                } else {
                    m.state
                }
            }),
        })
    }

    pub fn attempt(&self, id: &str) -> Result<ConnectionAttempt, ConnectionError> {
        self.0
            .attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .map(|a| a.status.clone())
            .ok_or(ConnectionError::Cancelled)
    }

    pub fn cancel(&self, id: &str) {
        if let Some(attempt) = self
            .0
            .attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(id)
        {
            if attempt.status.state != AttemptState::Pending {
                return;
            }
            attempt.cancel.send_replace(true);
            attempt.status.state = AttemptState::Cancelled;
            attempt.status.message =
                Some("Sign-in cancelled or expired. Connect again to retry.".into());
        }
    }

    pub async fn disconnect(&self, provider: ServiceProvider) -> Result<(), ConnectionError> {
        let manager = self.clone();
        tokio::spawn(async move { manager.disconnect_inner(provider).await })
            .await
            .map_err(|_| ConnectionError::Cancelled)?
    }

    async fn disconnect_inner(&self, provider: ServiceProvider) -> Result<(), ConnectionError> {
        {
            let mut attempts = self
                .0
                .attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for attempt in attempts.values_mut().filter(|a| a.provider == provider) {
                attempt.cancel.send_replace(true);
                attempt.status.state = AttemptState::Cancelled;
            }
        }
        // Disconnect is local. Provider-wide revocation can disrupt other app grants.
        self.0.store.lease(provider).await?.clear().await
    }

    pub(crate) async fn access(
        &self,
        provider: ServiceProvider,
        id: &str,
    ) -> Result<ConnectionAccess, ConnectionError> {
        let manager = self.clone();
        let id = id.to_owned();
        // A dropped UI/tool request must not interrupt rotating-token persistence.
        tokio::spawn(async move { manager.access_inner(provider, &id).await })
            .await
            .map_err(|_| ConnectionError::Cancelled)?
    }

    async fn access_inner(
        &self,
        provider: ServiceProvider,
        id: &str,
    ) -> Result<ConnectionAccess, ConnectionError> {
        let config = self
            .0
            .configs
            .get(&provider)
            .ok_or(ConnectionError::NotConfigured)?;
        let lease = self.0.store.lease(provider).await?;
        if !lease
            .metadata()
            .await?
            .is_some_and(|m| m.id == id && m.state == ConnectionState::Connected)
        {
            return Err(ConnectionError::SignIn);
        }
        let Some(mut tokens) = lease.read().await? else {
            lease.needs_sign_in().await?;
            return Err(ConnectionError::SignIn);
        };
        if tokens.registration != config.registration() || tokens.id != id {
            return Err(ConnectionError::SignIn);
        }
        if tokens
            .expires_at_ms
            .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis())
        {
            match oauth::refresh(config, &tokens).await {
                Ok(refreshed) => {
                    tokens = refreshed;
                    lease.write(&tokens).await?;
                }
                Err(ConnectionError::SignIn) => {
                    lease.needs_sign_in().await?;
                    return Err(ConnectionError::SignIn);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(ConnectionAccess {
            value: Zeroizing::new(tokens.access_token.clone()),
            _lease: lease,
        })
    }
}

#[cfg(test)]
mod tests;
