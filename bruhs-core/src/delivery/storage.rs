use super::{DeliveryError, DeliveryResult, Journal};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(super) fn open(root: &Path) -> DeliveryResult<(Journal, File)> {
    std::fs::create_dir_all(root).map_err(storage_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(storage_error)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(root.join("owner.lock"))
        .map_err(storage_error)?;
    lock.try_lock().map_err(|_| DeliveryError::Conflict("Another delivery host owns this store. Connect to that server instead of starting another host.".into()))?;
    let file = root.join("journal.json");
    let journal = match File::open(file) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(storage_error)?;
            serde_json::from_slice::<Journal>(&bytes).map_err(|error| DeliveryError::Storage(format!("The delivery journal is invalid: {error}. Restore a backup; no records were overwritten.")))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Journal::default(),
        Err(error) => return Err(storage_error(error)),
    };
    if journal.version != 1 {
        return Err(DeliveryError::Storage(format!(
            "Journal version {} is unsupported. Use the Bruhs version that wrote it.",
            journal.version
        )));
    }
    Ok((journal, lock))
}

pub(super) fn save(root: &Path, journal: &Journal) -> DeliveryResult<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(root).map_err(storage_error)?;
    serde_json::to_writer(&mut temporary, journal)
        .map_err(|error| DeliveryError::Storage(error.to_string()))?;
    temporary.flush().map_err(storage_error)?;
    temporary.as_file().sync_all().map_err(storage_error)?;
    temporary
        .persist(root.join("journal.json"))
        .map_err(|error| storage_error(error.error))?;
    #[cfg(unix)]
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(storage_error)?;
    Ok(())
}

fn storage_error(error: std::io::Error) -> DeliveryError {
    DeliveryError::Storage(error.to_string())
}
