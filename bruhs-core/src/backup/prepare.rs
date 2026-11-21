//! Preparing a staged restore for this computer (D10 to D12, D14; rulings
//! F17, F18 and the Task 3 and Task 4 notes), between the checks of an
//! open and the seal of its keychain items.
//!
//! Every path the backup's computer named moves to this one: each staged
//! JSON file and SQLite database outside the owner's own files is
//! rewritten, the credential catalogs are re-keyed (D12) and the staged
//! keychain items follow their accounts. Absolute paths that no rule moves
//! are kept and named for the preview. The staged crew is paused with
//! fresh sessions (D14, F18). Only staged copies change; live data never.
//!
//! Staged databases are hostile input, like the rest of the backup: each
//! passes `PRAGMA quick_check` first and is written with defensive mode
//! on, an untrusted schema and its triggers off (F17), and only through
//! SQLite, never as raw bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use bruhs_models::backup::{
    collect_outside, collect_outside_in, folded_entry_key, path_field, rewritable, rewrite_json,
    rewrite_path, rewrite_rules, PathRewrite, RootsInfo, DAMAGED,
};
use bruhs_models::weekly_reports::managed_preset_id;
use rusqlite::config::DbConfig;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde_json::Value;

use super::host::random_id;
use super::open::{meta_folder, staging_dir, staging_roots, sync_folder, write_new, Staged, META};
use super::{BackupError, RootKey, Roots};
use crate::reactive::pause_all_for_restore;
use crate::secrets::{check_items, rekey_catalogs, AccountMap, KeychainItem};

/// The staged folder of the credential catalogs, re-keyed on their own.
const CREDENTIALS: &str = "credentials";
/// The catalogs whose project paths the preview checks.
const CATALOGS: [&str; 2] = ["catalog.json", "vault.json"];
/// The staged reactive database, relative to the core staging.
const REACTIVE_DB: &str = "server/reactive.sqlite3";
/// The old account to new account map, in the core staging's `meta`.
const ACCOUNTS_FILE: &str = "accounts.json";
/// The first bytes of every SQLite database.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// A rewritten file's temporary name starts with this, beside the file.
const REWRITE_PREFIX: &str = ".bruhs-rewrite-";

/// What preparing a staged restore did, for its preview.
pub(crate) struct Prepared {
    /// The prefixes moved, longest first: empty on the same computer.
    pub rewrites: Vec<PathRewrite>,
    /// Absolute paths no rule moves, by their first two components.
    pub outside_home: Vec<String>,
    /// Bruhs in the staged reactive database, managed presets aside.
    pub bruhs: u32,
    /// Vault entries and legacy saved values in the staged catalogs.
    pub vault_entries: u32,
}

/// Prepares `staged` for `roots`, this computer's (D10 to D12, D14):
/// rewrites paths in the staged data, re-keys the staged catalogs and
/// renames `staged.items` to match, stores the account map in `meta`, and
/// pauses the staged crew. Any error leaves the staging half-written: the
/// caller discards all of it and never prepares it again (B Task 4 note).
pub(crate) async fn prepare(roots: &Roots, staged: &mut Staged) -> Result<Prepared, BackupError> {
    let roots = roots.clone();
    let id = staged.id.clone();
    let from = RootsInfo {
        home: staged.manifest.home.clone(),
        core: staged.manifest.core_root.clone(),
        desktop: staged.manifest.desktop_root.clone(),
    };
    let mut items = std::mem::take(&mut staged.items);
    let (prepared, items) = tokio::task::spawn_blocking(move || {
        let prepared = prepare_staged(&roots, &id, &from, &mut items);
        (prepared, items)
    })
    .await
    .map_err(|_| BackupError::Io(io::Error::other("the prepare worker stopped")))?;
    staged.items = items;
    prepared
}

fn prepare_staged(
    roots: &Roots,
    id: &str,
    from: &RootsInfo,
    items: &mut [KeychainItem],
) -> Result<Prepared, BackupError> {
    let rules = rewrite_rules(from, &roots.info());
    let core = staging_dir(roots, RootKey::Core, id);
    let credentials = core.join(CREDENTIALS);
    let mut outside = BTreeSet::new();
    let mut map = AccountMap::default();
    // The same computer moves nothing, so its staged data stays as the
    // backup holds it.
    if !rules.is_empty() {
        for root in staging_roots(roots) {
            rewrite_tree(&staging_dir(roots, *root, id), &rules, &mut outside)?;
        }
        note_catalogs(&credentials, &rules, &mut outside)?;
        // Writes `catalog.json`, then `vault.json`: a failure between them
        // fails the open, which discards the whole staging.
        map = rekey_catalogs(&credentials, &rules).map_err(|_| damaged())?;
        map.apply(items);
        // A renamed item that lands on another's account is damage.
        check_items(items).map_err(|_| damaged())?;
    }
    let accounts = serde_json::to_vec(&map.0).map_err(io::Error::other)?;
    write_new(&meta_folder(&core)?.join(ACCOUNTS_FILE), &accounts)?;
    let bruhs = pause_crew(&core.join(REACTIVE_DB))?;
    Ok(Prepared {
        rewrites: rules,
        outside_home: outside.into_iter().collect(),
        bruhs,
        vault_entries: vault_entries(&credentials),
    })
}

fn damaged() -> BackupError {
    BackupError::Format(DAMAGED)
}

/// Rewrites every JSON file and SQLite database under `folder`, a staging
/// root, that [`rewritable`] allows: never `meta`, `credentials`, the
/// content folders or a bruh's `work` and `browser` folders. Answers how
/// many strings changed.
fn rewrite_tree(
    folder: &Path,
    rules: &[PathRewrite],
    outside: &mut BTreeSet<String>,
) -> Result<usize, BackupError> {
    let mut changed = 0;
    let mut folders = vec![(folder.to_path_buf(), String::new())];
    while let Some((at, rel)) = folders.pop() {
        for entry in std::fs::read_dir(&at)? {
            let entry = entry?;
            // Every staged name is UTF-8 (the open refused any other).
            let Ok(name) = entry.file_name().into_string() else {
                return Err(damaged());
            };
            if rel.is_empty() && folded_entry_key(&name) == META {
                continue;
            }
            let rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if !rewritable(&rel) {
                continue;
            }
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                folders.push((path, rel));
            } else if kind.is_file() {
                if json_name(&name) {
                    changed += rewrite_json_file(&path, rules, outside)?;
                } else if is_sqlite(&path)? {
                    changed += rewrite_sqlite(&path, rules, outside)?;
                }
            }
        }
    }
    Ok(changed)
}

/// JSON documents, and the desktop's backup copy of its state.
fn json_name(name: &str) -> bool {
    name.ends_with(".json") || name.ends_with(".json.bak")
}

/// Opens a staged file for reading, never through a link.
fn open_staged_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

/// Whether the staged file at `path` starts as a SQLite database does.
/// Staged files are no live database's, so a plain read is safe here.
fn is_sqlite(path: &Path) -> Result<bool, BackupError> {
    let mut head = [0_u8; 16];
    let mut file = open_staged_file(path)?;
    match file.read_exact(&mut head) {
        Ok(()) => Ok(&head == SQLITE_MAGIC),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Rewrites the paths in the JSON file at `path` (D10): every string value
/// and object key, kept in its compact or indented form. A file that isn't
/// JSON is left as it is. Adds the paths no rule moves to `outside` and
/// answers how many strings changed. The file is replaced whole, through a
/// new file beside it.
pub fn rewrite_json_file(
    path: &Path,
    rules: &[PathRewrite],
    outside: &mut BTreeSet<String>,
) -> Result<usize, BackupError> {
    let mut text = Vec::new();
    open_staged_file(path)?.read_to_end(&mut text)?;
    let Ok(mut value) = serde_json::from_slice::<Value>(&text) else {
        return Ok(0);
    };
    collect_outside(&value, rules, outside);
    let changed = rewrite_json(&mut value, rules);
    if changed == 0 {
        return Ok(0);
    }
    let bytes = if text.trim_ascii().contains(&b'\n') {
        serde_json::to_vec_pretty(&value)
    } else {
        serde_json::to_vec(&value)
    }
    .map_err(io::Error::other)?;
    replace_file(path, &bytes)?;
    Ok(changed)
}

/// Replaces the staged file at `path` with `bytes`, keeping whether its
/// owner may run it: a new file beside it, synced, renamed over it.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let mode = if std::fs::symlink_metadata(path)?.permissions().mode() & 0o100 == 0 {
        0o600
    } else {
        0o700
    };
    let folder = path.parent().ok_or_else(damaged)?;
    let temporary = folder.join(format!("{REWRITE_PREFIX}{}", random_id()));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.set_permissions(Permissions::from_mode(mode))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if let Err(error) = written {
        // Best effort: the staging goes with the failed open anyway.
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    sync_folder(folder)
}

/// Rewrites the paths in the staged SQLite database at `path` (D10), in
/// one transaction: every text cell of every ordinary table (never a
/// virtual table, its shadow tables or SQLite's own) that holds a rule's
/// old prefix, walked as JSON when it parses as an object or array and
/// otherwise rewritten as a whole string. Adds the paths no rule moves,
/// from path columns and the path fields of JSON cells, to `outside`, and
/// answers how many cells changed. A database that fails `quick_check`,
/// or any statement, is `DAMAGED` (F17).
pub fn rewrite_sqlite(
    path: &Path,
    rules: &[PathRewrite],
    outside: &mut BTreeSet<String>,
) -> Result<usize, BackupError> {
    let mut connection = open_staged_database(path)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_failed)?;
    let mut changed = 0;
    for table in plain_tables(&transaction)? {
        for column in columns(&transaction, &table)? {
            changed += rewrite_column(&transaction, &table, &column, rules, outside)?;
        }
    }
    transaction.commit().map_err(sqlite_failed)?;
    connection
        .close()
        .map_err(|(_, error)| sqlite_failed(error))?;
    Ok(changed)
}

/// Opens a staged database for writing as F17 asks: no link anywhere in
/// its path, defensive mode on, the schema untrusted and its triggers off,
/// then `PRAGMA quick_check`.
fn open_staged_database(path: &Path) -> Result<Connection, BackupError> {
    // SQLite's no-follow refuses a link anywhere in the path (`/var` is
    // one on macOS), so the staging folder, which Bruhs made, is resolved.
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(damaged());
    };
    let path = std::fs::canonicalize(folder)?.join(name);
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(sqlite_failed)?;
    for (config, on) in [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
    ] {
        if connection
            .set_db_config(config, on)
            .map_err(sqlite_failed)?
            != on
        {
            return Err(damaged());
        }
    }
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(sqlite_failed)?;
    if check != "ok" {
        return Err(damaged());
    }
    Ok(connection)
}

/// A SQLite failure on staged data: a full disk or an io error is
/// [`BackupError::Io`]; anything else means the database is damaged.
fn sqlite_failed(error: rusqlite::Error) -> BackupError {
    match &error {
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DiskFull | rusqlite::ErrorCode::SystemIoFailure
            ) =>
        {
            BackupError::Io(io::Error::other("a staged database couldn't be written"))
        }
        _ => damaged(),
    }
}

/// The ordinary tables, in the order the schema made them: not SQLite's
/// own (`sqlite_%`), not a virtual table and not one whose name starts
/// with a virtual table's name and `_` (its shadow tables).
fn plain_tables(connection: &Connection) -> Result<Vec<String>, BackupError> {
    let mut statement = connection
        .prepare("SELECT name, sql FROM sqlite_master WHERE type='table' ORDER BY rowid")
        .map_err(sqlite_failed)?;
    let tables = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(sqlite_failed)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sqlite_failed)?;
    let virtual_tables: Vec<String> = tables
        .iter()
        .filter(|(_, sql)| {
            sql.as_deref().is_some_and(|sql| {
                sql.trim_start()
                    .to_ascii_uppercase()
                    .starts_with("CREATE VIRTUAL TABLE")
            })
        })
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect();
    Ok(tables
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| {
            let name = name.to_ascii_lowercase();
            !name.starts_with("sqlite_")
                && !virtual_tables.iter().any(|table| {
                    name == *table
                        || name
                            .strip_prefix(table.as_str())
                            .is_some_and(|rest| rest.starts_with('_'))
                })
        })
        .collect())
}

/// A table's columns, generated ones aside (`PRAGMA table_info` leaves
/// them out, and they can't be written).
fn columns(connection: &Connection, table: &str) -> Result<Vec<String>, BackupError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(sqlite_failed)?;
    let names = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(sqlite_failed)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sqlite_failed)?;
    Ok(names)
}

/// An identifier quoted for SQL, whatever it holds.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Rewrites one column's text cells and answers how many changed. Reads
/// the cells that hold a rule's old prefix, and those the outside scan
/// needs: every path in a path column, and JSON cells with a `/`.
fn rewrite_column(
    connection: &Connection,
    table: &str,
    column: &str,
    rules: &[PathRewrite],
    outside: &mut BTreeSet<String>,
) -> Result<usize, BackupError> {
    let (table, cell) = (quoted(table), quoted(column));
    let mut wanted: Vec<String> = (1..=rules.len())
        .map(|at| format!("instr({cell},?{at})>0"))
        .collect();
    wanted.push(if path_field(column) {
        format!("substr({cell},1,1)='/'")
    } else {
        format!("(substr(ltrim({cell}),1,1) IN ('{{','[') AND instr({cell},'/')>0)")
    });
    let select = format!(
        "SELECT {cell} FROM {table} WHERE typeof({cell})='text' AND ({})",
        wanted.join(" OR ")
    );
    let mut updates = BTreeMap::new();
    {
        let mut statement = connection.prepare(&select).map_err(sqlite_failed)?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(
                rules.iter().map(|rule| rule.from.as_str()),
            ))
            .map_err(sqlite_failed)?;
        while let Some(row) = rows.next().map_err(sqlite_failed)? {
            // Text that isn't UTF-8 holds no path a rule names.
            let ValueRef::Text(bytes) = row.get_ref(0).map_err(sqlite_failed)? else {
                continue;
            };
            let Ok(text) = std::str::from_utf8(bytes) else {
                continue;
            };
            if updates.contains_key(text) {
                continue;
            }
            if let Some(new) = rewritten_cell(column, text, rules, outside) {
                updates.insert(text.to_owned(), new);
            }
        }
    }
    let update = format!(
        "UPDATE {table} SET {cell}=?1 WHERE typeof({cell})='text' AND {cell}=?2 COLLATE BINARY"
    );
    let mut changed = 0;
    for (old, new) in &updates {
        changed += connection
            .execute(&update, [new, old])
            .map_err(sqlite_failed)?;
    }
    Ok(changed)
}

/// A cell's new text, or `None` when no rule moves it.
fn rewritten_cell(
    column: &str,
    text: &str,
    rules: &[PathRewrite],
    outside: &mut BTreeSet<String>,
) -> Option<String> {
    if text.trim_start().starts_with(['{', '[']) {
        if let Ok(mut value) = serde_json::from_str::<Value>(text) {
            collect_outside_in(column, &value, rules, outside);
            return (rewrite_json(&mut value, rules) > 0).then(|| value.to_string());
        }
    }
    collect_outside_in(column, &Value::String(text.to_owned()), rules, outside);
    rewrite_path(text, rules)
}

/// Adds the project paths of the staged catalogs that no rule moves to
/// `outside`: legacy `project_path`s and vault Project scopes' `path`s.
/// The catalogs themselves are re-keyed, not rewritten here.
fn note_catalogs(
    credentials: &Path,
    rules: &[PathRewrite],
    outside: &mut BTreeSet<String>,
) -> Result<(), BackupError> {
    for name in CATALOGS {
        let mut text = Vec::new();
        match open_staged_file(&credentials.join(name)) {
            Ok(mut file) => file.read_to_end(&mut text)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let value: Value = serde_json::from_slice(&text).map_err(|_| damaged())?;
        collect_outside(&value, rules, outside);
        project_scopes(&value, &mut |path| {
            collect_outside_in(
                "project_path",
                &Value::String(path.to_owned()),
                rules,
                outside,
            );
        });
    }
    Ok(())
}

/// Calls `found` with the path of every Project scope in `value`.
fn project_scopes(value: &Value, found: &mut dyn FnMut(&str)) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("project") {
                if let Some(path) = map.get("path").and_then(Value::as_str) {
                    found(path);
                }
            }
            for item in map.values() {
                project_scopes(item, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                project_scopes(item, found);
            }
        }
        _ => {}
    }
}

/// Pauses the staged crew with fresh sessions (D14, F18) and answers how
/// many bruhs it holds, managed presets aside. No staged reactive
/// database, or one that isn't a database, holds none.
fn pause_crew(path: &Path) -> Result<u32, BackupError> {
    if !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        || !is_sqlite(path)?
    {
        return Ok(0);
    }
    let mut connection = open_staged_database(path)?;
    pause_all_for_restore(&mut connection).map_err(|_| damaged())?;
    let agents = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='reactive_agents'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_failed)?;
    let bruhs = if agents == 0 {
        0
    } else {
        let mut statement = connection
            .prepare("SELECT id FROM reactive_agents")
            .map_err(sqlite_failed)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_failed)?;
        ids.iter()
            .filter(|id| managed_preset_id(id).is_none())
            .count()
    };
    connection
        .close()
        .map_err(|(_, error)| sqlite_failed(error))?;
    Ok(u32::try_from(bruhs).unwrap_or(u32::MAX))
}

/// How many vault entries and legacy saved values the staged catalogs
/// hold, counted as a backup counts them.
fn vault_entries(credentials: &Path) -> u32 {
    CATALOGS
        .iter()
        .filter_map(|name| {
            let mut text = Vec::new();
            open_staged_file(&credentials.join(name))
                .and_then(|mut file| file.read_to_end(&mut text))
                .ok()?;
            Some(text)
        })
        .filter_map(|text| serde_json::from_slice::<Value>(&text).ok())
        .filter_map(|catalog| catalog.get("entries")?.as_object().map(|map| map.len()))
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
        .fold(0, u32::saturating_add)
}
