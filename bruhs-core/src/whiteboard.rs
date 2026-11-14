//! Native per-board storage. Neither agents nor the HTTP server own UI board data.
use bruhs_models::whiteboard::*;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::Mutex,
};
mod sources;
mod writer;
pub use sources::source_fingerprint;
pub use writer::{BoardWriter, WriterStatus};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Board(#[from] BoardError),
    #[error("Whiteboard storage failed: {0}. Keep the editor open and export a recovery copy.")]
    Io(#[from] std::io::Error),
    #[error(
        "This board is open for editing in another native window. Open it read-only or save a copy."
    )]
    Busy,
    #[error("Saved whiteboard data is invalid. The original file was preserved: {0}")]
    Corrupt(#[from] serde_json::Error),
    #[error("The board storage lock failed. Reopen this editor; unsaved data remains in memory.")]
    Poisoned,
}
type Result<T> = std::result::Result<T, StoreError>;
#[derive(Clone)]
pub struct BoardStore {
    root: PathBuf,
}
/// Identifies the exact bytes an editor loaded, including external same-revision edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardVersion {
    revision: u64,
    digest: [u8; 32],
}
impl BoardVersion {
    pub fn revision(self) -> u64 {
        self.revision
    }
}
pub struct BoardLease {
    store: BoardStore,
    id: String,
    lock: File,
    digest: Mutex<Option<[u8; 32]>>,
}
impl Drop for BoardLease {
    fn drop(&mut self) {
        if let Err(error) = self.lock.unlock() {
            tracing::error!(%error,"Whiteboard lease could not be released");
        }
    }
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn checked_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| BoardError::Invalid("A native board identity must be a UUID.".into()))?;
    Ok(())
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(BoardError::Invalid(
            "The board file exceeds the local content budget or is not a regular file.".into(),
        )
        .into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(BoardError::Invalid(
            "The board file grew beyond its local content budget.".into(),
        )
        .into());
    }
    Ok(bytes)
}
impl BoardStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        if !fs::symlink_metadata(&root)?.is_dir() {
            return Err(BoardError::Invalid(
                "The board data directory must be a real directory.".into(),
            )
            .into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        fs::create_dir_all(root.join("assets"))?;
        if !fs::symlink_metadata(root.join("assets"))?.is_dir() {
            return Err(BoardError::Invalid(
                "The asset directory must be a real directory.".into(),
            )
            .into());
        }
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        checked_id(id)?;
        Ok(self.root.join(format!("{id}.json")))
    }
    pub fn list(&self, project_path: &str) -> Result<Vec<BoardSummary>> {
        let mut boards = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if uuid::Uuid::parse_str(id).is_err() {
                    continue;
                }
                let board = self.load(id)?;
                if board.project_path == project_path {
                    boards.push(board.summary());
                }
            }
        }
        boards.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then(a.title.cmp(&b.title))
        });
        Ok(boards)
    }
    pub fn load(&self, id: &str) -> Result<BoardDocument> {
        self.snapshot(id).map(|(board, _)| board)
    }
    pub fn snapshot(&self, id: &str) -> Result<(BoardDocument, BoardVersion)> {
        let bytes = read(&self.path(id)?, MAX_BOARD_BYTES)?;
        let board: BoardDocument = serde_json::from_slice(&bytes)?;
        board.validate()?;
        if board.id != id {
            return Err(BoardError::Invalid(
                "The saved board identity does not match its file.".into(),
            )
            .into());
        }
        let version = BoardVersion {
            revision: board.revision,
            digest: digest(&bytes),
        };
        Ok((board, version))
    }
    pub fn lease(&self, id: &str) -> Result<BoardLease> {
        checked_id(id)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(self.root.join(format!("{id}.lock")))?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => StoreError::Busy,
            std::fs::TryLockError::Error(error) => StoreError::Io(error),
        })?;
        let bytes = match read(&self.path(id)?, MAX_BOARD_BYTES) {
            Ok(bytes) => Some(bytes),
            Err(StoreError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                let _ = lock.unlock();
                return Err(error);
            }
        };
        Ok(BoardLease {
            store: self.clone(),
            id: id.into(),
            lock,
            digest: Mutex::new(bytes.as_ref().map(|bytes| digest(bytes))),
        })
    }
    pub fn put_asset(&self, asset: &Asset, payload: &AssetPayload) -> Result<()> {
        let bytes = payload.decode(asset)?;
        let path = self.root.join("assets").join(&asset.sha256);
        if path.try_exists()? {
            if read(&path, bruhs_models::logic::attachments::MAX_IMAGE_BYTES)? != bytes {
                return Err(BoardError::Invalid(
                    "Existing image bytes conflict with their digest.".into(),
                )
                .into());
            }
            return Ok(());
        }
        atomic_write(&path, &bytes)?;
        Ok(())
    }
    pub fn asset(&self, asset: &Asset) -> Result<AssetPayload> {
        if asset.sha256.len() != 64 || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(BoardError::Invalid("The image identity is invalid.".into()).into());
        }
        let bytes = read(
            &self.root.join("assets").join(&asset.sha256),
            bruhs_models::logic::attachments::MAX_IMAGE_BYTES,
        )?;
        let (_, payload) = AssetPayload::from_bytes(&bytes)?;
        payload.decode(asset)?;
        Ok(payload)
    }
    pub fn portable(&self, board: BoardDocument) -> Result<PortableBoard> {
        let mut payloads = std::collections::BTreeMap::new();
        for asset in board.assets.values() {
            payloads.insert(asset.sha256.clone(), self.asset(asset)?);
        }
        Ok(PortableBoard { board, payloads })
    }
}
impl BoardLease {
    pub fn save(
        &self,
        board: &BoardDocument,
        expected: Option<BoardVersion>,
    ) -> Result<BoardVersion> {
        board.validate()?;
        if board.id != self.id {
            return Err(
                BoardError::Invalid("A board lease cannot write another identity.".into()).into(),
            );
        }
        let bytes = serde_json::to_vec(board)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BOARD_BYTES {
            return Err(BoardError::Invalid("This board exceeds the existing local content budget. Export and split it into smaller boards.".into()).into());
        }
        let mut last = self.digest.lock().map_err(|_| StoreError::Poisoned)?;
        let path = self.store.path(&self.id)?;
        let current = match read(&path, MAX_BOARD_BYTES) {
            Ok(bytes) => Some(bytes),
            Err(StoreError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if current.as_ref().map(|bytes| digest(bytes)) != *last {
            return Err(BoardError::Conflict.into());
        }
        match (&current, expected) {
            (None, None) => {}
            (Some(current), Some(version)) => {
                let saved: BoardDocument = serde_json::from_slice(current)?;
                if saved.revision != version.revision
                    || digest(current) != version.digest
                    || saved.project_path != board.project_path
                    || board.revision <= version.revision
                {
                    return Err(BoardError::Conflict.into());
                }
            }
            _ => return Err(BoardError::Conflict.into()),
        }
        for asset in board.assets.values() {
            self.store.asset(asset)?;
        }
        atomic_write(&path, &bytes)?;
        let version = BoardVersion {
            revision: board.revision,
            digest: digest(&bytes),
        };
        *last = Some(version.digest);
        drop(last);
        Ok(version)
    }
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| BoardError::Invalid("The board directory is unavailable.".into()))?;
    let temp = parent.join(format!(".{}.pending", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_cas_external_edits_and_reopen_preserve_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(dir.path().join("boards")).unwrap();
        let mut board = benchmark_scene(1000).unwrap();
        let lease = store.lease(&board.id).unwrap();
        let initial = lease.save(&board, None).unwrap();
        assert!(matches!(store.lease(&board.id), Err(StoreError::Busy)));
        board.revision = 1;
        board.title = "Edited".into();
        let edited = lease.save(&board, Some(initial)).unwrap();
        assert!(lease.save(&board, Some(initial)).is_err());
        let path = store.path(&board.id).unwrap();
        let mut external = board.clone();
        external.title = "External".into();
        fs::write(&path, serde_json::to_vec(&external).unwrap()).unwrap();
        board.revision = 2;
        assert!(lease.save(&board, Some(edited)).is_err());
        assert_eq!(store.load(&board.id).unwrap().title, "External");
        drop(lease);
        let next = store.lease(&board.id).unwrap();
        assert!(
            next.save(&board, Some(edited)).is_err(),
            "a new lease cannot authorize overwriting an unseen same-revision edit"
        );
        let (mut refreshed, version) = store.snapshot(&board.id).unwrap();
        refreshed.revision += 1;
        refreshed.title = "Reviewed edit".into();
        next.save(&refreshed, Some(version)).unwrap();
        assert_eq!(store.list(&board.project_path).unwrap().len(), 1);
    }
    #[test]
    fn invalid_board_does_not_replace_saved_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(dir.path().to_owned()).unwrap();
        let mut board = benchmark_scene(1000).unwrap();
        let lease = store.lease(&board.id).unwrap();
        let version = lease.save(&board, None).unwrap();
        let before = fs::read(store.path(&board.id).unwrap()).unwrap();
        board.revision = 1;
        board.shapes.get_mut("node-00000").unwrap().rect.x = f64::NAN;
        assert!(lease.save(&board, Some(version)).is_err());
        assert_eq!(fs::read(store.path(&board.id).unwrap()).unwrap(), before);
    }
}
