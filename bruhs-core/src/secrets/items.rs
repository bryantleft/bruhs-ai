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
    backup::{
        valid_id, valid_stamp, PathRewrite, DAMAGED, KEYCHAIN_ITEM_REFUSED, KEYCHAIN_NEEDS_YOU,
        KEYCHAIN_READ_FAILED, KEYCHAIN_SET_UNREADABLE, RESTORE_CANT_MAP,
    },
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

/// The longest account Bruhs writes is a vault account with a long bruh id.
const MAX_ACCOUNT_BYTES: usize = 512;

/// Refuse an item outside Bruhs' own services: an unknown service, a
/// secrets account of neither known shape, a calendar account other than
/// [`CALENDAR_ACCOUNT`], a provider Bruhs doesn't know, or a seal key that
/// isn't `staged:<id>` or `pre-restore:<stamp>`.
pub(super) fn check_item(service: &str, account: &str) -> Result<(), SecretError> {
    let allowed = !account.is_empty()
        && account.len() <= MAX_ACCOUNT_BYTES
        && !account.chars().any(char::is_control)
        && match service {
            SECRETS_SERVICE => secrets_account(account),
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
        Err(SecretError::Invalid(KEYCHAIN_ITEM_REFUSED))
    }
}

/// An item that may travel in a backup or a sealed set: one of Bruhs' own,
/// and never a seal key (ruling F11).
fn check_item_ref(service: &str, account: &str) -> Result<(), SecretError> {
    if service == BACKUP_SERVICE {
        return Err(SecretError::Invalid(KEYCHAIN_ITEM_REFUSED));
    }
    check_item(service, account)
}

/// Check items read from a backup before anything else sees them (B Task 7
/// ruling): each one of Bruhs' own and never a seal key, and no service
/// and account twice. Refused with fixed copy.
pub(crate) fn check_items(items: &[KeychainItem]) -> Result<(), SecretError> {
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        check_item_ref(&item.service, &item.account)?;
        if !seen.insert((item.service.as_str(), item.account.as_str())) {
            return Err(SecretError::Invalid(KEYCHAIN_SET_UNREADABLE));
        }
    }
    Ok(())
}

/// The two shapes of a `bruhs-ai-secrets` account: a legacy id (64
/// lowercase hex, `storage_key`) or `vault:<scope key>:v1-<32 hex>`, the
/// scope key `global`, `project:<16 hex>` or `bruh:<agent id>`.
fn secrets_account(account: &str) -> bool {
    let hex = |text: &str, len: usize| {
        text.len() == len
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if hex(account, 64) {
        return true;
    }
    let Some((scope, id)) = account
        .strip_prefix("vault:")
        .and_then(|rest| rest.rsplit_once(':'))
    else {
        return false;
    };
    id.strip_prefix("v1-").is_some_and(|id| hex(id, 32))
        && (scope == "global"
            || scope
                .strip_prefix("project:")
                .is_some_and(|key| hex(key, 16))
            || scope
                .strip_prefix("bruh:")
                .is_some_and(|agent| !agent.is_empty()))
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
    /// Base64 in every serialized form (ruling B Task 4 I2): base64 never
    /// needs a JSON escape, so reading it never copies the value through
    /// serde_json's unwiped scratch buffer.
    #[serde(with = "base64_value")]
    pub value: Zeroizing<String>,
}

mod base64_value {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde::{de, Deserializer, Serializer};
    use std::fmt;
    use zeroize::{Zeroize as _, Zeroizing};

    pub(super) fn serialize<S: Serializer>(
        value: &Zeroizing<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let text = Zeroizing::new(STANDARD.encode(value.as_bytes()));
        serializer.serialize_str(&text)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Zeroizing<String>, D::Error> {
        deserializer.deserialize_str(Value)
    }

    struct Value;

    impl de::Visitor<'_> for Value {
        type Value = Zeroizing<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a base64 value")
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
            decode(text).ok_or_else(|| E::custom("not a base64 value"))
        }
    }

    /// Decodes into a buffer sized once, then reuses it as the `String`.
    fn decode(text: &str) -> Option<Zeroizing<String>> {
        let mut bytes =
            Zeroizing::new(Vec::with_capacity(base64::decoded_len_estimate(text.len())));
        STANDARD.decode_vec(text, &mut bytes).ok()?;
        match String::from_utf8(std::mem::take(&mut *bytes)) {
            Ok(value) => Some(Zeroizing::new(value)),
            Err(error) => {
                error.into_bytes().zeroize();
                None
            }
        }
    }
}

/// Why [`read_items`] stopped (ruling F12).
#[derive(Debug, thiserror::Error)]
pub enum ItemsError {
    /// A read failed or waited past its bound, as an unanswered keychain
    /// prompt does: the owner needs to allow access.
    #[error("{}", KEYCHAIN_NEEDS_YOU)]
    NeedsOwner,
    /// A read's worker stopped.
    #[error("{}", KEYCHAIN_READ_FAILED)]
    Storage,
    /// An item outside Bruhs' services or a seal key: a caller's mistake,
    /// refused before any read.
    #[error("{}", KEYCHAIN_ITEM_REFUSED)]
    Refused,
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
    for provider in ServiceProvider::ALL {
        // A folder that can't be read fails the backup rather than
        // silently leaving a token out.
        match fs::metadata(connections.join(format!("{}.json", provider.id()))) {
            Ok(metadata) if metadata.is_file() => {
                items.push(item(CONNECTIONS_SERVICE, provider.id().to_owned()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(SecretError::Catalog),
        }
    }
    if calendar_state(calendar)? {
        items.push(item(CALENDAR_SERVICE, CALENDAR_ACCOUNT.to_owned()));
    }
    items.sort();
    items.dedup();
    Ok(items)
}

/// Whether the calendar folder holds state: a directory with an entry
/// that isn't hidden (a stray `.DS_Store` is not state). A missing path,
/// or one that isn't a directory, holds none.
fn calendar_state(calendar: &Path) -> Result<bool, SecretError> {
    match fs::metadata(calendar) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(SecretError::Catalog),
    }
    for entry in fs::read_dir(calendar).map_err(|_| SecretError::Catalog)? {
        let entry = entry.map_err(|_| SecretError::Catalog)?;
        if !entry.file_name().as_encoded_bytes().starts_with(b".") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Read each item in its own blocking task, bounded by `wait`. A missing
/// item is skipped and counted; a read that fails or outlasts `wait` stops
/// everything with [`ItemsError::NeedsOwner`] (D5). An item outside
/// Bruhs' services, or a seal key, is [`ItemsError::Refused`] before any
/// read.
pub async fn read_items(
    store: &SecretStore,
    refs: &[ItemRef],
    wait: Duration,
) -> Result<(Vec<KeychainItem>, u32), ItemsError> {
    if refs
        .iter()
        .any(|item| check_item_ref(&item.service, &item.account).is_err())
    {
        return Err(ItemsError::Refused);
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
pub fn rekey_catalogs(credentials: &Path, rules: &[PathRewrite]) -> Result<AccountMap, RekeyError> {
    storage::rekey(credentials, rules).map(AccountMap)
}

/// Why [`rekey_catalogs`] stopped. A failure after the first write leaves
/// the catalogs half moved: the caller discards them (B Task 4 note).
#[derive(Debug, thiserror::Error)]
pub enum RekeyError {
    /// A catalog doesn't read, or breaks its own rules.
    #[error("{}", DAMAGED)]
    Unreadable,
    /// Two entries land on one id once their paths move.
    #[error("{}", RESTORE_CANT_MAP)]
    Clash,
    /// A moved catalog couldn't be written.
    #[error("Bruhs couldn't write the moved catalogs.")]
    Write,
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
/// OS-random bytes. The plaintext lives only in a wiped buffer. An item
/// outside Bruhs' services, or a seal key, is refused, so every set made
/// here opens again.
pub fn seal_items(
    items: &[KeychainItem],
    random: [u8; 23],
    key: [u8; 32],
) -> Result<Vec<u8>, SecretError> {
    for item in items {
        check_item_ref(&item.service, &item.account)?;
    }
    let set = SealedOut { items };
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, &set).expect("keychain items always serialize");
    // Sized up front: a growing Vec would leave copies of values behind.
    let mut plain = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *plain, &set).expect("keychain items always serialize");
    Ok(bruhs_backup::seal(&plain, &Key::from_bytes(key), random))
}

/// Open a set made by [`seal_items`]. A wrong key, another kind, a damaged
/// blob or an item outside Bruhs' services (a seal key included) is
/// `Invalid`, with fixed copy.
pub fn unseal_items(blob: &[u8], key: [u8; 32]) -> Result<Vec<KeychainItem>, SecretError> {
    let unreadable = || SecretError::Invalid(KEYCHAIN_SET_UNREADABLE);
    let plain = bruhs_backup::unseal(blob, &Key::from_bytes(key)).map_err(|_| unreadable())?;
    let set: SealedIn = serde_json::from_slice(&plain).map_err(|_| unreadable())?;
    if set
        .items
        .iter()
        .any(|item| check_item_ref(&item.service, &item.account).is_err())
    {
        return Err(unreadable());
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
