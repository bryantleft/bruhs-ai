//! Vault entries beside the legacy catalog (D4, D5).
//!
//! A Project key stays in `catalog.json` under today's id and account, so
//! older binaries keep reading it (ruling F1). Every other entry is a
//! value-free row in `vault.json`, id `v1-<uuid>`, keychain account
//! `vault:<scope key>:<id>`. Both files change only under `catalog.lock`;
//! value reads never take it (ruling F13).

use super::{
    read_catalog, storage_key, write_atomically, Backend, Catalog, Keys, SecretError, SecretStore,
    Vault,
};
use bruhs_models::{
    secrets::{SavedCredential, SavedCredentialState},
    vault::{
        base32_decode, base32_encode, parse_totp_setup, CredentialScope, LoginFields, LoginInput,
        LoginMeta, SavedSource, TotpAlgorithm, TotpInput, VaultCreate, VaultEntry, VaultKind,
        VaultSecretInput, VaultUpdate, DUPLICATE_NAME,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    io::BufReader,
    path::Path,
    sync::PoisonError,
};
use zeroize::Zeroizing;

const VAULT_FILE: &str = "vault.json";
const VAULT_VERSION: u32 = 1;
const ID_PREFIX: &str = "v1-";

/// `vault.json`: every entry that isn't a Project key. Never holds a value.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VaultCatalog {
    version: u32,
    entries: BTreeMap<String, VaultEntry>,
    /// Per entry, how many times its keychain item was written or removed.
    /// An update that read the item before the lock writes only if this
    /// is still what it read, so it never writes back an older value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    revisions: BTreeMap<String, u64>,
}

impl Default for VaultCatalog {
    fn default() -> Self {
        Self {
            version: VAULT_VERSION,
            entries: BTreeMap::new(),
            revisions: BTreeMap::new(),
        }
    }
}

impl VaultCatalog {
    fn revision(&self, id: &str) -> u64 {
        self.revisions.get(id).copied().unwrap_or_default()
    }
}

/// An entry's keychain values, read for one use. No `Debug` or `Clone`.
/// Only the fields of the entry's kind are set.
#[cfg_attr(not(test), expect(dead_code, reason = "used from Task 6"))]
pub(in crate::secrets) struct VaultSecret {
    /// A key's value.
    pub value: Option<Zeroizing<String>>,
    /// A Saved login's password.
    pub password: Option<Zeroizing<String>>,
    /// A Saved login's one-time code setup.
    pub totp: Option<StoredTotp>,
}

/// A Saved login's one-time code setup. No `Debug` or `Clone`.
pub(in crate::secrets) struct StoredTotp {
    pub secret: Zeroizing<Vec<u8>>,
    pub digits: u32,
    pub period: u64,
    pub algorithm: TotpAlgorithm,
}

/// The keychain blob of a Saved login, as written: borrowed, so the values
/// are never copied out of their `Zeroizing` homes.
#[derive(Serialize)]
struct BlobOut<'a> {
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    totp: Option<TotpOut<'a>>,
}

/// `secret` is base32.
#[derive(Serialize)]
struct TotpOut<'a> {
    secret: &'a str,
    digits: u32,
    period: u64,
    algorithm: TotpAlgorithm,
}

/// The blob as read. Each string moves into `Zeroizing` as soon as it is
/// parsed.
#[derive(Deserialize)]
struct BlobIn {
    password: String,
    #[serde(default)]
    totp: Option<TotpIn>,
}

#[derive(Deserialize)]
struct TotpIn {
    secret: String,
    digits: u32,
    period: u64,
    algorithm: TotpAlgorithm,
}

/// A Saved login's keychain values.
struct LoginBlob {
    password: Zeroizing<String>,
    totp: Option<StoredTotp>,
}

impl LoginBlob {
    fn to_json(&self) -> Result<Zeroizing<String>, SecretError> {
        let secret = self
            .totp
            .as_ref()
            .map(|totp| Zeroizing::new(base32_encode(&totp.secret)));
        let blob = BlobOut {
            password: &self.password,
            totp: self
                .totp
                .as_ref()
                .zip(secret.as_ref())
                .map(|(totp, secret)| TotpOut {
                    secret,
                    digits: totp.digits,
                    period: totp.period,
                    algorithm: totp.algorithm,
                }),
        };
        // Sized up front (escapes take at most six bytes per byte), so the
        // buffer never reallocates and leaves a copy behind.
        let capacity = (self.password.len() + secret.as_ref().map_or(0, |s| s.len())) * 6 + 128;
        let mut bytes = Zeroizing::new(Vec::with_capacity(capacity));
        serde_json::to_writer(&mut *bytes, &blob).map_err(|_| SecretError::Storage)?;
        String::from_utf8(std::mem::take(&mut *bytes))
            .map(Zeroizing::new)
            .map_err(|_| SecretError::Storage)
    }

    fn parse(text: &str) -> Result<Self, SecretError> {
        let blob: BlobIn = serde_json::from_str(text).map_err(|_| SecretError::NotSaved)?;
        let password = Zeroizing::new(blob.password);
        let totp = match blob.totp {
            None => None,
            Some(totp) => {
                let encoded = Zeroizing::new(totp.secret);
                let secret = Zeroizing::new(base32_decode(&encoded).ok_or(SecretError::NotSaved)?);
                Some(StoredTotp {
                    secret,
                    digits: totp.digits,
                    period: totp.period,
                    algorithm: totp.algorithm,
                })
            }
        };
        Ok(Self { password, totp })
    }
}

/// What an update does to a Saved login's one-time code setup. A pasted
/// setup moves into `Zeroizing` before anything can fail.
enum TotpChange {
    Keep,
    Remove,
    Set(Zeroizing<String>),
}

impl From<TotpInput> for TotpChange {
    fn from(input: TotpInput) -> Self {
        match input {
            TotpInput::Keep => Self::Keep,
            TotpInput::Remove => Self::Remove,
            TotpInput::Set { setup } => Self::Set(Zeroizing::new(setup)),
        }
    }
}

/// What a mutation does to an entry's keychain item.
enum Item {
    /// Leave it, and the entry's state, as they are.
    Keep,
    Write(Zeroizing<String>),
    /// Remove it (a missing item is fine).
    Remove,
}

/// A Project key lives in the legacy catalog (ruling F1).
fn is_legacy_kind(scope: &CredentialScope, kind: &VaultKind) -> bool {
    matches!(scope, CredentialScope::Project { .. }) && *kind == VaultKind::Value
}

/// The keychain account of an entry (D5): the id for a legacy entry,
/// `vault:<scope key>:<id>` otherwise.
fn account(entry: &VaultEntry) -> String {
    if is_legacy_kind(&entry.scope, &entry.kind) {
        entry.id.clone()
    } else {
        format!("vault:{}:{}", entry.scope.key(), entry.id)
    }
}

/// Whether an entry keeps a keychain item: keys and Saved logins.
fn has_item(kind: &VaultKind) -> bool {
    matches!(
        kind,
        VaultKind::Value
            | VaultKind::Login(LoginMeta {
                fields: LoginFields::Saved { .. },
                ..
            })
    )
}

fn legacy_entry(credential: &SavedCredential) -> VaultEntry {
    VaultEntry {
        id: credential.id.clone(),
        name: credential.name.clone(),
        scope: CredentialScope::Project {
            path: credential.project_path.clone(),
        },
        kind: VaultKind::Value,
        state: credential.state,
        attention: None,
    }
}

fn scope_order(scope: &CredentialScope) -> (u8, &str) {
    match scope {
        CredentialScope::Global => (0, ""),
        CredentialScope::Project { path } => (1, path),
        CredentialScope::Bruh { agent_id } => (2, agent_id),
    }
}

/// Both files as one listing, sorted by scope then name.
fn merged(catalog: &Catalog, vault: &VaultCatalog) -> Vec<VaultEntry> {
    let mut entries: Vec<_> = catalog
        .entries
        .values()
        .map(legacy_entry)
        .chain(vault.entries.values().cloned())
        .collect();
    entries.sort_by(|a, b| {
        (scope_order(&a.scope), &a.name, &a.id).cmp(&(scope_order(&b.scope), &b.name, &b.id))
    });
    entries
}

fn new_id() -> String {
    format!("{ID_PREFIX}{}", uuid::Uuid::new_v4().simple())
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|rest| {
        rest.len() == 32
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Read `vault.json`. A missing file is an empty vault; a damaged one, an
/// unknown version or a row that breaks the file's rules is an error,
/// never an empty list.
pub(super) fn read_vault_catalog(directory: &Path) -> Result<VaultCatalog, SecretError> {
    let mut catalog: VaultCatalog = match File::open(directory.join(VAULT_FILE)) {
        Ok(file) => {
            serde_json::from_reader(BufReader::new(file)).map_err(|_| SecretError::Catalog)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => VaultCatalog::default(),
        Err(_) => return Err(SecretError::Catalog),
    };
    if catalog.version != VAULT_VERSION {
        return Err(SecretError::Catalog);
    }
    for (id, entry) in &mut catalog.entries {
        if *id != entry.id || !valid_id(id) || is_legacy_kind(&entry.scope, &entry.kind) {
            return Err(SecretError::Catalog);
        }
        // Attention lives in memory only.
        entry.attention = None;
    }
    Ok(catalog)
}

/// The keychain item for a new or updated entry's values.
fn new_login_blob(
    password: Zeroizing<String>,
    totp: Option<StoredTotp>,
) -> Result<Item, SecretError> {
    Ok(Item::Write(LoginBlob { password, totp }.to_json()?))
}

fn parsed_totp(setup: &str) -> Result<StoredTotp, SecretError> {
    let setup = parse_totp_setup(setup).map_err(SecretError::Invalid)?;
    Ok(StoredTotp {
        secret: Zeroizing::new(setup.secret),
        digits: setup.digits,
        period: setup.period,
        algorithm: setup.algorithm,
    })
}

impl Vault {
    /// `vault.json`, read on first use under the lock.
    fn vault(&mut self) -> Result<&mut VaultCatalog, SecretError> {
        if self.vault.is_none() {
            let directory = self.directory.as_deref().ok_or(SecretError::Catalog)?;
            self.vault = Some(read_vault_catalog(directory)?);
        }
        self.vault.as_mut().ok_or(SecretError::Catalog)
    }

    fn persist_vault(&self) -> Result<(), SecretError> {
        match (&self.directory, &self.vault) {
            (Some(directory), Some(vault)) => write_atomically(directory, VAULT_FILE, vault),
            _ => Ok(()),
        }
    }

    /// Names are unique per scope across both files, compared trimmed.
    fn ensure_unique(&mut self, scope: &CredentialScope, name: &str) -> Result<(), SecretError> {
        let legacy = match scope {
            CredentialScope::Project { path } => self
                .catalog
                .entries
                .values()
                .any(|entry| entry.project_path == *path && entry.name.trim() == name),
            CredentialScope::Global | CredentialScope::Bruh { .. } => false,
        };
        let vault = self
            .vault()?
            .entries
            .values()
            .any(|entry| entry.scope == *scope && entry.name.trim() == name);
        if legacy || vault {
            return Err(SecretError::Invalid(DUPLICATE_NAME));
        }
        Ok(())
    }

    /// Store a `vault.json` row, intent first: a row whose item changes is
    /// persisted as needing attention with its revision bumped, the item
    /// is written or removed, then the row is persisted as saved.
    fn put(&mut self, mut entry: VaultEntry, item: Item) -> Result<VaultEntry, SecretError> {
        let id = entry.id.clone();
        entry.attention = None;
        let changes_item = !matches!(item, Item::Keep);
        if changes_item {
            entry.state = SavedCredentialState::NeedsAttention;
        }
        let account = account(&entry);
        let vault = self.vault()?;
        vault.entries.insert(id.clone(), entry.clone());
        if changes_item {
            let revision = vault.revisions.entry(id.clone()).or_default();
            *revision = revision.wrapping_add(1);
        }
        self.persist_vault()?;
        match item {
            Item::Keep => return Ok(entry),
            Item::Write(value) => self.keys.write(&account, &value)?,
            Item::Remove => self.keys.remove(&account)?,
        }
        let entry = self
            .vault()?
            .entries
            .get_mut(&id)
            .ok_or(SecretError::Catalog)?;
        entry.state = SavedCredentialState::Saved;
        let entry = entry.clone();
        self.persist_vault()?;
        Ok(entry)
    }

    /// Remove a `vault.json` row and its item, intent first.
    fn remove(&mut self, id: &str) -> Result<(), SecretError> {
        let Some(entry) = self.vault()?.entries.get_mut(id) else {
            return Err(SecretError::NotSaved);
        };
        entry.state = SavedCredentialState::NeedsAttention;
        let account = account(entry);
        self.persist_vault()?;
        // Every kind: a 1Password entry may keep a Saved login's old blob.
        self.keys.remove(&account)?;
        let vault = self.vault()?;
        vault.entries.remove(id);
        vault.revisions.remove(id);
        self.persist_vault()
    }
}

/// A metadata snapshot and a keychain handle, taken without the lock.
struct Snapshot {
    catalog: Catalog,
    /// `None` when `vault.json` can't be read; legacy rows still can.
    vault: Option<VaultCatalog>,
    keys: Keys,
}

impl Snapshot {
    fn vault(&self) -> Result<&VaultCatalog, SecretError> {
        self.vault.as_ref().ok_or(SecretError::Catalog)
    }

    fn entries(&self) -> Result<Vec<VaultEntry>, SecretError> {
        Ok(merged(&self.catalog, self.vault()?))
    }

    /// A legacy row needs only `catalog.json`.
    fn entry(&self, id: &str) -> Result<Option<VaultEntry>, SecretError> {
        match self.catalog.entries.get(id) {
            Some(credential) => Ok(Some(legacy_entry(credential))),
            None => Ok(self.vault()?.entries.get(id).cloned()),
        }
    }
}

impl SecretStore {
    /// Read both files without the lock: atomic renames give a coherent
    /// snapshot even while a keychain prompt holds a writer.
    async fn snapshot(&self) -> Result<Snapshot, SecretError> {
        let backend = self.0.clone();
        tokio::task::spawn_blocking(move || match backend {
            Backend::Os => {
                let directory = super::catalog_directory()?;
                Ok(Snapshot {
                    catalog: read_catalog(&directory)?,
                    vault: read_vault_catalog(&directory).ok(),
                    keys: Keys::Os,
                })
            }
            Backend::Memory(vault) => {
                let vault = vault.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(Snapshot {
                    catalog: vault.catalog.clone(),
                    vault: vault.vault.clone(),
                    keys: vault.keys.handle(),
                })
            }
            #[cfg(test)]
            Backend::Unavailable => Err(SecretError::Storage),
            #[cfg(test)]
            Backend::FileTest(directory, keys) => Ok(Snapshot {
                catalog: read_catalog(&directory)?,
                vault: read_vault_catalog(&directory).ok(),
                keys: Keys::Memory(keys),
            }),
        })
        .await
        .map_err(|_| SecretError::Catalog)?
    }

    /// Every entry, value free: Project keys from the legacy catalog and
    /// the rest from `vault.json`, sorted by scope then name. Never reads
    /// the keychain.
    pub async fn vault_list(&self) -> Result<Vec<VaultEntry>, SecretError> {
        self.snapshot().await?.entries()
    }

    pub async fn vault_entry(&self, id: &str) -> Result<Option<VaultEntry>, SecretError> {
        self.snapshot().await?.entry(id)
    }

    /// Create an entry. The caller has canonicalized a project path and
    /// checked that a bruh exists; this checks the rest (D6), including a
    /// name unique in its scope across both files.
    pub async fn vault_create(&self, mut create: VaultCreate) -> Result<VaultEntry, SecretError> {
        if let Err(error) = create.validate() {
            scrub(create.secret);
            return Err(SecretError::Invalid(error));
        }
        let kind = create.secret.kind_meta().unwrap_or(VaultKind::Value);
        let VaultCreate {
            name,
            scope,
            secret,
        } = create;
        let item = match secret {
            VaultSecretInput::Value { value } => Item::Write(Zeroizing::new(
                value.ok_or(SecretError::Invalid(bruhs_models::vault::ENTER_VALUE))?,
            )),
            VaultSecretInput::OnePassword { .. } => Item::Keep,
            VaultSecretInput::Login { fields, .. } => match fields {
                LoginInput::OnePassword { .. } => Item::Keep,
                LoginInput::Saved { password, totp, .. } => {
                    let totp = TotpChange::from(totp);
                    let password = Zeroizing::new(
                        password
                            .ok_or(SecretError::Invalid(bruhs_models::vault::ENTER_PASSWORD))?,
                    );
                    let totp = match totp {
                        TotpChange::Set(setup) => Some(parsed_totp(&setup)?),
                        TotpChange::Keep | TotpChange::Remove => None,
                    };
                    new_login_blob(password, totp)?
                }
            },
        };
        self.access(move |vault| {
            vault.ensure_unique(&scope, &name)?;
            if let (CredentialScope::Project { path }, VaultKind::Value, Item::Write(value)) =
                (&scope, &kind, &item)
            {
                let id = storage_key(Path::new(path), &name);
                let credential = SavedCredential {
                    id,
                    name: name.clone(),
                    project_path: path.clone(),
                    state: SavedCredentialState::NeedsAttention,
                };
                vault.save(credential.clone(), value)?;
                return Ok(legacy_entry(&SavedCredential {
                    state: SavedCredentialState::Saved,
                    ..credential
                }));
            }
            let entry = VaultEntry {
                id: new_id(),
                name,
                scope,
                kind,
                state: SavedCredentialState::Saved,
                attention: None,
            };
            vault.put(entry, item)
        })
        .await
    }

    /// Update an entry's values and metadata; name, scope, kind and a
    /// login's origin stay. A missing value keeps the stored one. A Project
    /// key accepts only a new value, through today's `replace`.
    pub async fn vault_update(
        &self,
        id: &str,
        update: VaultUpdate,
    ) -> Result<VaultEntry, SecretError> {
        let checked = match self.snapshot().await {
            Ok(snapshot) => match snapshot.entry(id) {
                Ok(Some(existing)) => update
                    .validate(&existing.kind)
                    .map(|()| (snapshot, existing))
                    .map_err(SecretError::Invalid),
                Ok(None) => Err(SecretError::NotSaved),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let (snapshot, existing) = match checked {
            Ok(checked) => checked,
            Err(error) => {
                scrub(update.secret);
                return Err(error);
            }
        };
        let kind = update.secret.kind_meta().unwrap_or(VaultKind::Value);
        if snapshot.catalog.entries.contains_key(id) {
            // Validation left a Value: a new one replaces, none keeps it.
            return match update.secret {
                VaultSecretInput::Value { value: Some(value) } => {
                    self.replace(id, value).await?;
                    self.vault_entry(id).await?.ok_or(SecretError::NotSaved)
                }
                _ => Ok(existing),
            };
        }
        let (kind, item) = match update.secret {
            VaultSecretInput::Value { value } => (
                kind,
                value.map_or(Item::Keep, |value| Item::Write(Zeroizing::new(value))),
            ),
            VaultSecretInput::OnePassword { .. }
            | VaultSecretInput::Login {
                fields: LoginInput::OnePassword { .. },
                ..
            } => (kind, removal(&existing)),
            VaultSecretInput::Login {
                fields: LoginInput::Saved { password, totp, .. },
                ..
            } => {
                let (password, totp) = (password.map(Zeroizing::new), TotpChange::from(totp));
                saved_login_change(&snapshot, &existing, kind, password, totp).await?
            }
        };
        let revision = snapshot.vault()?.revision(id);
        let id = id.to_owned();
        self.access(move |vault| {
            let catalog = vault.vault()?;
            match catalog.entries.get(&id) {
                None => Err(SecretError::NotSaved),
                // Another save changed the row or its item since the
                // snapshot: writing now could put back an older value.
                Some(current) if *current != existing || catalog.revision(&id) != revision => {
                    Err(SecretError::Conflict)
                }
                Some(_) => vault.put(VaultEntry { kind, ..existing }, item),
            }
        })
        .await
    }

    /// Delete an entry and its keychain item. A Project key goes through
    /// today's `delete_saved`.
    pub async fn vault_delete(&self, id: &str) -> Result<(), SecretError> {
        let id = id.to_owned();
        self.access(move |vault| {
            if vault.catalog.entries.contains_key(&id) {
                vault.delete_saved(&id)
            } else {
                vault.remove(&id)
            }
        })
        .await
    }

    /// Forget the own entries of bruhs that were deleted (ruling F15:
    /// `gone` holds the deleted bruhs' ids, never "not in this list").
    /// Returns how many entries were removed.
    pub async fn forget_bruh_entries(&self, gone: &HashSet<String>) -> Result<usize, SecretError> {
        if gone.is_empty() {
            return Ok(0);
        }
        let gone = gone.clone();
        self.access(move |vault| {
            let ids: Vec<_> = vault
                .vault()?
                .entries
                .values()
                .filter(|entry| {
                    matches!(&entry.scope, CredentialScope::Bruh { agent_id } if gone.contains(agent_id))
                })
                .map(|entry| entry.id.clone())
                .collect();
            for id in &ids {
                vault.remove(id)?;
            }
            Ok(ids.len())
        })
        .await
    }

    /// The entry a thread's credential request named `name` in `project`
    /// matches (D17): the project's own key, then a 1Password entry for the
    /// project, then a Global key or 1Password entry. Logins never match.
    /// An unindexed legacy value is registered as `find` does.
    #[cfg_attr(not(test), expect(dead_code, reason = "used from Task 7"))]
    pub(in crate::secrets) async fn request_match(
        &self,
        project: &str,
        name: &str,
    ) -> Result<Option<(String, SavedSource)>, SecretError> {
        let snapshot = self.snapshot().await?;
        let key = storage_key(Path::new(project), name);
        let indexed = snapshot
            .catalog
            .entries
            .get(&key)
            .is_some_and(|entry| entry.state == SavedCredentialState::Saved);
        if indexed || self.find(project, name).await?.is_some() {
            return Ok(Some((key, SavedSource::Project)));
        }
        let vault = snapshot.vault()?;
        let usable =
            |entry: &&VaultEntry| entry.name == name && entry.state == SavedCredentialState::Saved;
        let project_reference = vault.entries.values().filter(usable).find(|entry| {
            matches!(&entry.scope, CredentialScope::Project { path } if path == project)
                && matches!(entry.kind, VaultKind::OnePassword { .. })
        });
        if let Some(entry) = project_reference {
            return Ok(Some((entry.id.clone(), SavedSource::OnePassword)));
        }
        Ok(vault
            .entries
            .values()
            .filter(usable)
            .filter(|entry| entry.scope == CredentialScope::Global)
            .find_map(|entry| match entry.kind {
                VaultKind::Value => Some((entry.id.clone(), SavedSource::Global)),
                VaultKind::OnePassword { .. } => Some((entry.id.clone(), SavedSource::OnePassword)),
                VaultKind::Login(_) => None,
            }))
    }

    /// An entry's keychain values, read without the catalog lock (ruling
    /// F13). `Ok(None)` only for a 1Password kind, which keeps no item; an
    /// unknown entry, or a key or Saved login whose item is missing, is
    /// `NotSaved`.
    #[cfg_attr(not(test), expect(dead_code, reason = "used from Task 6"))]
    pub(in crate::secrets) async fn vault_secret(
        &self,
        id: &str,
    ) -> Result<Option<VaultSecret>, SecretError> {
        let snapshot = self.snapshot().await?;
        let entry = snapshot.entry(id)?.ok_or(SecretError::NotSaved)?;
        if !has_item(&entry.kind) {
            return Ok(None);
        }
        let account = account(&entry);
        let keys = snapshot.keys;
        let text = tokio::task::spawn_blocking(move || keys.read(&account))
            .await
            .map_err(|_| SecretError::Catalog)??
            .ok_or(SecretError::NotSaved)?;
        Ok(Some(match entry.kind {
            VaultKind::Login(_) => {
                let blob = LoginBlob::parse(&text)?;
                VaultSecret {
                    value: None,
                    password: Some(blob.password),
                    totp: blob.totp,
                }
            }
            VaultKind::Value | VaultKind::OnePassword { .. } => VaultSecret {
                value: Some(text),
                password: None,
                totp: None,
            },
        }))
    }
}

/// The metadata and keychain item of a Saved login update. A missing
/// password keeps the stored one and `Keep` keeps a stored setup key (and
/// `totp: true`); a kept part is read before the lock (ruling F13).
async fn saved_login_change(
    snapshot: &Snapshot,
    existing: &VaultEntry,
    kind: VaultKind,
    password: Option<Zeroizing<String>>,
    totp: TotpChange,
) -> Result<(VaultKind, Item), SecretError> {
    let (stored_login, stored_totp) = match &existing.kind {
        VaultKind::Login(LoginMeta {
            fields: LoginFields::Saved { totp, .. },
            ..
        }) => (true, *totp),
        _ => (false, false),
    };
    let keep_totp = matches!(totp, TotpChange::Keep);
    let unchanged = stored_login && password.is_none() && keep_totp;
    let keeps_part = stored_login && (password.is_none() || (stored_totp && keep_totp));
    let stored = if keeps_part && !unchanged {
        let keys = snapshot.keys.handle();
        let account = account(existing);
        let text = tokio::task::spawn_blocking(move || keys.read(&account))
            .await
            .map_err(|_| SecretError::Catalog)??
            .ok_or(SecretError::NotSaved)?;
        Some(LoginBlob::parse(&text)?)
    } else {
        None
    };
    let (stored_password, stored_setup) = match stored {
        Some(blob) => (Some(blob.password), blob.totp),
        None => (None, None),
    };
    let totp = match totp {
        TotpChange::Set(setup) => Some(parsed_totp(&setup)?),
        TotpChange::Remove => None,
        TotpChange::Keep => stored_setup,
    };
    let has_totp = if unchanged {
        stored_totp
    } else {
        totp.is_some()
    };
    let kind = match kind {
        VaultKind::Login(mut meta) => {
            if let LoginFields::Saved { totp, .. } = &mut meta.fields {
                *totp = has_totp;
            }
            VaultKind::Login(meta)
        }
        other => other,
    };
    if unchanged {
        return Ok((kind, Item::Keep));
    }
    let password = password.or(stored_password).ok_or(SecretError::NotSaved)?;
    Ok((kind, new_login_blob(password, totp)?))
}

/// Move a refused input's values into `Zeroizing`, so dropping them wipes
/// them.
fn scrub(secret: VaultSecretInput) {
    match secret {
        VaultSecretInput::Value { value } => drop(value.map(Zeroizing::new)),
        VaultSecretInput::Login {
            fields: LoginInput::Saved { password, totp, .. },
            ..
        } => {
            drop(password.map(Zeroizing::new));
            if let TotpInput::Set { setup } = totp {
                drop(Zeroizing::new(setup));
            }
        }
        VaultSecretInput::Login {
            fields: LoginInput::OnePassword { .. },
            ..
        }
        | VaultSecretInput::OnePassword { .. } => {}
    }
}

/// A kind without an item: remove a Saved login's blob, or retry a removal
/// an earlier change left needing attention.
fn removal(existing: &VaultEntry) -> Item {
    let saved_login = matches!(
        &existing.kind,
        VaultKind::Login(LoginMeta {
            fields: LoginFields::Saved { .. },
            ..
        })
    );
    if saved_login || existing.state == SavedCredentialState::NeedsAttention {
        Item::Remove
    } else {
        Item::Keep
    }
}
