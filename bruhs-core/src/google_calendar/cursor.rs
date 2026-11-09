//! On-disk calendar state: one sync cursor per calendar under
//! `<dir>/cursors/<hex(id)>.json` and the user's calendar list (with their
//! sync toggles) in `<dir>/calendars.json`.
//!
//! [`CalendarFiles`] owns the directory. Every operation runs on tokio's
//! blocking pool, and all of them are serialized by one async mutex so a
//! read-modify-write of `calendars.json` cannot interleave with another
//! write. Files are replaced atomically through a uniquely named temp file.
//!
//! A `calendars.json` that cannot be read or parsed is an error, never an
//! empty list: treating it as empty would let the next save overwrite the
//! user's sync toggles.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bruhs_models::Calendar;

use super::paths::default_google_calendar_dir;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SyncCursor {
    pub sync_token: Option<String>,
    pub last_synced_at_ms: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CursorError {
    #[error("calendar state directory could not be resolved (no home directory)")]
    NoDataDir,
    #[error("calendar state io ({}): {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("calendar state file is corrupt ({}): {source}", path.display())]
    Corrupt {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("calendar state could not be encoded: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("calendar state task did not finish: {0}")]
    Join(#[from] tokio::task::JoinError),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> CursorError + '_ {
    move |source| CursorError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Lossless, case-insensitive-filesystem-safe file stem for a calendar id:
/// lowercase hex of its UTF-8 bytes. Distinct ids always get distinct names.
fn encoded_calendar_id(id: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(id.len() * 2);
    for b in id.bytes() {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    out
}

/// The pre-hex file stem. Lossy (`a.b` and `a_b` collide); only read, to
/// migrate cursors written by older builds.
fn legacy_calendar_id(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

const CALENDARS_FILENAME: &str = "calendars.json";
const CURSORS_DIR: &str = "cursors";

/// Read and parse `path`; `Ok(None)` when it does not exist.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, CursorError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(path)(e)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| CursorError::Corrupt {
            path: path.to_path_buf(),
            source,
        })
}

/// Write `value` to `path` through a uniquely named temp file + rename, so
/// concurrent writers never share a temp file and readers never see a
/// partial file.
fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), CursorError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(CursorError::Encode)?;
    let dir = path.parent().ok_or(CursorError::NoDataDir)?;
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        // Best effort: the temp file may not exist; the write error is
        // what the caller needs to see.
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(&tmp)(e));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Same as above: report the rename failure, not the cleanup.
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(path)(e));
    }
    Ok(())
}

/// Synchronous file operations on one state directory.
#[derive(Debug)]
struct Dir(PathBuf);

impl Dir {
    fn cursor_path(&self, calendar_id: &str) -> PathBuf {
        self.0
            .join(CURSORS_DIR)
            .join(format!("{}.json", encoded_calendar_id(calendar_id)))
    }

    fn legacy_cursor_path(&self, calendar_id: &str) -> PathBuf {
        self.0
            .join(CURSORS_DIR)
            .join(format!("{}.json", legacy_calendar_id(calendar_id)))
    }

    fn calendars_path(&self) -> PathBuf {
        self.0.join(CALENDARS_FILENAME)
    }

    fn load_cursor(&self, calendar_id: &str) -> Result<SyncCursor, CursorError> {
        if let Some(cursor) = read_json(&self.cursor_path(calendar_id))? {
            return Ok(cursor);
        }
        Ok(read_json(&self.legacy_cursor_path(calendar_id))?.unwrap_or_default())
    }

    fn save_cursor(&self, calendar_id: &str, cursor: &SyncCursor) -> Result<(), CursorError> {
        write_json(&self.cursor_path(calendar_id), cursor)
    }

    fn load_calendars(&self) -> Result<Vec<Calendar>, CursorError> {
        Ok(read_json(&self.calendars_path())?.unwrap_or_default())
    }

    fn save_calendars(&self, list: &[Calendar]) -> Result<(), CursorError> {
        write_json(&self.calendars_path(), list)
    }
}

/// Handle to the calendar state directory. Cheap to clone; clones share the
/// lock.
#[derive(Debug, Clone)]
pub struct CalendarFiles {
    dir: Option<Arc<Dir>>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl CalendarFiles {
    /// The per-user default (`<config>/calendar/google`). Resolving it does
    /// no I/O; the directory is created on first write.
    pub fn default_location() -> Self {
        Self {
            dir: default_google_calendar_dir().map(|d| Arc::new(Dir(d))),
            lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// State kept under `dir` (tests point this at a temp directory).
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: Some(Arc::new(Dir(dir.into()))),
            lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Run `op` on the blocking pool while holding the state lock.
    async fn run<T, F>(&self, op: F) -> Result<T, CursorError>
    where
        T: Send + 'static,
        F: FnOnce(&Dir) -> Result<T, CursorError> + Send + 'static,
    {
        let dir = self.dir.clone().ok_or(CursorError::NoDataDir)?;
        let _guard = self.lock.lock().await;
        tokio::task::spawn_blocking(move || op(&dir)).await?
    }

    pub async fn load_cursor(&self, calendar_id: &str) -> Result<SyncCursor, CursorError> {
        let id = calendar_id.to_string();
        self.run(move |dir| dir.load_cursor(&id)).await
    }

    pub async fn save_cursor(
        &self,
        calendar_id: &str,
        cursor: SyncCursor,
    ) -> Result<(), CursorError> {
        let id = calendar_id.to_string();
        self.run(move |dir| dir.save_cursor(&id, &cursor)).await
    }

    pub async fn load_calendars(&self) -> Result<Vec<Calendar>, CursorError> {
        self.run(|dir| dir.load_calendars()).await
    }

    /// Replace the calendar list with `merge(existing)` in one locked step.
    /// A corrupt existing file aborts before anything is written.
    pub async fn merge_calendars<F>(&self, merge: F) -> Result<Vec<Calendar>, CursorError>
    where
        F: FnOnce(Vec<Calendar>) -> Vec<Calendar> + Send + 'static,
    {
        self.run(move |dir| {
            let next = merge(dir.load_calendars()?);
            dir.save_calendars(&next)?;
            Ok(next)
        })
        .await
    }

    /// Set one calendar's `sync_enabled`. Returns the new full list, or
    /// `None` (and writes nothing) when the id is unknown.
    pub async fn set_sync_enabled(
        &self,
        id: &str,
        sync_enabled: bool,
    ) -> Result<Option<Vec<Calendar>>, CursorError> {
        let id = id.to_string();
        self.run(move |dir| {
            let mut list = dir.load_calendars()?;
            let Some(slot) = list.iter_mut().find(|c| c.id == id) else {
                return Ok(None);
            };
            slot.sync_enabled = sync_enabled;
            dir.save_calendars(&list)?;
            Ok(Some(list))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::EventSource;

    fn calendar(id: &str, sync_enabled: bool) -> Calendar {
        Calendar {
            id: id.into(),
            source: EventSource::Google,
            provider_id: Some(format!("{id}@group.calendar.google.com")),
            name: id.into(),
            color: None,
            sync_enabled,
            primary: false,
            last_synced_at: None,
        }
    }

    #[test]
    fn encoded_ids_are_lossless_and_case_safe() {
        assert_eq!(encoded_calendar_id("primary"), "7072696d617279");
        // Collided under the old scheme ("a_b" for both).
        assert_ne!(encoded_calendar_id("a.b"), encoded_calendar_id("a_b"));
        // Would collide on a case-insensitive filesystem if kept verbatim.
        assert_ne!(
            encoded_calendar_id("Primary").to_lowercase(),
            encoded_calendar_id("primary").to_lowercase()
        );
        assert!(encoded_calendar_id("x/../y")
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn missing_cursor_is_default() {
        let tmp = tempfile::tempdir().unwrap();
        let files = CalendarFiles::at(tmp.path());
        assert_eq!(
            files.load_cursor("primary").await.unwrap(),
            SyncCursor::default()
        );
    }

    #[tokio::test]
    async fn cursor_round_trips_and_ids_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let files = CalendarFiles::at(tmp.path());
        let a = SyncCursor {
            sync_token: Some("tok-a".into()),
            last_synced_at_ms: Some(1),
        };
        let b = SyncCursor {
            sync_token: Some("tok-b".into()),
            last_synced_at_ms: Some(2),
        };
        files.save_cursor("a.b", a.clone()).await.unwrap();
        files.save_cursor("a_b", b.clone()).await.unwrap();
        assert_eq!(files.load_cursor("a.b").await.unwrap(), a);
        assert_eq!(files.load_cursor("a_b").await.unwrap(), b);
        // No temp files are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path().join(CURSORS_DIR))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[tokio::test]
    async fn legacy_cursor_file_is_read_when_the_new_one_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let cursors = tmp.path().join(CURSORS_DIR);
        std::fs::create_dir_all(&cursors).unwrap();
        // Shape and name written by older builds.
        std::fs::write(
            cursors.join("xyz_group_calendar_google_com.json"),
            r#"{ "sync_token": "old-tok", "last_synced_at_ms": 5 }"#,
        )
        .unwrap();
        let files = CalendarFiles::at(tmp.path());
        let id = "xyz@group.calendar.google.com";
        let got = files.load_cursor(id).await.unwrap();
        assert_eq!(got.sync_token.as_deref(), Some("old-tok"));

        // Once saved under the new name, the new file wins.
        files
            .save_cursor(
                id,
                SyncCursor {
                    sync_token: Some("new-tok".into()),
                    last_synced_at_ms: Some(6),
                },
            )
            .await
            .unwrap();
        let got = files.load_cursor(id).await.unwrap();
        assert_eq!(got.sync_token.as_deref(), Some("new-tok"));
    }

    #[tokio::test]
    async fn calendars_round_trip_and_toggle() {
        let tmp = tempfile::tempdir().unwrap();
        let files = CalendarFiles::at(tmp.path());
        assert!(files.load_calendars().await.unwrap().is_empty());
        files
            .merge_calendars(|_| vec![calendar("one", true), calendar("two", false)])
            .await
            .unwrap();
        let updated = files.set_sync_enabled("two", true).await.unwrap().unwrap();
        assert!(updated.iter().all(|c| c.sync_enabled));
        assert!(files
            .set_sync_enabled("nope", true)
            .await
            .unwrap()
            .is_none());
        assert_eq!(files.load_calendars().await.unwrap(), updated);
    }

    /// A corrupt list used to read as `[]`, and the next save then replaced
    /// the user's toggles with defaults.
    #[tokio::test]
    async fn corrupt_calendar_list_is_an_error_and_is_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(CALENDARS_FILENAME);
        std::fs::write(&path, b"{ not json").unwrap();
        let files = CalendarFiles::at(tmp.path());

        assert!(matches!(
            files.load_calendars().await,
            Err(CursorError::Corrupt { .. })
        ));
        assert!(matches!(
            files.set_sync_enabled("one", true).await,
            Err(CursorError::Corrupt { .. })
        ));
        let merged = files
            .merge_calendars(|existing| {
                assert!(existing.is_empty());
                vec![calendar("fresh", true)]
            })
            .await;
        assert!(matches!(merged, Err(CursorError::Corrupt { .. })));
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }

    /// Concurrent toggles must not lose each other's writes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_toggles_are_serialized() {
        let tmp = tempfile::tempdir().unwrap();
        let files = CalendarFiles::at(tmp.path());
        let ids: Vec<String> = (0..16).map(|i| format!("cal-{i}")).collect();
        let initial: Vec<Calendar> = ids.iter().map(|id| calendar(id, false)).collect();
        files.merge_calendars(move |_| initial).await.unwrap();

        let tasks: Vec<_> = ids
            .iter()
            .map(|id| {
                let files = files.clone();
                let id = id.clone();
                tokio::spawn(async move { files.set_sync_enabled(&id, true).await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let list = files.load_calendars().await.unwrap();
        assert!(list.iter().all(|c| c.sync_enabled), "{list:?}");
    }

    #[tokio::test]
    async fn unresolvable_location_is_an_error() {
        let files = CalendarFiles {
            dir: None,
            lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        assert!(matches!(
            files.load_calendars().await,
            Err(CursorError::NoDataDir)
        ));
    }
}
