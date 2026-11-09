use bruhs_models::{ActivityMonitorCommand, ActivityMonitorStatus, ACTIVITY_MONITOR_LEASE_MS};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

struct Lease {
    token: String,
    expires: Instant,
    probe_error: Option<String>,
}

struct Inner {
    epoch: String,
    epoch_expires: Instant,
    lease: Option<Lease>,
}

impl Inner {
    fn revoke(&mut self, now: Instant) {
        self.lease = None;
        self.epoch = uuid::Uuid::new_v4().to_string();
        self.epoch_expires = now + Duration::from_millis(ACTIVITY_MONITOR_LEASE_MS);
    }

    fn expire(&mut self, now: Instant) {
        if self
            .lease
            .as_ref()
            .map_or(now >= self.epoch_expires, |lease| now >= lease.expires)
        {
            self.revoke(now);
        }
    }

    fn status(&self, now: Instant) -> ActivityMonitorStatus {
        ActivityMonitorStatus {
            epoch: self.epoch.clone(),
            enabled: self.lease.is_some(),
            expires_in_ms: self.lease.as_ref().map_or(0, |lease| {
                u64::try_from(lease.expires.saturating_duration_since(now).as_millis())
                    .unwrap_or(u64::MAX)
            }),
            probe_error: self
                .lease
                .as_ref()
                .and_then(|lease| lease.probe_error.clone()),
        }
    }
}

/// One probe at a time, and no grant survives restart, revocation, or expiry.
pub struct MonitorConsent {
    inner: Mutex<Inner>,
    probing: AtomicBool,
    pub changed: Notify,
    drained: Notify,
}

impl MonitorConsent {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                epoch: uuid::Uuid::new_v4().to_string(),
                epoch_expires: Instant::now() + Duration::from_millis(ACTIVITY_MONITOR_LEASE_MS),
                lease: None,
            }),
            probing: AtomicBool::new(false),
            changed: Notify::new(),
            drained: Notify::new(),
        })
    }

    #[cfg(test)]
    pub fn status(&self) -> ActivityMonitorStatus {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.expire(now);
        inner.status(now)
    }

    /// Offer a short-lived challenge, not microphone permission. An idle GET
    /// must not create a token that could enable hours after its client closed.
    pub fn offer(&self) -> ActivityMonitorStatus {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.expire(now);
        if inner.lease.is_none() {
            inner.revoke(now);
        }
        inner.status(now)
    }

    pub fn apply(
        &self,
        command: ActivityMonitorCommand,
    ) -> Result<ActivityMonitorStatus, &'static str> {
        self.apply_at(command, Instant::now())
    }

    fn apply_at(
        &self,
        command: ActivityMonitorCommand,
        now: Instant,
    ) -> Result<ActivityMonitorStatus, &'static str> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.expire(now);
        let epoch = match &command {
            ActivityMonitorCommand::Enable { epoch, .. }
            | ActivityMonitorCommand::Renew { epoch, .. }
            | ActivityMonitorCommand::Disable { epoch } => epoch,
        };
        if epoch != &inner.epoch {
            return Err("Activity monitoring permission expired or was revoked. Turn it on again to grant a new session.");
        }
        let notify = !matches!(command, ActivityMonitorCommand::Renew { .. });
        match command {
            ActivityMonitorCommand::Enable { token, .. } => {
                if uuid::Uuid::parse_str(&token).is_err() {
                    return Err("Invalid activity monitoring session.");
                }
                if inner.lease.is_some() || self.probing.load(Ordering::Acquire) {
                    return Err(
                        "Activity monitoring is already active or stopping in another app session.",
                    );
                }
                let expires = inner.epoch_expires;
                inner.lease = Some(Lease {
                    token,
                    expires,
                    probe_error: None,
                });
            }
            ActivityMonitorCommand::Renew { token, .. } => {
                let lease = inner
                    .lease
                    .as_mut()
                    .filter(|lease| lease.token == token)
                    .ok_or("Activity monitoring session is no longer active.")?;
                lease.expires = now + Duration::from_millis(ACTIVITY_MONITOR_LEASE_MS);
            }
            ActivityMonitorCommand::Disable { .. } => inner.revoke(now),
        }
        let status = inner.status(now);
        drop(inner);
        if notify {
            self.changed.notify_one();
        }
        Ok(status)
    }

    pub fn acquire(self: &Arc<Self>) -> Option<ProbeGuard> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.expire(Instant::now());
        let lease = inner.lease.as_ref()?;
        if self.probing.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(ProbeGuard(SamplePermit {
            consent: self.clone(),
            epoch: inner.epoch.clone(),
            token: lease.token.clone(),
        }))
    }

    /// A successful disable response means even a queued blocking probe finished.
    pub async fn wait_stopped(&self) -> Result<(), &'static str> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = self.drained.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if !self.probing.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| "Permission revoked, but the microphone probe has not confirmed shutdown yet.")
    }
}

#[derive(Clone)]
pub struct SamplePermit {
    consent: Arc<MonitorConsent>,
    pub epoch: String,
    token: String,
}

impl SamplePermit {
    pub fn report_probe_error(&self, error: Option<&str>) {
        let mut inner = self
            .consent
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.expire(Instant::now());
        if inner.epoch == self.epoch {
            if let Some(lease) = inner
                .lease
                .as_mut()
                .filter(|lease| lease.token == self.token)
            {
                lease.probe_error = error.map(str::to_owned);
            }
        }
    }

    pub fn while_authorized<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let mut inner = self
            .consent
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.expire(Instant::now());
        if inner.epoch != self.epoch
            || inner
                .lease
                .as_ref()
                .is_none_or(|lease| lease.token != self.token)
        {
            return None;
        }
        // Held while `f` runs: consent cannot be revoked part-way through.
        let out = f();
        drop(inner);
        Some(out)
    }

    pub fn authorized(&self) -> bool {
        self.while_authorized(|| ()).is_some()
    }
}

pub struct ProbeGuard(pub SamplePermit);

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        self.0.consent.probing.store(false, Ordering::Release);
        self.0.consent.drained.notify_waiters();
    }
}

#[cfg(test)]
mod tests;
