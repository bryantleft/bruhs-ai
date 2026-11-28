//! Managed relay configuration and 24-hour offline webhook holding mailbox.
use bruhs_models::environment_parity::{
    HeldWebhookSummary, HoldWebhookRequest, RelayConfig, RelaySnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Maximum held webhooks retained while offline.
pub const MAX_HELD_WEBHOOKS: usize = 256;
/// Maximum payload size per held webhook (matches reactive hook 64 KiB budget).
pub const MAX_HELD_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RelayMailboxError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Refused(String),
    #[error("Relay mailbox storage failed: {0}")]
    Storage(#[from] std::io::Error),
    #[error("Relay mailbox data is corrupt: {0}")]
    Json(#[from] serde_json::Error),
}

pub type RelayMailboxResult<T> = Result<T, RelayMailboxError>;

/// Full persisted held webhook ready to replay when the environment is online.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldWebhookRecord {
    pub id: Uuid,
    pub task_id: String,
    pub delivery_key: String,
    pub received_at_ms: i64,
    pub expires_at_ms: i64,
    pub uri: String,
    pub headers: BTreeMap<String, Vec<String>>,
    pub query: BTreeMap<String, Vec<String>>,
    pub body: String,
}

impl HeldWebhookRecord {
    fn summary(&self) -> HeldWebhookSummary {
        HeldWebhookSummary {
            id: self.id,
            task_id: self.task_id.clone(),
            delivery_key: self.delivery_key.clone(),
            received_at_ms: self.received_at_ms,
            expires_at_ms: self.expires_at_ms,
            uri: self.uri.clone(),
            body_bytes: self.body.len(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    config: RelayConfig,
    held: Vec<HeldWebhookRecord>,
    last_flushed_at_ms: Option<i64>,
}

struct Inner {
    path: PathBuf,
    store: Mutex<Store>,
}

/// Durable relay preferences and offline webhook mailbox.
#[derive(Clone)]
pub struct RelayMailbox(Arc<Inner>);

impl RelayMailbox {
    pub fn default_path() -> std::io::Result<PathBuf> {
        Ok(crate::agent_extensions::directory()?.join("relay-mailbox.json"))
    }

    pub async fn open(path: PathBuf) -> RelayMailboxResult<Self> {
        let store = tokio::task::spawn_blocking({
            let path = path.clone();
            move || load_store(&path)
        })
        .await
        .map_err(|err| std::io::Error::other(err.to_string()))??;
        Ok(Self(Arc::new(Inner {
            path,
            store: Mutex::new(store),
        })))
    }

    pub async fn snapshot(&self, now_ms: i64) -> RelaySnapshot {
        let mut store = self.0.store.lock().await;
        let pruned = prune_expired(&mut store.held, now_ms);
        if pruned > 0 {
            let _ = persist_store(&self.0.path, &store);
        }
        RelaySnapshot {
            config: store.config.clone(),
            held_webhook_count: store.held.len(),
            held_webhooks: store.held.iter().map(HeldWebhookRecord::summary).collect(),
            last_flushed_at_ms: store.last_flushed_at_ms,
        }
    }

    pub async fn update_config(
        &self,
        mut config: RelayConfig,
        now_ms: i64,
    ) -> RelayMailboxResult<RelaySnapshot> {
        config.relay_url = config.relay_url.trim().to_owned();
        config.environment_label = config.environment_label.trim().to_owned();
        config
            .validate()
            .map_err(|msg| RelayMailboxError::Invalid(msg.into()))?;
        let mut store = self.0.store.lock().await;
        prune_expired(&mut store.held, now_ms);
        store.config = config;
        persist_store(&self.0.path, &store)?;
        Ok(RelaySnapshot {
            config: store.config.clone(),
            held_webhook_count: store.held.len(),
            held_webhooks: store.held.iter().map(HeldWebhookRecord::summary).collect(),
            last_flushed_at_ms: store.last_flushed_at_ms,
        })
    }

    /// Hold a webhook request while offline, respecting the 24h cap, body cap, and dedupe key.
    pub async fn hold_webhook(
        &self,
        request: HoldWebhookRequest,
        now_ms: i64,
    ) -> RelayMailboxResult<HeldWebhookSummary> {
        let task_id = request.task_id.trim();
        let delivery_key = request.delivery_key.trim();
        if task_id.is_empty() || task_id.chars().any(char::is_control) {
            return Err(RelayMailboxError::Invalid(
                "Webhook hold request requires a valid task_id.".into(),
            ));
        }
        if delivery_key.is_empty() || delivery_key.chars().any(char::is_control) {
            return Err(RelayMailboxError::Invalid(
                "Webhook hold request requires a valid delivery_key.".into(),
            ));
        }
        if request.body.len() > MAX_HELD_BODY_BYTES {
            return Err(RelayMailboxError::Invalid(
                "Held webhook payload exceeds the 64 KiB budget.".into(),
            ));
        }
        let mut store = self.0.store.lock().await;
        if !store.config.hold_webhooks_while_offline {
            return Err(RelayMailboxError::Refused(
                "Hold webhooks while offline is turned off for this environment.".into(),
            ));
        }
        prune_expired(&mut store.held, now_ms);
        if let Some(existing) = store
            .held
            .iter()
            .find(|row| row.task_id == task_id && row.delivery_key == delivery_key)
        {
            return Ok(existing.summary());
        }
        if store.held.len() >= MAX_HELD_WEBHOOKS {
            store.held.remove(0);
        }
        let ttl_ms = i64::from(store.config.max_hold_hours.clamp(1, 24)) * 3_600_000;
        let record = HeldWebhookRecord {
            id: Uuid::new_v4(),
            task_id: task_id.to_owned(),
            delivery_key: delivery_key.to_owned(),
            received_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
            uri: request.uri,
            headers: request.headers,
            query: request.query,
            body: request.body,
        };
        let summary = record.summary();
        store.held.push(record);
        let res = persist_store(&self.0.path, &store);
        drop(store);
        res?;
        Ok(summary)
    }

    /// Drain all unexpired held webhooks for delivery, returning `(ready, expired_count)`.
    pub async fn drain_ready(
        &self,
        now_ms: i64,
    ) -> RelayMailboxResult<(Vec<HeldWebhookRecord>, usize)> {
        let mut store = self.0.store.lock().await;
        let expired = prune_expired(&mut store.held, now_ms);
        let ready = std::mem::take(&mut store.held);
        store.last_flushed_at_ms = Some(now_ms);
        let res = persist_store(&self.0.path, &store);
        drop(store);
        res?;
        Ok((ready, expired))
    }

    /// Re-queue records whose replay failed so they are not silently dropped.
    pub async fn requeue_failed(&self, records: Vec<HeldWebhookRecord>) -> RelayMailboxResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut store = self.0.store.lock().await;
        for record in records {
            if store.held.len() < MAX_HELD_WEBHOOKS {
                store.held.push(record);
            }
        }
        let res = persist_store(&self.0.path, &store);
        drop(store);
        res
    }
}

fn prune_expired(held: &mut Vec<HeldWebhookRecord>, now_ms: i64) -> usize {
    let before = held.len();
    held.retain(|row| row.expires_at_ms > now_ms);
    before.saturating_sub(held.len())
}

fn load_store(path: &Path) -> RelayMailboxResult<Store> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(err) => Err(RelayMailboxError::Storage(err)),
    }
}

fn persist_store(path: &Path, store: &Store) -> RelayMailboxResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(store)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn holds_dedupes_expires_and_persists_offline_webhooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.json");
        let mailbox = RelayMailbox::open(path.clone()).await.unwrap();

        // Refused while disabled
        let req = HoldWebhookRequest {
            task_id: "task-1".into(),
            delivery_key: "deliv-1".into(),
            uri: "/api/reactive/tasks/task-1/hook".into(),
            headers: BTreeMap::new(),
            query: BTreeMap::new(),
            body: r#"{"action":"opened"}"#.into(),
        };
        assert!(matches!(
            mailbox.hold_webhook(req.clone(), 1_000).await,
            Err(RelayMailboxError::Refused(_))
        ));

        // Enable holding with 2-hour window
        let cfg = RelayConfig {
            enabled: true,
            relay_url: "https://relay.bruhs.example".into(),
            environment_label: "Studio".into(),
            hold_webhooks_while_offline: true,
            max_hold_hours: 2,
            ..Default::default()
        };
        let snap = mailbox.update_config(cfg, 1_000).await.unwrap();
        assert!(snap.config.hold_webhooks_while_offline);

        // Enqueue and dedupe same delivery_key
        let first = mailbox.hold_webhook(req.clone(), 1_000).await.unwrap();
        let dup = mailbox.hold_webhook(req, 2_000).await.unwrap();
        assert_eq!(first.id, dup.id);

        // Enqueue a second webhook 1 hour later
        let req2 = HoldWebhookRequest {
            task_id: "task-1".into(),
            delivery_key: "deliv-2".into(),
            uri: "/api/reactive/tasks/task-1/hook".into(),
            headers: BTreeMap::new(),
            query: BTreeMap::new(),
            body: r#"{"action":"closed"}"#.into(),
        };
        mailbox.hold_webhook(req2, 3_600_000 + 1_000).await.unwrap();

        // Reopen from disk to prove persistence
        let reopened = RelayMailbox::open(path).await.unwrap();
        assert_eq!(
            reopened
                .snapshot(3_600_000 + 2_000)
                .await
                .held_webhook_count,
            2
        );

        // At 2.5 hours, first expired (> 2h) while second is still valid
        let (ready, expired) = reopened.drain_ready(9_000_000).await.unwrap();
        assert_eq!(expired, 1);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].delivery_key, "deliv-2");
        assert_eq!(reopened.snapshot(9_000_000).await.held_webhook_count, 0);
    }
}
