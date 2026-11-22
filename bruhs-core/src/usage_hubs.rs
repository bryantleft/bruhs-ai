//! CLIProxyAPI hubs configured on this computer. Each account reports its own quota.
use crate::secret_sources::{self, SecretReference};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io, path::Path};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

mod antigravity;
pub(crate) use antigravity::fetch_available_models as antigravity_models;
pub(crate) use antigravity::read as antigravity_usage;
mod credits;
mod cursor;
pub(crate) use cursor::read as cursor_usage;
mod grok;
pub(crate) use grok::read as grok_usage;
mod grouping;
pub mod linked;
pub use grouping::{group_accounts, AccountGroup, Origin, Reading};
mod opencode;
pub(crate) use opencode::read as opencode_usage;
pub use opencode::ExternalUsage;
mod report;
pub use credits::{redeem, Redemption, ResetCredit, ResetCredits};
pub use report::{device_accounts, report, AccountUsage};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hub {
    pub url: String,
    pub label: String,
    revision: Uuid,
    key: Option<Key>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Key {
    Device(Uuid),
    Reference(SecretReference),
}
pub enum ManagementKey {
    Device(Zeroizing<String>),
    Reference(SecretReference),
}

pub fn normalize_url(value: &str) -> io::Result<Url> {
    let invalid = || {
        io::Error::other(
            "Enter an HTTP or HTTPS hub URL without embedded credentials, query or fragment.",
        )
    };
    let mut url = Url::parse(value.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let mut segments = url.path().trim_end_matches('/').to_owned();
    if segments.to_ascii_lowercase().ends_with("/management.html") {
        segments.truncate(segments.len() - "/management.html".len());
    }
    url.set_path(&format!("{segments}/"));
    Ok(url)
}

fn entry(id: Uuid) -> io::Result<keyring::Entry> {
    keyring::Entry::new("bruhs-ai-usage-hubs", &id.to_string()).map_err(|_| key_error())
}
fn key_error() -> io::Error {
    io::Error::other("Could not access the management key in the system credential store.")
}
impl Hub {
    async fn checked_key(&self) -> io::Result<Zeroizing<String>> {
        let key = self.management_key().await?;
        // The saved revision can change while the user unlocks their credential store.
        let rows = tokio::task::spawn_blocking(list)
            .await
            .map_err(io::Error::other)??;
        if !rows.contains(self) {
            return Err(stale());
        }
        Ok(key)
    }

    async fn management_key(&self) -> io::Result<Zeroizing<String>> {
        match &self.key {
            Some(Key::Device(id)) => {
                let id = *id;
                tokio::task::spawn_blocking(move || {
                    entry(id)?
                        .get_password()
                        .map(Zeroizing::new)
                        .map_err(|_| key_error())
                })
                .await
                .map_err(io::Error::other)?
            }
            Some(Key::Reference(reference)) => reference.read().await,
            None => Err(io::Error::other(
                "Add this hub again to set its management key.",
            )),
        }
    }
}

fn read<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> io::Result<T> {
    match fs::File::open(path) {
        Ok(file) => Ok(serde_json::from_reader(file)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}
fn write<T: Serialize>(root: &Path, name: &str, value: &T) -> io::Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(&mut tmp, value)?;
    tmp.as_file().sync_all()?;
    tmp.persist(root.join(name))?;
    Ok(())
}
const FILE: &str = "usage-hubs.json";
const JOURNAL: &str = "usage-hub-keys.json";
pub fn list() -> io::Result<Vec<Hub>> {
    read(&crate::agent_extensions::directory()?.join(FILE))
}

fn device_keys(rows: &[Hub]) -> BTreeSet<Uuid> {
    rows.iter()
        .filter_map(|row| match row.key {
            Some(Key::Device(id)) => Some(id),
            _ => None,
        })
        .collect()
}
type NewKey = Option<(Uuid, Zeroizing<String>)>;
fn edit_at(
    root: &Path,
    change: impl FnOnce(&mut Vec<Hub>) -> io::Result<NewKey>,
) -> io::Result<Vec<Hub>> {
    crate::agent_extensions::private_directory(root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("usage-hubs.lock"))?;
    lock.lock()?;
    let mut rows: Vec<Hub> = read(&root.join(FILE))?;
    let previous = device_keys(&rows);
    let new_key = change(&mut rows)?;
    let next = device_keys(&rows);
    let mut pending: BTreeSet<Uuid> = read(&root.join(JOURNAL))?;
    pending.extend(previous.symmetric_difference(&next));
    if !pending.is_empty() {
        write(root, JOURNAL, &pending)?;
    }
    let result = (|| {
        if let Some((id, value)) = new_key {
            entry(id)?.set_password(&value).map_err(|_| key_error())?;
        }
        write(root, FILE, &rows)
    })();
    if !pending.is_empty() {
        let live = if result.is_ok() { &next } else { &previous };
        pending.retain(|id| {
            !live.contains(id)
                && entry(*id).map_or(true, |key| {
                    !matches!(
                        key.delete_credential(),
                        Ok(()) | Err(keyring::Error::NoEntry)
                    )
                })
        });
        // Retrying the old journal is safe after a failed cleanup write.
        if write(root, JOURNAL, &pending).is_err() || !pending.is_empty() {
            tracing::warn!("Usage hub credential cleanup will retry on the next edit");
        }
    }
    result.map(|_| rows)
}

pub fn save(
    url: &str,
    label: &str,
    key: ManagementKey,
    previous: Option<&Hub>,
) -> io::Result<Vec<Hub>> {
    let url = normalize_url(url)?;
    let label = if label.trim().is_empty() {
        url.host_str().unwrap_or_default()
    } else {
        label.trim()
    }
    .to_owned();
    edit_at(&crate::agent_extensions::directory()?, |rows| {
        let existing = rows.iter().position(|row| row.url == url.as_str());
        if existing.map(|i| &rows[i]) != previous {
            return Err(stale());
        }
        let revision = Uuid::new_v4();
        let (key, value) = match key {
            ManagementKey::Device(value) if !value.trim().is_empty() => {
                (Key::Device(revision), Some((revision, value)))
            }
            ManagementKey::Device(_) => return Err(io::Error::other("Enter a management key.")),
            ManagementKey::Reference(reference) => {
                secret_sources::validate_reference(&reference.reference)?;
                if !secret_sources::source_exists(&reference.source)? {
                    return Err(io::Error::other("Choose an available 1Password source."));
                }
                (Key::Reference(reference), None)
            }
        };
        let hub = Hub {
            url: url.into(),
            label,
            revision,
            key: Some(key),
        };
        match existing {
            Some(i) => rows[i] = hub,
            None => rows.push(hub),
        }
        Ok(value)
    })
}
fn stale() -> io::Error {
    io::Error::other("This hub changed in another window. Refresh and try again.")
}
pub fn remove(hub: &Hub) -> io::Result<Vec<Hub>> {
    edit_at(&crate::agent_extensions::directory()?, |rows| {
        let i = rows.iter().position(|row| row == hub).ok_or_else(stale)?;
        rows.remove(i);
        Ok(None)
    })
}
fn uses(hub: &Hub, source: &str) -> bool {
    matches!(&hub.key, Some(Key::Reference(r)) if r.source == source)
}
pub fn source_consumers(source: &str) -> io::Result<Vec<String>> {
    Ok(list()?
        .into_iter()
        .filter(|hub| uses(hub, source))
        .map(|hub| format!("{} · Hub management key", hub.label))
        .collect())
}
pub(crate) fn unset_source(source: &str) -> io::Result<()> {
    edit_at(&crate::agent_extensions::directory()?, |rows| {
        for row in rows.iter_mut().filter(|row| uses(row, source)) {
            row.key = None;
            row.revision = Uuid::new_v4();
        }
        Ok(None)
    })
    .map(drop)
}
