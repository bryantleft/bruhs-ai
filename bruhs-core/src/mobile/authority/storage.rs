use super::{Catalog, MobileError, MobileResult};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
/// Same wire-payload budget as the workspace bridge's largest response.
const CATALOG_BYTES: usize = 1024 * 1024;
pub(super) fn open(path: &Path) -> MobileResult<(Catalog, File)> {
    let directory = path.parent().ok_or(MobileError::Storage)?;
    std::fs::create_dir_all(directory).map_err(|_| MobileError::Storage)?;
    if std::fs::symlink_metadata(directory)
        .map_err(|_| MobileError::Storage)?
        .file_type()
        .is_symlink()
    {
        return Err(MobileError::Storage);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| MobileError::Storage)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let owner = options
        .open(directory.join("owner.lock"))
        .map_err(|_| MobileError::Storage)?;
    if !owner
        .metadata()
        .map_err(|_| MobileError::Storage)?
        .is_file()
    {
        return Err(MobileError::Storage);
    }
    owner.try_lock().map_err(|_| {
        MobileError::Conflict(
            "Another desktop owns mobile access. Stop it before starting this session.".into(),
        )
    })?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let catalog = match options.open(path) {
        Ok(mut file) => {
            if !file.metadata().map_err(|_| MobileError::Storage)?.is_file() {
                return Err(MobileError::Storage);
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(u64::try_from(CATALOG_BYTES).unwrap_or(u64::MAX) + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| MobileError::Storage)?;
            if bytes.len() > CATALOG_BYTES {
                return Err(MobileError::Storage);
            }
            serde_json::from_slice::<Catalog>(&bytes).map_err(|_| MobileError::Storage)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Catalog::default(),
        Err(_) => return Err(MobileError::Storage),
    };
    if catalog.devices.len() > bruhs_models::mobile::PAGE_SIZE {
        return Err(MobileError::Storage);
    }
    if catalog.version != 1 {
        return Err(MobileError::Invalid(
            "Unsupported mobile device catalog version. Use the desktop version that wrote it."
                .into(),
        ));
    }
    Ok((catalog, owner))
}
pub(super) fn save(path: &Path, catalog: &Catalog) -> MobileResult<()> {
    #[cfg(test)]
    pause_write(path);
    let directory = path.parent().ok_or(MobileError::Storage)?;
    if std::fs::symlink_metadata(directory)
        .map_err(|_| MobileError::Storage)?
        .file_type()
        .is_symlink()
    {
        return Err(MobileError::Storage);
    }
    if std::fs::symlink_metadata(path)
        .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
    {
        return Err(MobileError::Storage);
    }
    let bytes = serde_json::to_vec(catalog).map_err(|_| MobileError::Storage)?;
    if bytes.len() > CATALOG_BYTES {
        return Err(MobileError::Storage);
    }
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| MobileError::Storage)?;
    file.write_all(&bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| MobileError::Storage)?;
    file.persist(path).map_err(|_| MobileError::Storage)?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| MobileError::Storage)?;
    Ok(())
}

#[cfg(test)]
type WritePause = (
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);
#[cfg(test)]
fn pauses() -> &'static std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, WritePause>>
{
    static PAUSES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, WritePause>>,
    > = std::sync::OnceLock::new();
    PAUSES.get_or_init(Default::default)
}
#[cfg(test)]
pub(super) fn pause_next(
    path: &Path,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (entered, wait) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    pauses()
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), (entered, paused));
    (wait, resume)
}
#[cfg(test)]
fn pause_write(path: &Path) {
    let pause = pauses().lock().unwrap().remove(path);
    if let Some((entered, resume)) = pause {
        let _ = entered.send(());
        let _ = resume.recv();
    }
}
