//! OS values and a private, value-free catalog. Mutations hold a cross-process lock.
use super::{storage_key, SecretError};
use bruhs_models::secrets::{SavedCredential, SavedCredentialState};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};
use zeroize::Zeroizing;

type MemoryKeys = Arc<Mutex<HashMap<String, Zeroizing<String>>>>;

#[derive(Clone)]
pub struct SecretStore(Backend);

#[derive(Clone)]
enum Backend {
    Os,
    Memory(Arc<Mutex<Vault>>),
    #[cfg(test)]
    Unavailable,
    #[cfg(test)]
    FileTest(PathBuf, MemoryKeys),
}

#[derive(Default, Serialize, Deserialize)]
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
    keys: Keys,
    directory: Option<PathBuf>,
}

impl SecretStore {
    pub fn os() -> Self {
        Self(Backend::Os)
    }

    /// Isolated test store. Never accesses the user's credentials or app files.
    pub fn in_memory() -> Self {
        Self(Backend::Memory(Arc::new(Mutex::new(Vault {
            catalog: Catalog::default(),
            keys: Keys::Memory(Arc::default()),
            directory: None,
        }))))
    }

    #[cfg(test)]
    pub(super) fn unavailable() -> Self {
        Self(Backend::Unavailable)
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
            #[cfg(test)]
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
                #[cfg(test)]
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

    pub(super) async fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        let key = key.to_owned();
        self.access(move |vault| vault.get(&key)).await
    }

    /// Register a value saved by the first, unindexed implementation when its name is known.
    pub(super) async fn find(
        &self,
        project: &str,
        name: &str,
    ) -> Result<Option<Zeroizing<String>>, SecretError> {
        let (project, name) = (project.to_owned(), name.to_owned());
        self.access(move |vault| {
            let key = storage_key(Path::new(&project), &name);
            let value = vault.get(&key)?;
            if value.is_some() && !vault.catalog.entries.contains_key(&key) {
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
            Ok(value)
        })
        .await
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
        self.access(move |vault| {
            if !vault.catalog.entries.contains_key(&id) {
                return Err(SecretError::NotSaved);
            }
            if let Some(entry) = vault.catalog.entries.get_mut(&id) {
                entry.state = SavedCredentialState::NeedsAttention;
            }
            vault.persist()?;
            match &vault.keys {
                #[cfg(test)]
                Keys::Unavailable => return Err(SecretError::Storage),
                Keys::Memory(keys) => {
                    keys.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                }
                Keys::Os => match keyring::Entry::new("bruhs-ai-secrets", &id)
                    .map_err(|_| SecretError::Storage)?
                    .delete_credential()
                {
                    Ok(()) | Err(keyring::Error::NoEntry) => {}
                    Err(_) => return Err(SecretError::Storage),
                },
            }
            vault.catalog.entries.remove(&id);
            vault.persist()
        })
        .await
    }
}

impl Vault {
    fn get(&self, key: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        match &self.keys {
            #[cfg(test)]
            Keys::Unavailable => Err(SecretError::Storage),
            Keys::Memory(values) => Ok(values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .cloned()),
            Keys::Os => match keyring::Entry::new("bruhs-ai-secrets", key)
                .map_err(|_| SecretError::Storage)?
                .get_password()
            {
                Ok(value) => Ok(Some(Zeroizing::new(value))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(SecretError::Storage),
            },
        }
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
        match &self.keys {
            #[cfg(test)]
            Keys::Unavailable => return Err(SecretError::Storage),
            Keys::Memory(keys) => {
                keys.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(id.clone(), value.clone());
            }
            Keys::Os => keyring::Entry::new("bruhs-ai-secrets", &id)
                .and_then(|entry| entry.set_password(value))
                .map_err(|_| SecretError::Storage)?,
        }
        if let Some(entry) = self.catalog.entries.get_mut(&id) {
            entry.state = SavedCredentialState::Saved;
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), SecretError> {
        let Some(directory) = &self.directory else {
            return Ok(());
        };
        let mut file =
            tempfile::NamedTempFile::new_in(directory).map_err(|_| SecretError::Catalog)?;
        serde_json::to_writer(&mut file, &self.catalog).map_err(|_| SecretError::Catalog)?;
        file.flush()
            .and_then(|()| file.as_file().sync_all())
            .map_err(|_| SecretError::Catalog)?;
        file.persist(directory.join("catalog.json"))
            .map_err(|_| SecretError::Catalog)?;
        #[cfg(unix)]
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| SecretError::Catalog)?;
        Ok(())
    }
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
        keys,
        directory: Some(directory.into()),
    });
    drop(lock);
    result
}

fn catalog_directory() -> Result<PathBuf, SecretError> {
    Ok(directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .ok_or(SecretError::Catalog)?
        .data_local_dir()
        .join("credentials"))
}

fn read_catalog(directory: &Path) -> Result<Catalog, SecretError> {
    let catalog: Catalog = match File::open(directory.join("catalog.json")) {
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

#[cfg(test)]
mod tests;
