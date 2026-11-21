//! OS values and a private, value-free catalog. Mutations hold a cross-process lock.
//!
//! `catalog.json` keeps project keys in the format older binaries read;
//! every other vault entry lives in `vault.json` beside it (see [`vault`]).
use super::{items::check_item, storage_key, RekeyError, SecretError, SECRETS_SERVICE};
use bruhs_models::{
    backup::{rewrite_path, PathRewrite},
    secrets::{SavedCredential, SavedCredentialState},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, LockResult, Mutex, MutexGuard, PoisonError},
};
use vault::VaultCatalog;
pub(super) use vault::{scrub, StoredTotp, VaultSecret};
use zeroize::Zeroizing;

mod vault;

/// The keychain account service every Bruhs secret is stored under.
const KEYCHAIN_SERVICE: &str = SECRETS_SERVICE;

/// The legacy catalog, in the format older binaries read.
const CATALOG_FILE: &str = "catalog.json";

type MemoryKeys = Arc<KeyMap>;

/// In-memory keychain items for the memory store and tests, or, for a
/// debug build's acceptance runs, items kept in one file (D26).
#[derive(Default)]
struct KeyMap {
    values: Mutex<HashMap<String, Zeroizing<String>>>,
    /// `<dir>/items.json`: every read and change goes to the file, so other
    /// processes on the same file see the same items.
    #[cfg(any(test, debug_assertions))]
    file: Option<PathBuf>,
    #[cfg(test)]
    hooks: KeyHooks,
}

/// Test-only faults: refused changes and reads that wait like an
/// unanswered keychain prompt.
#[cfg(test)]
#[derive(Default)]
struct KeyHooks {
    /// Writes and removals that succeed before every later one fails.
    changes_left: Mutex<Option<usize>>,
    reads_held: (Mutex<bool>, std::sync::Condvar),
    /// Reads waiting on `reads_held` right now.
    waiting: std::sync::atomic::AtomicUsize,
}

impl KeyMap {
    /// The map key of an item: the account alone for Bruhs' secrets, as
    /// before, and `service\0account` for every other service, so two
    /// services never share a slot (accounts never hold NUL).
    fn slot(service: &str, account: &str) -> String {
        if service == KEYCHAIN_SERVICE {
            account.to_owned()
        } else {
            format!("{service}\u{0}{account}")
        }
    }

    /// Items kept in `file` instead of memory (D26).
    #[cfg(any(test, debug_assertions))]
    fn in_file(file: PathBuf) -> Self {
        Self {
            file: Some(file),
            ..Self::default()
        }
    }

    fn lock(&self) -> LockResult<MutexGuard<'_, HashMap<String, Zeroizing<String>>>> {
        self.values.lock()
    }

    fn read(&self, account: &str) -> Option<Zeroizing<String>> {
        #[cfg(test)]
        {
            use std::sync::atomic::Ordering;
            let (held, open) = &self.hooks.reads_held;
            self.hooks.waiting.fetch_add(1, Ordering::SeqCst);
            drop(
                open.wait_while(
                    held.lock().unwrap_or_else(PoisonError::into_inner),
                    |held| *held,
                )
                .unwrap_or_else(PoisonError::into_inner),
            );
            self.hooks.waiting.fetch_sub(1, Ordering::SeqCst);
        }
        #[cfg(any(test, debug_assertions))]
        if let Some(file) = &self.file {
            // An unreadable file reads as no item, as a keychain without it.
            return item_file::read(file).ok()?.remove(account);
        }
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .cloned()
    }

    fn write(&self, account: &str, value: &str) -> Result<(), SecretError> {
        #[cfg(test)]
        self.change()?;
        #[cfg(any(test, debug_assertions))]
        if let Some(file) = &self.file {
            return item_file::change(file, |items| {
                items.insert(account.to_owned(), Zeroizing::new(value.to_owned()));
            });
        }
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account.to_owned(), Zeroizing::new(value.to_owned()));
        Ok(())
    }

    fn remove(&self, account: &str) -> Result<(), SecretError> {
        #[cfg(test)]
        self.change()?;
        #[cfg(any(test, debug_assertions))]
        if let Some(file) = &self.file {
            return item_file::change(file, |items| {
                items.remove(account);
            });
        }
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account);
        Ok(())
    }

    /// Whether a test still lets this write or removal through.
    #[cfg(test)]
    fn change(&self) -> Result<(), SecretError> {
        if let Some(left) = self
            .hooks
            .changes_left
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            if *left == 0 {
                return Err(SecretError::Storage);
            }
            *left -= 1;
        }
        Ok(())
    }

    /// Let `count` more writes and removals succeed, then refuse every
    /// later one; `None` lets every one through again.
    #[cfg(test)]
    fn refuse_writes_after(&self, count: Option<usize>) {
        *self
            .hooks
            .changes_left
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = count;
    }

    /// How many reads are waiting on [`Self::hold_reads`].
    #[cfg(test)]
    fn held_reads(&self) -> usize {
        self.hooks.waiting.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// While held, every read waits, like a keychain prompt nobody answers.
    #[cfg(test)]
    fn hold_reads(&self, hold: bool) {
        let (held, open) = &self.hooks.reads_held;
        *held.lock().unwrap_or_else(PoisonError::into_inner) = hold;
        open.notify_all();
    }
}

#[derive(Clone)]
pub struct SecretStore(Backend);

#[derive(Clone)]
enum Backend {
    Os,
    Memory(Arc<Mutex<Vault>>),
    #[cfg(test)]
    Unavailable,
    /// Catalog files in a folder, keychain items in memory or, for D26's
    /// [`SecretStore::test_file`], in a file.
    #[cfg(any(test, debug_assertions))]
    FileTest(PathBuf, MemoryKeys),
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    entries: BTreeMap<String, SavedCredential>,
}

enum Keys {
    Os,
    Memory(MemoryKeys),
    #[cfg(test)]
    Unavailable,
}

struct Vault {
    catalog: Catalog,
    /// `vault.json`, read on first use, so a damaged or newer file stops
    /// vault operations only, never legacy thread saves and reads.
    vault: Option<VaultCatalog>,
    keys: Keys,
    directory: Option<PathBuf>,
}

impl Keys {
    /// Another handle on the same items, for a read made without the lock.
    fn handle(&self) -> Self {
        match self {
            Self::Os => Self::Os,
            Self::Memory(keys) => Self::Memory(keys.clone()),
            #[cfg(test)]
            Self::Unavailable => Self::Unavailable,
        }
    }

    fn read(&self, account: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        self.read_in(KEYCHAIN_SERVICE, account)
    }

    fn write(&self, account: &str, value: &str) -> Result<(), SecretError> {
        self.write_in(KEYCHAIN_SERVICE, account, value)
    }

    /// Remove an item. A missing one is already removed.
    fn remove(&self, account: &str) -> Result<(), SecretError> {
        self.remove_in(KEYCHAIN_SERVICE, account)
    }

    fn read_in(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<String>>, SecretError> {
        match self {
            #[cfg(test)]
            Self::Unavailable => Err(SecretError::Storage),
            Self::Memory(keys) => Ok(keys.read(&KeyMap::slot(service, account))),
            Self::Os => match keyring::Entry::new(service, account)
                .map_err(|_| SecretError::Storage)?
                .get_password()
            {
                Ok(value) => Ok(Some(Zeroizing::new(value))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(SecretError::Storage),
            },
        }
    }

    fn write_in(&self, service: &str, account: &str, value: &str) -> Result<(), SecretError> {
        match self {
            #[cfg(test)]
            Self::Unavailable => Err(SecretError::Storage),
            Self::Memory(keys) => keys.write(&KeyMap::slot(service, account), value),
            Self::Os => keyring::Entry::new(service, account)
                .and_then(|entry| entry.set_password(value))
                .map_err(|_| SecretError::Storage),
        }
    }

    fn remove_in(&self, service: &str, account: &str) -> Result<(), SecretError> {
        match self {
            #[cfg(test)]
            Self::Unavailable => Err(SecretError::Storage),
            Self::Memory(keys) => keys.remove(&KeyMap::slot(service, account)),
            Self::Os => match keyring::Entry::new(service, account)
                .map_err(|_| SecretError::Storage)?
                .delete_credential()
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(SecretError::Storage),
            },
        }
    }
}

impl SecretStore {
    pub fn os() -> Self {
        Self(Backend::Os)
    }

    /// Isolated test store. Never accesses the user's credentials or app files.
    pub fn in_memory() -> Self {
        Self(Backend::Memory(Arc::new(Mutex::new(Vault {
            catalog: Catalog::default(),
            vault: Some(VaultCatalog::default()),
            keys: Keys::Memory(Arc::default()),
            directory: None,
        }))))
    }

    /// Whether this store keeps nothing on disk.
    pub(crate) fn is_memory(&self) -> bool {
        matches!(self.0, Backend::Memory(_))
    }

    /// While the returned guard lives, every read and change of an
    /// in-memory store waits, like a catalog that doesn't answer. `None`
    /// for any other store.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn hold_for_tests(&self) -> Option<impl Sized + '_> {
        match &self.0 {
            Backend::Memory(vault) => Some(vault.lock().unwrap_or_else(PoisonError::into_inner)),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(super) fn unavailable() -> Self {
        Self(Backend::Unavailable)
    }

    /// Catalog files in `directory` and keychain items in memory, for
    /// tests that read or write the files a backup carries.
    #[cfg(test)]
    pub(crate) fn file_test(directory: PathBuf) -> Self {
        Self(Backend::FileTest(directory, MemoryKeys::default()))
    }

    /// D26, debug builds only: the usual catalog files, with keychain items
    /// in `<directory>/items.json` (0600, rewritten atomically) instead of
    /// the keychain, so an acceptance run's items outlive the server and
    /// reach the restore's own process. `directory` belongs outside the
    /// data roots. For a scratch `HOME` only: the catalogs stay in this
    /// account's real credentials folder, so under a real `HOME` they
    /// would name items the file doesn't hold.
    #[cfg(any(test, debug_assertions))]
    pub fn test_file(directory: PathBuf) -> Self {
        // Without a data folder nothing else runs either; the catalogs
        // then sit beside the items.
        let catalogs = catalog_directory().unwrap_or_else(|_| directory.join("credentials"));
        Self(Backend::FileTest(
            catalogs,
            Arc::new(KeyMap::in_file(directory.join(item_file::ITEMS))),
        ))
    }

    /// One of Bruhs' own keychain items ([`super::ITEM_SERVICES`]), read
    /// without the catalog lock. It may wait on a keychain prompt, so call
    /// it off the async runtime.
    pub fn read_item(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<String>>, SecretError> {
        check_item(service, account)?;
        self.key_handle()?.read_in(service, account)
    }

    /// Write one of Bruhs' own keychain items. Blocks like [`Self::read_item`].
    pub fn write_item(&self, service: &str, account: &str, value: &str) -> Result<(), SecretError> {
        check_item(service, account)?;
        self.key_handle()?.write_in(service, account, value)
    }

    /// Remove one of Bruhs' own keychain items; a missing one is fine.
    pub fn remove_item(&self, service: &str, account: &str) -> Result<(), SecretError> {
        check_item(service, account)?;
        self.key_handle()?.remove_in(service, account)
    }

    async fn access<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Vault) -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        let backend = self.0.clone();
        tokio::task::spawn_blocking(move || match backend {
            Backend::Memory(vault) => {
                operation(&mut vault.lock().unwrap_or_else(PoisonError::into_inner))
            }
            Backend::Os => {
                let directory = catalog_directory()?;
                with_catalog(&directory, Keys::Os, operation)
            }
            #[cfg(test)]
            Backend::Unavailable => Err(SecretError::Storage),
            #[cfg(any(test, debug_assertions))]
            Backend::FileTest(directory, keys) => {
                with_catalog(&directory, Keys::Memory(keys), operation)
            }
        })
        .await
        .map_err(|_| SecretError::Catalog)?
    }

    /// Listing touches metadata only; it does not read or unlock keychain values.
    pub async fn list(&self) -> Result<Vec<SavedCredential>, SecretError> {
        let backend = self.0.clone();
        tokio::task::spawn_blocking(move || {
            // Atomic renames provide a coherent snapshot even while a keychain prompt holds the writer lock.
            let mut entries: Vec<_> = match backend {
                Backend::Os => read_catalog(&catalog_directory()?)?
                    .entries
                    .into_values()
                    .collect(),
                Backend::Memory(vault) => vault
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .catalog
                    .entries
                    .values()
                    .cloned()
                    .collect(),
                #[cfg(any(test, debug_assertions))]
                Backend::FileTest(directory, _) => {
                    read_catalog(&directory)?.entries.into_values().collect()
                }
                #[cfg(test)]
                Backend::Unavailable => return Err(SecretError::Storage),
            };
            entries.sort_by(|a, b| (&a.project_path, &a.name).cmp(&(&b.project_path, &b.name)));
            Ok(entries)
        })
        .await
        .map_err(|_| SecretError::Catalog)?
    }

    /// A legacy value by id. Reads the keychain without the catalog lock,
    /// so an unanswered keychain prompt never stops a save elsewhere.
    pub(super) async fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        let key = key.to_owned();
        let keys = self.keys().await?;
        tokio::task::spawn_blocking(move || keys.read(&key))
            .await
            .map_err(|_| SecretError::Catalog)?
    }

    /// A handle on this store's keychain items, taken without the catalog lock.
    async fn keys(&self) -> Result<Keys, SecretError> {
        self.key_handle()
    }

    fn key_handle(&self) -> Result<Keys, SecretError> {
        Ok(match &self.0 {
            Backend::Os => Keys::Os,
            Backend::Memory(vault) => vault
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .keys
                .handle(),
            #[cfg(test)]
            Backend::Unavailable => return Err(SecretError::Storage),
            #[cfg(any(test, debug_assertions))]
            Backend::FileTest(_, keys) => Keys::Memory(keys.clone()),
        })
    }

    /// A project's value by name. Registers a value saved by the first,
    /// unindexed implementation. The keychain is read without the lock;
    /// the lock is taken only to add a missing catalog row.
    pub(super) async fn find(
        &self,
        project: &str,
        name: &str,
    ) -> Result<Option<Zeroizing<String>>, SecretError> {
        let key = storage_key(Path::new(project), name);
        let value = self.get(&key).await?;
        if value.is_none() || self.list().await?.iter().any(|entry| entry.id == key) {
            return Ok(value);
        }
        let (project, name) = (project.to_owned(), name.to_owned());
        self.access(move |vault| {
            if !vault.catalog.entries.contains_key(&key) {
                vault.catalog.entries.insert(
                    key.clone(),
                    SavedCredential {
                        id: key,
                        name,
                        project_path: project,
                        state: SavedCredentialState::Saved,
                    },
                );
                vault.persist()?;
            }
            Ok(())
        })
        .await?;
        Ok(value)
    }

    /// Save an approved project credential. Only names and project paths enter the catalog.
    pub async fn save(
        &self,
        project: &str,
        name: &str,
        value: Zeroizing<String>,
    ) -> Result<(), SecretError> {
        if value.is_empty() || value.contains('\0') {
            return Err(SecretError::Invalid(
                "Enter a nonempty credential without NUL characters.",
            ));
        }
        let (project, name) = (project.to_owned(), name.to_owned());
        self.access(move |vault| {
            // A project's 1Password entry of that name answers the thread
            // already; a saved key beside it would break unique names (D6).
            if vault.has_project_entry(&project, &name) {
                return Err(SecretError::Invalid(bruhs_models::vault::DUPLICATE_NAME));
            }
            let id = storage_key(Path::new(&project), &name);
            vault.save(
                SavedCredential {
                    id,
                    name,
                    project_path: project,
                    state: SavedCredentialState::NeedsAttention,
                },
                &value,
            )
        })
        .await
    }

    /// A write-only update. The stored value is never returned to the caller.
    pub async fn replace(&self, id: &str, value: String) -> Result<(), SecretError> {
        let (id, value) = (id.to_owned(), Zeroizing::new(value));
        if value.is_empty() || value.contains('\0') {
            return Err(SecretError::Invalid(
                "Enter a nonempty credential without NUL characters.",
            ));
        }
        self.access(move |vault| {
            let entry = vault
                .catalog
                .entries
                .get(&id)
                .cloned()
                .ok_or(SecretError::NotSaved)?;
            vault.save(entry, &value)
        })
        .await
    }

    /// Delete the saved copy. Already-running commands keep their own process environment.
    pub async fn delete_saved(&self, id: &str) -> Result<(), SecretError> {
        let id = id.to_owned();
        self.access(move |vault| vault.delete_saved(&id)).await
    }
}

/// Hooks for the manager's tests, on a memory or file test store.
#[cfg(test)]
impl SecretStore {
    fn memory_keys(&self) -> MemoryKeys {
        match &self.0 {
            Backend::Memory(vault) => {
                match &vault.lock().unwrap_or_else(PoisonError::into_inner).keys {
                    Keys::Memory(keys) => keys.clone(),
                    _ => unreachable!("memory stores keep memory keys"),
                }
            }
            Backend::FileTest(_, keys) => keys.clone(),
            _ => unreachable!("tests use memory keys"),
        }
    }

    /// While held, every keychain read waits, like an unanswered prompt.
    pub(crate) fn hold_reads(&self, hold: bool) {
        self.memory_keys().hold_reads(hold);
    }

    /// How many keychain reads are waiting on [`Self::hold_reads`].
    pub(super) fn held_reads(&self) -> usize {
        self.memory_keys().held_reads()
    }

    /// Remove a keychain item behind the catalog's back.
    pub(super) fn drop_item(&self, account: &str) {
        self.memory_keys()
            .remove(account)
            .expect("test stores accept removals");
    }

    /// Write a keychain item as is, to stand in for a damaged one.
    pub(super) fn put_item(&self, account: &str, text: &str) {
        self.memory_keys()
            .write(account, text)
            .expect("test stores accept writes");
    }
}

impl Vault {
    fn delete_saved(&mut self, id: &str) -> Result<(), SecretError> {
        let Some(entry) = self.catalog.entries.get_mut(id) else {
            return Err(SecretError::NotSaved);
        };
        entry.state = SavedCredentialState::NeedsAttention;
        self.persist()?;
        self.keys.remove(id)?;
        self.catalog.entries.remove(id);
        self.persist()
    }

    fn save(
        &mut self,
        mut entry: SavedCredential,
        value: &Zeroizing<String>,
    ) -> Result<(), SecretError> {
        // Durable intent first: a crash/failure cannot leave a newly saved key unlisted.
        entry.state = SavedCredentialState::NeedsAttention;
        let id = entry.id.clone();
        self.catalog.entries.insert(id.clone(), entry);
        self.persist()?;
        self.keys.write(&id, value)?;
        if let Some(entry) = self.catalog.entries.get_mut(&id) {
            entry.state = SavedCredentialState::Saved;
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), SecretError> {
        match &self.directory {
            Some(directory) => write_atomically(directory, CATALOG_FILE, &self.catalog),
            None => Ok(()),
        }
    }
}

/// Replace `directory/name` with `value`'s JSON: a private temp file,
/// fsync, rename, then a directory fsync. Readers see the old or the new
/// file, never a partial one.
fn write_atomically(
    directory: &Path,
    name: &str,
    value: &impl Serialize,
) -> Result<(), SecretError> {
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| SecretError::Catalog)?;
    serde_json::to_writer(&mut file, value).map_err(|_| SecretError::Catalog)?;
    file.flush()
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| SecretError::Catalog)?;
    file.persist(directory.join(name))
        .map_err(|_| SecretError::Catalog)?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| SecretError::Catalog)?;
    Ok(())
}

fn with_catalog<T>(
    directory: &Path,
    keys: Keys,
    operation: impl FnOnce(&mut Vault) -> Result<T, SecretError>,
) -> Result<T, SecretError> {
    fs::create_dir_all(directory).map_err(|_| SecretError::Catalog)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| SecretError::Catalog)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options
        .open(directory.join("catalog.lock"))
        .map_err(|_| SecretError::Catalog)?;
    lock.lock().map_err(|_| SecretError::Catalog)?;
    let catalog = read_catalog(directory)?;
    let result = operation(&mut Vault {
        catalog,
        vault: None,
        keys,
        directory: Some(directory.into()),
    });
    drop(lock);
    result
}

/// Every keychain account under `bruhs-ai-secrets` the catalogs in
/// `directory` name: legacy ids, and `vault.json` rows with an item, a
/// newer Bruhs' rows included. Reads both files without the lock, as
/// [`SecretStore::list`] does; a missing file names nothing.
pub(super) fn keychain_accounts(directory: &Path) -> Result<Vec<String>, SecretError> {
    let mut accounts: Vec<String> = read_catalog(directory)?.entries.into_keys().collect();
    accounts.extend(vault::item_accounts(directory)?);
    Ok(accounts)
}

/// Move the staged catalogs in `directory` to this computer's paths
/// (D12): each legacy entry's project path and id, and each `vault.json`
/// Project scope. Answers each moved keychain account, old to new.
/// Staged files have no other reader, so this takes no lock; it reads
/// both files before it writes either. Two entries landing on one id is
/// `Catalog`, with nothing written.
pub(super) fn rekey(
    directory: &Path,
    rules: &[PathRewrite],
) -> Result<BTreeMap<String, String>, RekeyError> {
    // A disk that fails a read is told apart from a catalog that breaks
    // its rules, which the parse below can't do (B Task 9 N2).
    for name in [CATALOG_FILE, vault::VAULT_FILE] {
        match fs::read(directory.join(name)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(RekeyError::Read),
        }
    }
    let mut moved = BTreeMap::new();
    let mut catalog = Catalog::default();
    for (id, mut entry) in read_catalog(directory)
        .map_err(|_| RekeyError::Unreadable)?
        .entries
    {
        if let Some(path) = rewrite_path(&entry.project_path, rules) {
            entry.id = storage_key(Path::new(&path), &entry.name);
            entry.project_path = path;
        }
        if entry.id != id {
            moved.insert(id, entry.id.clone());
        }
        if catalog.entries.insert(entry.id.clone(), entry).is_some() {
            return Err(RekeyError::Clash);
        }
    }
    let legacy_moved = !moved.is_empty();
    let vault = vault::rekeyed(directory, rules, &mut moved).map_err(|_| RekeyError::Unreadable)?;
    if legacy_moved {
        write_atomically(directory, CATALOG_FILE, &catalog).map_err(|_| RekeyError::Write)?;
    }
    if let Some(vault) = vault {
        vault::write_vault(directory, &vault).map_err(|_| RekeyError::Write)?;
    }
    Ok(moved)
}

pub(super) fn catalog_directory() -> Result<PathBuf, SecretError> {
    Ok(directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .ok_or(SecretError::Catalog)?
        .data_local_dir()
        .join("credentials"))
}

fn read_catalog(directory: &Path) -> Result<Catalog, SecretError> {
    let catalog: Catalog = match File::open(directory.join(CATALOG_FILE)) {
        Ok(file) => {
            serde_json::from_reader(BufReader::new(file)).map_err(|_| SecretError::Catalog)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Catalog::default(),
        Err(_) => return Err(SecretError::Catalog),
    };
    for (id, entry) in &catalog.entries {
        if id != &entry.id || *id != storage_key(Path::new(&entry.project_path), &entry.name) {
            return Err(SecretError::Catalog);
        }
    }
    Ok(catalog)
}

#[cfg(any(test, debug_assertions))]
mod item_file;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vault_tests;
