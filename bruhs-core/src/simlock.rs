//! Server-side Simlock device host pool, lease manager, health probe, and durable device policy.
use bruhs_models::environment_parity::{
    DeviceAccessPolicy, SimlockHostSpec, SimlockLease, SimlockLeaseRequest, SimlockPoolSnapshot,
    SimlockProbeResult,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum SimlockError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Simlock pool storage failed: {0}")]
    Storage(#[from] std::io::Error),
    #[error("Simlock pool data is corrupt: {0}")]
    Json(#[from] serde_json::Error),
}

pub type SimlockResult<T> = Result<T, SimlockError>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    policy: DeviceAccessPolicy,
    hosts: Vec<SimlockHostSpec>,
    leases: Vec<SimlockLease>,
}

struct Inner {
    path: PathBuf,
    store: Mutex<Store>,
    client: reqwest::Client,
}

/// Durable Simlock device host pool and lease controller.
#[derive(Clone)]
pub struct SimlockPool(Arc<Inner>);

impl SimlockPool {
    pub fn default_path() -> std::io::Result<PathBuf> {
        Ok(crate::agent_extensions::directory()?.join("simlock-pool.json"))
    }

    pub async fn open(path: PathBuf) -> SimlockResult<Self> {
        let store = tokio::task::spawn_blocking({
            let path = path.clone();
            move || load_store(&path)
        })
        .await
        .map_err(|err| std::io::Error::other(err.to_string()))??;
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        Ok(Self(Arc::new(Inner {
            path,
            store: Mutex::new(store),
            client,
        })))
    }

    pub async fn snapshot(&self, now_ms: i64) -> SimlockPoolSnapshot {
        let mut store = self.0.store.lock().await;
        if prune_expired(&mut store.leases, now_ms) > 0 {
            let _ = persist_store(&self.0.path, &store);
        }
        SimlockPoolSnapshot {
            policy: store.policy,
            hosts: store.hosts.clone(),
            active_leases: store.leases.clone(),
        }
    }

    pub async fn set_policy(
        &self,
        policy: DeviceAccessPolicy,
        now_ms: i64,
    ) -> SimlockResult<SimlockPoolSnapshot> {
        let mut store = self.0.store.lock().await;
        prune_expired(&mut store.leases, now_ms);
        store.policy = policy;
        if policy == DeviceAccessPolicy::Off {
            store.leases.clear();
        }
        persist_store(&self.0.path, &store)?;
        Ok(SimlockPoolSnapshot {
            policy: store.policy,
            hosts: store.hosts.clone(),
            active_leases: store.leases.clone(),
        })
    }

    pub async fn upsert_host(
        &self,
        mut spec: SimlockHostSpec,
        now_ms: i64,
    ) -> SimlockResult<SimlockPoolSnapshot> {
        spec.label = spec.label.trim().to_owned();
        spec.origin = spec.origin.trim().trim_end_matches('/').to_owned();
        spec.validate()
            .map_err(|msg| SimlockError::Invalid(msg.into()))?;
        let mut store = self.0.store.lock().await;
        prune_expired(&mut store.leases, now_ms);
        if let Some(existing) = store.hosts.iter_mut().find(|h| h.id == spec.id) {
            *existing = spec;
        } else {
            store.hosts.push(spec);
        }
        persist_store(&self.0.path, &store)?;
        Ok(SimlockPoolSnapshot {
            policy: store.policy,
            hosts: store.hosts.clone(),
            active_leases: store.leases.clone(),
        })
    }

    pub async fn remove_host(&self, id: Uuid, now_ms: i64) -> SimlockResult<SimlockPoolSnapshot> {
        let mut store = self.0.store.lock().await;
        prune_expired(&mut store.leases, now_ms);
        let before = store.hosts.len();
        store.hosts.retain(|h| h.id != id);
        if store.hosts.len() == before {
            return Err(SimlockError::NotFound("Simlock host not found.".into()));
        }
        store.leases.retain(|lease| lease.host_id != id);
        persist_store(&self.0.path, &store)?;
        Ok(SimlockPoolSnapshot {
            policy: store.policy,
            hosts: store.hosts.clone(),
            active_leases: store.leases.clone(),
        })
    }

    /// Acquire or renew a Simlock device lease, enforcing policy, capacity, and exclusivity.
    pub async fn acquire_lease(
        &self,
        request: SimlockLeaseRequest,
        now_ms: i64,
    ) -> SimlockResult<SimlockLease> {
        let device_id = request.device_id.trim();
        let thread_id = request.thread_id.trim();
        if device_id.is_empty() || device_id.chars().any(char::is_control) {
            return Err(SimlockError::Invalid("Device ID is required.".into()));
        }
        if thread_id.is_empty() || thread_id.chars().any(char::is_control) {
            return Err(SimlockError::Invalid("Thread ID is required.".into()));
        }
        let mut store = self.0.store.lock().await;
        if store.policy == DeviceAccessPolicy::Off {
            return Err(SimlockError::Refused(
                "Device access is turned off in Settings → Devices.".into(),
            ));
        }
        prune_expired(&mut store.leases, now_ms);
        let host = store
            .hosts
            .iter()
            .find(|h| h.id == request.host_id)
            .cloned()
            .ok_or_else(|| SimlockError::NotFound("Simlock host not found.".into()))?;
        let ttl_ms = i64::from(host.lease_minutes.clamp(5, 240)) * 60_000;

        if let Some(existing) = store
            .leases
            .iter_mut()
            .find(|l| l.host_id == host.id && l.device_id == device_id)
        {
            if existing.thread_id == thread_id {
                existing.expires_at_ms = now_ms.saturating_add(ttl_ms);
                let renewed = existing.clone();
                persist_store(&self.0.path, &store)?;
                return Ok(renewed);
            }
            return Err(SimlockError::Conflict(format!(
                "Device {device_id} on {} is currently leased by another thread.",
                host.label
            )));
        }

        let active_on_host = store.leases.iter().filter(|l| l.host_id == host.id).count();
        if active_on_host >= usize::from(host.max_devices) {
            return Err(SimlockError::Conflict(format!(
                "{} reached its maximum of {} leased devices.",
                host.label, host.max_devices
            )));
        }

        let lease = SimlockLease {
            lease_id: Uuid::new_v4(),
            host_id: host.id,
            host_label: host.label,
            device_id: device_id.to_owned(),
            platform: request.platform,
            thread_id: thread_id.to_owned(),
            acquired_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
        };
        store.leases.push(lease.clone());
        let res = persist_store(&self.0.path, &store);
        drop(store);
        res?;
        Ok(lease)
    }

    pub async fn release_lease(&self, lease_id: Uuid) -> SimlockResult<bool> {
        let mut store = self.0.store.lock().await;
        let before = store.leases.len();
        store.leases.retain(|l| l.lease_id != lease_id);
        let removed = store.leases.len() != before;
        let res = if removed {
            persist_store(&self.0.path, &store)
        } else {
            Ok(())
        };
        drop(store);
        res?;
        Ok(removed)
    }

    pub async fn release_thread_leases(&self, thread_id: &str) -> SimlockResult<usize> {
        let mut store = self.0.store.lock().await;
        let before = store.leases.len();
        store.leases.retain(|l| l.thread_id != thread_id);
        let removed = before.saturating_sub(store.leases.len());
        let res = if removed > 0 {
            persist_store(&self.0.path, &store)
        } else {
            Ok(())
        };
        drop(store);
        res?;
        Ok(removed)
    }

    /// Probe `<origin>/health` on a configured Simlock host.
    pub async fn probe_host(&self, host_id: Uuid) -> SimlockResult<SimlockProbeResult> {
        let host = {
            let store = self.0.store.lock().await;
            store
                .hosts
                .iter()
                .find(|h| h.id == host_id)
                .cloned()
                .ok_or_else(|| SimlockError::NotFound("Simlock host not found.".into()))?
        };
        let url = format!("{}/health", host.origin.trim_end_matches('/'));
        let started = Instant::now();
        match self.0.client.get(&url).send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let ok = resp.status().is_success();
                let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                Ok(SimlockProbeResult {
                    host_id,
                    reachable: ok,
                    status_code: Some(code),
                    latency_ms: Some(latency_ms),
                    detail: if ok {
                        format!("{} responded HTTP {code} in {latency_ms} ms.", host.label)
                    } else {
                        format!("{} returned HTTP {code}.", host.label)
                    },
                })
            }
            Err(err) => Ok(SimlockProbeResult {
                host_id,
                reachable: false,
                status_code: None,
                latency_ms: None,
                detail: format!("Could not reach {}: {err}", host.label),
            }),
        }
    }
}

fn prune_expired(leases: &mut Vec<SimlockLease>, now_ms: i64) -> usize {
    let before = leases.len();
    leases.retain(|lease| lease.expires_at_ms > now_ms);
    before.saturating_sub(leases.len())
}

fn load_store(path: &Path) -> SimlockResult<Store> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(err) => Err(SimlockError::Storage(err)),
    }
}

fn persist_store(path: &Path, store: &Store) -> SimlockResult<()> {
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
    use bruhs_models::local_tools::DevicePlatform;

    #[tokio::test]
    async fn simlock_pool_enforces_policy_capacity_conflicts_and_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("simlock.json");
        let pool = SimlockPool::open(path.clone()).await.unwrap();

        let host_id = Uuid::new_v4();
        pool.upsert_host(
            SimlockHostSpec {
                id: host_id,
                label: "Mac Studio Lab".into(),
                origin: "http://127.0.0.1:4700".into(),
                max_devices: 1,
                lease_minutes: 10,
            },
            1_000,
        )
        .await
        .unwrap();

        // Off policy refuses leases
        pool.set_policy(DeviceAccessPolicy::Off, 1_000)
            .await
            .unwrap();
        assert!(matches!(
            pool.acquire_lease(
                SimlockLeaseRequest {
                    host_id,
                    device_id: "iphone-16".into(),
                    platform: DevicePlatform::Ios,
                    thread_id: "t1".into(),
                },
                1_000
            )
            .await,
            Err(SimlockError::Refused(_))
        ));

        // Persistent policy allows lease, rejects competing thread and exceeding max_devices
        pool.set_policy(DeviceAccessPolicy::Persistent, 1_000)
            .await
            .unwrap();
        let lease = pool
            .acquire_lease(
                SimlockLeaseRequest {
                    host_id,
                    device_id: "iphone-16".into(),
                    platform: DevicePlatform::Ios,
                    thread_id: "t1".into(),
                },
                1_000,
            )
            .await
            .unwrap();
        assert_eq!(lease.expires_at_ms, 1_000 + 10 * 60_000);

        assert!(matches!(
            pool.acquire_lease(
                SimlockLeaseRequest {
                    host_id,
                    device_id: "iphone-16".into(),
                    platform: DevicePlatform::Ios,
                    thread_id: "t2".into(),
                },
                2_000
            )
            .await,
            Err(SimlockError::Conflict(_))
        ));

        // Capacity of 1 blocks a second device while first is active
        assert!(matches!(
            pool.acquire_lease(
                SimlockLeaseRequest {
                    host_id,
                    device_id: "pixel-9".into(),
                    platform: DevicePlatform::Android,
                    thread_id: "t2".into(),
                },
                2_000
            )
            .await,
            Err(SimlockError::Conflict(_))
        ));

        // After 11 minutes, first lease expires and second succeeds
        let next = pool
            .acquire_lease(
                SimlockLeaseRequest {
                    host_id,
                    device_id: "pixel-9".into(),
                    platform: DevicePlatform::Android,
                    thread_id: "t2".into(),
                },
                1_000 + 11 * 60_000,
            )
            .await
            .unwrap();
        assert_eq!(next.device_id, "pixel-9");

        // Reopen from disk retains policy, host, and active lease
        let reopened = SimlockPool::open(path).await.unwrap();
        let snap = reopened.snapshot(1_000 + 12 * 60_000).await;
        assert_eq!(snap.policy, DeviceAccessPolicy::Persistent);
        assert_eq!(snap.hosts.len(), 1);
        assert_eq!(snap.active_leases.len(), 1);
        assert_eq!(snap.active_leases[0].lease_id, next.lease_id);
    }

    #[tokio::test]
    async fn simlock_probe_checks_live_health_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .await;
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let pool = SimlockPool::open(dir.path().join("simlock.json"))
            .await
            .unwrap();
        let host_id = Uuid::new_v4();
        pool.upsert_host(
            SimlockHostSpec {
                id: host_id,
                label: "Local Simlock".into(),
                origin: format!("http://{addr}"),
                max_devices: 2,
                lease_minutes: 30,
            },
            1_000,
        )
        .await
        .unwrap();

        let probe = pool.probe_host(host_id).await.unwrap();
        assert!(probe.reachable);
        assert_eq!(probe.status_code, Some(200));
        server.await.unwrap();
    }
}
