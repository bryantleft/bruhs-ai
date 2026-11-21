//! D26's stand-in for the keychain in debug builds: every item in one
//! JSON file, `<dir>/items.json`, readable only by this account (0600) and
//! replaced atomically under a lock beside it, so several processes (the
//! server, the restore's applier) share one set of items. It holds values
//! in the clear by design, so it lives outside the data roots and only
//! acceptance runs use it.

use std::collections::HashMap;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use zeroize::Zeroizing;

use super::SecretError;

/// The items file's name inside the folder `BRUHS_SECRET_STORE=file:` names.
pub(super) const ITEMS: &str = "items.json";
const LOCK: &str = "items.lock";
/// More than any test run writes.
const ITEMS_MAX: u64 = 16 << 20;

type Items = HashMap<String, Zeroizing<String>>;

/// Every item in `file`; a missing file holds none.
pub(super) fn read(file: &Path) -> Result<Items, SecretError> {
    let bytes = match std::fs::symlink_metadata(file) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= ITEMS_MAX => {
            Zeroizing::new(std::fs::read(file).map_err(|_| SecretError::Storage)?)
        }
        Ok(_) => return Err(SecretError::Storage),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Items::new()),
        Err(_) => return Err(SecretError::Storage),
    };
    let items: HashMap<String, String> =
        serde_json::from_slice(&bytes).map_err(|_| SecretError::Storage)?;
    Ok(items
        .into_iter()
        .map(|(slot, value)| (slot, Zeroizing::new(value)))
        .collect())
}

/// Applies `edit` to the items in `file` and writes them back, holding the
/// lock beside it throughout.
pub(super) fn change(file: &Path, edit: impl FnOnce(&mut Items)) -> Result<(), SecretError> {
    let folder = file.parent().ok_or(SecretError::Storage)?;
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        builder.mode(0o700);
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    builder.create(folder).map_err(|_| SecretError::Storage)?;
    let lock = options
        .open(folder.join(LOCK))
        .map_err(|_| SecretError::Storage)?;
    lock.lock().map_err(|_| SecretError::Storage)?;
    let mut items = read(file)?;
    edit(&mut items);
    let text = Zeroizing::new(
        serde_json::to_vec(
            &items
                .iter()
                .map(|(slot, value)| (slot.as_str(), value.as_str()))
                .collect::<HashMap<_, _>>(),
        )
        .map_err(|_| SecretError::Storage)?,
    );
    // A private temp file (0600), synced, then renamed over the old one.
    let mut temp = tempfile::NamedTempFile::new_in(folder).map_err(|_| SecretError::Storage)?;
    temp.write_all(&text)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|_| SecretError::Storage)?;
    temp.persist(file).map_err(|_| SecretError::Storage)?;
    drop(lock);
    Ok(())
}
