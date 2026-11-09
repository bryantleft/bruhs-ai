//! Ephemeral, explicit permission to probe microphone activity (not recording).

use serde::{Deserialize, Serialize};

pub const ACTIVITY_MONITOR_LEASE_MS: u64 = 30_000;
pub const ACTIVITY_MONITOR_RENEW_MS: u64 = 10_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActivityMonitorCommand {
    Enable { epoch: String, token: String },
    Renew { epoch: String, token: String },
    Disable { epoch: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityMonitorStatus {
    /// Rotated on revocation, expiry, and daemon restart. Not a durable grant.
    pub epoch: String,
    pub enabled: bool,
    pub expires_in_ms: u64,
    #[serde(default)]
    pub probe_error: Option<String>,
}

pub const ACTIVITY_MONITOR_HELP: &str = "Periodically checks microphone volume locally to suggest recording. Does not save audio or identify a meeting. Permission lasts only while this app session stays connected; turn it on again after reopening. Requires microphone access.";

/// Why a microphone-activity permission request or status was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivityMonitorError {
    #[error("Microphone activity checks are already enabled in another app session. Turn them off there first.")]
    EnabledElsewhere,
    #[error("Microphone activity permission was already requested.")]
    AlreadyRequested,
    /// The daemon's probe reported this error.
    #[error(
        "The microphone activity probe failed: {0}. Check microphone access and your input device."
    )]
    ProbeFailed(String),
    #[error("Microphone activity permission expired or was revoked. Turn it on again to allow a new session.")]
    Revoked,
}

/// A client-side consent session. All transitions share one lock so cancelling
/// while status/enable is in flight either prevents enable or returns the exact
/// epoch to revoke. A delayed cleanup can never revoke a newer daemon epoch.
#[derive(Debug, Default)]
pub struct ActivityMonitorSession {
    state: std::sync::Mutex<ClientState>,
}

#[derive(Debug, Default)]
struct ClientState {
    cancelled: bool,
    grant: Option<(String, String)>,
}

impl ActivityMonitorSession {
    pub fn prepare(
        &self,
        status: ActivityMonitorStatus,
    ) -> Result<Option<ActivityMonitorCommand>, ActivityMonitorError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.cancelled {
            return Ok(None);
        }
        if status.enabled {
            return Err(ActivityMonitorError::EnabledElsewhere);
        }
        if state.grant.is_some() {
            return Err(ActivityMonitorError::AlreadyRequested);
        }
        let token = uuid::Uuid::new_v4().to_string();
        state.grant = Some((status.epoch.clone(), token.clone()));
        drop(state);
        Ok(Some(ActivityMonitorCommand::Enable {
            epoch: status.epoch,
            token,
        }))
    }

    pub fn renew(&self) -> Option<ActivityMonitorCommand> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.cancelled {
            return None;
        }
        state
            .grant
            .as_ref()
            .map(|(epoch, token)| ActivityMonitorCommand::Renew {
                epoch: epoch.clone(),
                token: token.clone(),
            })
    }

    pub fn cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancelled
    }

    pub fn validate(&self, status: &ActivityMonitorStatus) -> Result<(), ActivityMonitorError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.cancelled {
            return Ok(());
        }
        if let Some(error) = &status.probe_error {
            return Err(ActivityMonitorError::ProbeFailed(error.clone()));
        }
        let granted = state
            .grant
            .as_ref()
            .is_some_and(|(epoch, _)| epoch == &status.epoch);
        drop(state);
        if !status.enabled || status.expires_in_ms == 0 || !granted {
            return Err(ActivityMonitorError::Revoked);
        }
        Ok(())
    }

    pub fn cancel(&self) -> Option<ActivityMonitorCommand> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cancelled = true;
        state
            .grant
            .take()
            .map(|(epoch, _)| ActivityMonitorCommand::Disable { epoch })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status() -> ActivityMonitorStatus {
        ActivityMonitorStatus {
            epoch: "epoch".into(),
            enabled: false,
            expires_in_ms: 0,
            probe_error: None,
        }
    }
    #[test]
    fn cancellation_before_status_prevents_enable() {
        let session = ActivityMonitorSession::default();
        assert!(session.cancel().is_none());
        assert!(session.prepare(status()).unwrap().is_none());
        assert!(session.renew().is_none());
    }
    #[test]
    fn cancellation_during_enable_revokes_exact_epoch_and_prevents_renewal() {
        let session = ActivityMonitorSession::default();
        assert!(matches!(
            session.prepare(status()).unwrap(),
            Some(ActivityMonitorCommand::Enable { .. })
        ));
        assert_eq!(
            session.cancel(),
            Some(ActivityMonitorCommand::Disable {
                epoch: "epoch".into()
            })
        );
        assert!(session.renew().is_none());
        assert!(session.cancel().is_none());
    }
    #[test]
    fn another_session_cannot_be_adopted_or_revoked_by_prepare() {
        let session = ActivityMonitorSession::default();
        assert!(session
            .prepare(ActivityMonitorStatus {
                enabled: true,
                ..status()
            })
            .is_err());
        assert!(session.cancel().is_none());
    }
    #[test]
    fn changed_or_inactive_epoch_is_not_confirmation() {
        let session = ActivityMonitorSession::default();
        session.prepare(status()).unwrap();
        assert!(session.validate(&status()).is_err());
        let active = ActivityMonitorStatus {
            enabled: true,
            expires_in_ms: 100,
            ..status()
        };
        assert!(session.validate(&active).is_ok());
        assert!(session
            .validate(&ActivityMonitorStatus {
                epoch: "new".into(),
                ..active
            })
            .is_err());
    }

    #[test]
    fn device_error_is_not_healthy_monitoring() {
        let session = ActivityMonitorSession::default();
        session.prepare(status()).unwrap();
        let active = ActivityMonitorStatus {
            enabled: true,
            expires_in_ms: 100,
            probe_error: Some("permission denied".into()),
            ..status()
        };
        let error = session.validate(&active).unwrap_err();
        assert_eq!(
            error,
            ActivityMonitorError::ProbeFailed("permission denied".into())
        );
        assert!(error.to_string().contains("Check microphone access"));
    }
}
