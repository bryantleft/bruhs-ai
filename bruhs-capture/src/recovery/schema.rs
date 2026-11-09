use super::RecoveryError;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::path::{Path, PathBuf};

const APPLICATION_ID: i64 = 0x42524352;

pub(super) fn open(path: &Path) -> Result<Connection, RecoveryError> {
    let path = prepare_file(path)?;
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let app: i64 = transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if app == 0 && version == 0 {
        let tables: i64 = transaction.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if tables != 0 {
            return Err(RecoveryError::UnknownFormat);
        }
        transaction.execute_batch(
            "CREATE TABLE recovery (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id TEXT NOT NULL UNIQUE,
            token TEXT NOT NULL
        );",
        )?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", 1)?;
    } else if app != APPLICATION_ID || ![1, 2].contains(&version) {
        return Err(RecoveryError::UnknownFormat);
    }
    if version < 2 {
        transaction.execute_batch(
            "CREATE TABLE capture_failures (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id TEXT NOT NULL,
                token TEXT NOT NULL,
                started_at_ms REAL NOT NULL,
                failed_at_ms REAL NOT NULL,
                UNIQUE(meeting_id, started_at_ms)
            );",
        )?;
        transaction.pragma_update(None, "user_version", 2)?;
    }
    transaction.commit()?;
    Ok(connection)
}

fn prepare_file(path: &Path) -> std::io::Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(parent)?;
    // Resolve /var -> /private/var on macOS, but never resolve the final file.
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("Recovery database needs a filename"))?;
    let path = parent.canonicalize()?.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(file) => {
            file.sync_all()?;
            std::fs::File::open(parent)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(std::io::Error::other(
                    "Recovery database must be a regular file",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(std::io::Error::other(
                        "Recovery database permissions are not private",
                    ));
                }
            }
        }
        Err(error) => return Err(error),
    }
    Ok(path)
}
