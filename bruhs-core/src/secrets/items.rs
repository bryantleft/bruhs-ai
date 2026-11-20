//! Bruhs' own keychain items, for backups (D5, D12, D13).
//!
//! A backup reads only the items Bruhs' catalogs name, under Bruhs' own
//! services. Values stay in [`Zeroizing`] buffers and leave memory only
//! sealed (kind 3) or inside an encrypted backup; nothing here logs,
//! prints or formats one.

use super::{storage, SecretError, SecretStore};
use base64::Engine as _;
use bruhs_backup::Key;
use bruhs_models::{
    backup::{valid_id, valid_stamp, PathRewrite},
    connections::ServiceProvider,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;

/// Saved keys, logins and app tokens: legacy ids and `vault:<scope>:<id>`.
pub const SECRETS_SERVICE: &str = "bruhs-ai-secrets";
/// Connection tokens, one account per provider id.
pub const CONNECTIONS_SERVICE: &str = "bruhs-ai-connections";
/// Google Calendar's tokens, under [`CALENDAR_ACCOUNT`] only.
pub const CALENDAR_SERVICE: &str = "bruhs-ai";
pub const CALENDAR_ACCOUNT: &str = "google-calendar";
/// Keys of sealed sets: `staged:<id>` and `pre-restore:<stamp>`. Never
/// part of a backup or a sealed set.
pub const BACKUP_SERVICE: &str = "bruhs-ai-backup";
/// Every service the item API reaches.
pub const ITEM_SERVICES: [&str; 4] = [
    SECRETS_SERVICE,
    CONNECTIONS_SERVICE,
    CALENDAR_SERVICE,
    BACKUP_SERVICE,
];

const OUTSIDE_ITEMS: &str = "Bruhs can only reach its own keychain items.";
const SEALED_SET_UNREADABLE: &str = "The saved keychain items could not be opened.";

/// Refuse an item outside Bruhs' own services: an unknown service, a
/// calendar account other than [`CALENDAR_ACCOUNT`], a provider Bruhs
/// doesn't know, or a seal key that isn't `staged:<id>` or
/// `pre-restore:<stamp>`.
pub(super) fn check_item(service: &str, account: &str) -> Result<(), SecretError> {
    let allowed = !account.is_empty()
        && !account.contains('\0')
        && match service {
            SECRETS_SERVICE => true,
            CONNECTIONS_SERVICE => ServiceProvider::ALL
                .iter()
                .any(|provider| provider.id() == account),
            CALENDAR_SERVICE => account == CALENDAR_ACCOUNT,
            BACKUP_SERVICE => seal_account(account),
            _ => false,
        };
    if allowed {
        Ok(())
    } else {
        Err(SecretError::Invalid(OUTSIDE_ITEMS))
    }
}

fn seal_account(account: &str) -> bool {
    account.strip_prefix("staged:").is_some_and(valid_id)
        || account
            .strip_prefix("pre-restore:")
            .is_some_and(valid_stamp)
}

/// Where a keychain item lives. Never holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ItemRef {
    pub service: String,
    pub account: String,
}

/// A keychain item and its value. Neither `Debug` nor `Clone`: the value
/// is wiped on drop and never printed.
#[derive(Serialize, Deserialize)]
pub struct KeychainItem {
    pub service: String,
    pub account: String,
    pub value: Zeroizing<String>,
}

/// Why [`read_items`] stopped (ruling F12).
#[derive(Debug, thiserror::Error)]
pub enum ItemsError {
    /// A read failed or waited past its bound, as an unanswered keychain
    /// prompt does: the owner needs to allow access.
    #[error("{}", bruhs_models::backup::KEYCHAIN_NEEDS_YOU)]
    NeedsOwner,
    /// The reads could not run: an item outside Bruhs' services, or a
    /// worker that stopped.
    #[error("Could not access the OS credential store. Unlock it and retry.")]
    Storage,
}

/// Every keychain item Bruhs' catalogs name (D5), sorted: `bruhs-ai-secrets`
/// for each legacy id and each `vault.json` row with an item (a newer
/// Bruhs' rows included), `bruhs-ai-connections` for each provider with a
/// `connections/<id>.json`, and the calendar item only when `calendar`
/// (the `calendar/google` state folder) holds something (ruling F11).
/// Reads the catalogs without the lock. Never names a seal key.
pub fn keychain_inventory(
    credentials: &Path,
    connections: &Path,
    calendar: &Path,
) -> Result<Vec<ItemRef>, SecretError> {
    let item = |service: &str, account: String| ItemRef {
        service: service.to_owned(),
        account,
    };
    let mut items: Vec<ItemRef> = storage::keychain_accounts(credentials)?
        .into_iter()
        .map(|account| item(SECRETS_SERVICE, account))
        .collect();
    items.extend(
        ServiceProvider::ALL
            .iter()
            .filter(|provider| {
                connections
                    .join(format!("{}.json", provider.id()))
                    .is_file()
            })
            .map(|provider| item(CONNECTIONS_SERVICE, provider.id().to_owned())),
    );
    let calendar_state = match fs::read_dir(calendar) {
        Ok(mut entries) => entries.next().is_some(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => return Err(SecretError::Catalog),
    };
    if calendar_state {
        items.push(item(CALENDAR_SERVICE, CALENDAR_ACCOUNT.to_owned()));
    }
    items.sort();
    items.dedup();
    Ok(items)
}

/// Read each item in its own blocking task, bounded by `wait`. A missing
/// item is skipped and counted; a read that fails or outlasts `wait` stops
/// everything with [`ItemsError::NeedsOwner`] (D5). An item outside
/// Bruhs' services is refused before any read.
pub async fn read_items(
    store: &SecretStore,
    refs: &[ItemRef],
    wait: Duration,
) -> Result<(Vec<KeychainItem>, u32), ItemsError> {
    if refs
        .iter()
        .any(|item| check_item(&item.service, &item.account).is_err())
    {
        return Err(ItemsError::Storage);
    }
    let mut items = Vec::with_capacity(refs.len());
    let mut missing = 0_u32;
    for item in refs {
        let (store, service, account) = (store.clone(), item.service.clone(), item.account.clone());
        let read = tokio::task::spawn_blocking(move || store.read_item(&service, &account));
        match tokio::time::timeout(wait, read).await {
            Ok(Ok(Ok(Some(value)))) => items.push(KeychainItem {
                service: item.service.clone(),
                account: item.account.clone(),
                value,
            }),
            Ok(Ok(Ok(None))) => missing = missing.saturating_add(1),
            Ok(Ok(Err(_))) | Err(_) => return Err(ItemsError::NeedsOwner),
            Ok(Err(_)) => return Err(ItemsError::Storage),
        }
    }
    Ok((items, missing))
}

/// Old keychain account to new, for staged items whose project moved (D12).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountMap(pub BTreeMap<String, String>);

impl AccountMap {
    /// Rename each `bruhs-ai-secrets` item the map names. Other services'
    /// accounts never hash a path, so they stay.
    pub fn apply(&self, items: &mut [KeychainItem]) {
        for item in items {
            if item.service != SECRETS_SERVICE {
                continue;
            }
            if let Some(account) = self.0.get(&item.account) {
                item.account.clone_from(account);
            }
        }
    }
}

/// Move the staged `credentials` catalogs to this computer's paths (D12):
/// legacy entries get the new path and the id `storage_key(new path,
/// name)`, map key and `id` together, and `vault.json` Project scopes move.
/// Paths are rewritten as text, never canonicalized. Answers the moved
/// accounts.
pub fn rekey_catalogs(
    credentials: &Path,
    rules: &[PathRewrite],
) -> Result<AccountMap, SecretError> {
    storage::rekey(credentials, rules).map(AccountMap)
}

#[derive(Serialize)]
struct SealedOut<'a> {
    items: &'a [KeychainItem],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedIn {
    items: Vec<KeychainItem>,
}

/// Counts bytes, so the plaintext buffer is sized once and never moves.
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Seal `items` as a kind 3 blob under `key` (D13). `random` must be fresh
/// OS-random bytes. The plaintext lives only in a wiped buffer.
pub fn seal_items(items: &[KeychainItem], random: [u8; 23], key: [u8; 32]) -> Vec<u8> {
    let set = SealedOut { items };
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, &set).expect("keychain items always serialize");
    // Sized up front: a growing Vec would leave copies of values behind.
    let mut plain = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *plain, &set).expect("keychain items always serialize");
    bruhs_backup::seal(&plain, &Key::from_bytes(key), random)
}

/// Open a set made by [`seal_items`]. A wrong key, another kind, a damaged
/// blob or an item outside Bruhs' services (a seal key included) is
/// `Invalid`, with fixed copy.
pub fn unseal_items(blob: &[u8], key: [u8; 32]) -> Result<Vec<KeychainItem>, SecretError> {
    let plain = bruhs_backup::unseal(blob, &Key::from_bytes(key))
        .map_err(|_| SecretError::Invalid(SEALED_SET_UNREADABLE))?;
    let set: SealedIn =
        serde_json::from_slice(&plain).map_err(|_| SecretError::Invalid(SEALED_SET_UNREADABLE))?;
    if set.items.iter().any(|item| {
        item.service == BACKUP_SERVICE || check_item(&item.service, &item.account).is_err()
    }) {
        return Err(SecretError::Invalid(SEALED_SET_UNREADABLE));
    }
    Ok(set.items)
}

/// Keep a sealed set's key under [`BACKUP_SERVICE`], base64, at `account`
/// (`staged:<id>` or `pre-restore:<stamp>`). Blocks on the keychain.
pub fn store_seal_key(
    store: &SecretStore,
    account: &str,
    key: &[u8; 32],
) -> Result<(), SecretError> {
    check_item(BACKUP_SERVICE, account)?;
    let text = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key));
    store.write_item(BACKUP_SERVICE, account, &text)
}

/// A sealed set's key. A missing key is `NotSaved`; a damaged one,
/// `Storage`. Blocks on the keychain.
pub fn read_seal_key(
    store: &SecretStore,
    account: &str,
) -> Result<Zeroizing<[u8; 32]>, SecretError> {
    let text = store
        .read_item(BACKUP_SERVICE, account)?
        .ok_or(SecretError::NotSaved)?;
    let bytes = Zeroizing::new(
        base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map_err(|_| SecretError::Storage)?,
    );
    let mut key = Zeroizing::new([0; 32]);
    if bytes.len() != key.len() {
        return Err(SecretError::Storage);
    }
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Remove a sealed set's key; a missing one is fine. Blocks on the keychain.
pub fn forget_seal_key(store: &SecretStore, account: &str) -> Result<(), SecretError> {
    store.remove_item(BACKUP_SERVICE, account)
}

#[cfg(test)]
mod tests;
