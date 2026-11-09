use super::*;
use rusqlite::OpenFlags;

const APPLICATION_ID: i64 = 0x42524853;

pub(super) fn open(path: &Path) -> MeetingStoreResult<Connection> {
    let path = prepare_file(path).map_err(|_| {
        MeetingStoreError::PersistenceFailed(
            "Private database file could not be opened safely.".into(),
        )
    })?;
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
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
            return Err(MeetingStoreError::PersistenceFailed(
                "Unrecognized database; original file was preserved.".into(),
            ));
        }
        transaction.execute_batch("CREATE TABLE meetings (id TEXT PRIMARY KEY NOT NULL, started REAL NOT NULL, payload TEXT NOT NULL);
            CREATE TABLE people (id TEXT PRIMARY KEY NOT NULL, payload TEXT NOT NULL, voice TEXT);
            CREATE TABLE person_names (name TEXT NOT NULL, person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE, PRIMARY KEY(name,person_id));
            CREATE TABLE meeting_people (meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, person_id TEXT NOT NULL, PRIMARY KEY(meeting_id,person_id));
            CREATE INDEX meeting_people_person ON meeting_people(person_id);
            CREATE TABLE segments (id TEXT PRIMARY KEY NOT NULL, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, started INTEGER NOT NULL, payload TEXT NOT NULL);
            CREATE INDEX segments_meeting ON segments(meeting_id,started,id);
            CREATE TABLE merged_people (source_id TEXT PRIMARY KEY NOT NULL, target_id TEXT NOT NULL, source_original TEXT NOT NULL, target_original TEXT NOT NULL);")?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", 1)?;
    } else if app != APPLICATION_ID || ![1, 2, 3, 4, 5].contains(&version) {
        return Err(MeetingStoreError::PersistenceFailed(
            "Unsupported database version; original file was preserved.".into(),
        ));
    }
    if version < 2 {
        transaction.execute_batch("CREATE TABLE summary_generations (id TEXT PRIMARY KEY NOT NULL, meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, pending INTEGER NOT NULL CHECK(pending IN (0,1)), payload TEXT NOT NULL);
            CREATE UNIQUE INDEX summary_generation_pending ON summary_generations(meeting_id) WHERE pending=1;")?;
        transaction.pragma_update(None, "user_version", 2)?;
    }
    if version < 3 {
        transaction.execute_batch("CREATE TABLE speaker_link_jobs (
            meeting_id TEXT PRIMARY KEY NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            token TEXT NOT NULL, attempts INTEGER NOT NULL CHECK(attempts >= 0), due INTEGER NOT NULL);
            CREATE INDEX speaker_link_jobs_due ON speaker_link_jobs(due,meeting_id);")?;
        transaction.pragma_update(None, "user_version", 3)?;
    }
    if version < 4 {
        transaction.execute_batch(
            "CREATE TABLE local_focus (
            id TEXT PRIMARY KEY NOT NULL,
            meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            observed INTEGER NOT NULL, payload TEXT NOT NULL);
            CREATE INDEX local_focus_meeting ON local_focus(meeting_id,observed,id);",
        )?;
        transaction.pragma_update(None, "user_version", 4)?;
    }
    if version < 5 {
        transaction.execute_batch(
            "CREATE TABLE segment_removals (
            meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            segment_id TEXT NOT NULL,
            PRIMARY KEY(meeting_id, segment_id));",
        )?;
        transaction.pragma_update(None, "user_version", 5)?;
    }
    transaction.commit()?;
    let check: String = connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
    if check != "ok" {
        return Err(MeetingStoreError::PersistenceFailed(
            "Database integrity check failed; original file was preserved.".into(),
        ));
    }
    connection.pragma_update(None, "foreign_keys", true)?;
    let journal: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !journal.eq_ignore_ascii_case("wal") {
        return Err(MeetingStoreError::PersistenceFailed(
            "Database could not enable its recovery journal.".into(),
        ));
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(connection)
}

fn prepare_file(path: &Path) -> std::io::Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    // macOS exposes its temporary directory through /var -> /private/var.
    // Resolve only the parent: resolving the file itself would hide a forbidden
    // database symlink from both the metadata check and SQLite's NOFOLLOW flag.
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("Database path must name a regular file"))?;
    let path = parent.canonicalize()?.join(file_name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(file) => file.sync_all(),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(std::io::Error::other("Database must be a regular file"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(std::io::Error::other(
                        "Database permissions are not private",
                    ));
                }
            }
            Ok(())
        }
        Err(error) => Err(error),
    }?;
    Ok(path)
}
