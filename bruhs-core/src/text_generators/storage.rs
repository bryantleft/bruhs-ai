use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::PathBuf, time::Duration};

pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| "The text generator store is unavailable.".to_owned())?
}
fn database() -> Result<Connection, String> {
    let path = std::env::var_os("BRUHS_GENERATORS_DB")
        .map(PathBuf::from)
        .or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.data_local_dir().join("bruhs-ai/text-generators.sqlite3"))
        })
        .ok_or("The application data directory is unavailable.")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let db = Connection::open(path).map_err(|e| e.to_string())?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS generators (id TEXT PRIMARY KEY, revision INTEGER NOT NULL, record TEXT NOT NULL); CREATE TABLE IF NOT EXISTS retired_keys (id TEXT PRIMARY KEY);").map_err(|e|e.to_string())?;
    Ok(db)
}
pub(super) fn list() -> Result<Vec<Generator>, String> {
    let db = database()?;
    cleanup_retired(&db);
    let mut query = db
        .prepare("SELECT record FROM generators ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let rows = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.map(|r| serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string()))
        .collect()
}
pub(super) fn get(id: Uuid) -> Result<Option<Generator>, String> {
    let data: Option<String> = database()?
        .query_row(
            "SELECT record FROM generators WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    data.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        .transpose()
}
const LOCKED: &str =
    "Could not access the text generator API key. Unlock the system credential store and retry.";
fn entry(id: Uuid) -> Result<keyring::Entry, String> {
    keyring::Entry::new(crate::secrets::GENERATORS_SERVICE, &id.to_string())
        .map_err(|_| LOCKED.into())
}
pub(super) async fn read_key(id: Option<Uuid>) -> Result<Option<Zeroizing<String>>, String> {
    let Some(id) = id else { return Ok(None) };
    blocking(move || {
        entry(id)?
            .get_password()
            .map(|s| Some(Zeroizing::new(s)))
            .map_err(|_| LOCKED.into())
    })
    .await
}
fn forget(db: &Connection, id: Uuid) -> Result<(), String> {
    match entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(_) => return Err(LOCKED.into()),
    }
    db.execute("DELETE FROM retired_keys WHERE id=?1", [id.to_string()])
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn drain_retired(db: &Connection) -> Result<(), String> {
    let mut q = db
        .prepare("SELECT id FROM retired_keys")
        .map_err(|e| e.to_string())?;
    let ids = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    for id in ids {
        forget(db, Uuid::parse_str(&id).map_err(|e| e.to_string())?)?;
    }
    Ok(())
}
// Metadata is already committed when retired credentials are collected. Keep the
// queue when the keychain is locked and retry on the next load or write.
fn cleanup_retired(db: &Connection) {
    if drain_retired(db).is_err() {
        tracing::warn!("Text generator credential cleanup will retry when the store is available");
    }
}
/// Generator names displayed in the Secrets removal confirmation.
pub fn secret_source_consumers(source: &str) -> Result<Vec<String>, String> {
    let db = database()?;
    let mut q=db.prepare("SELECT json_extract(record,'$.label') FROM generators WHERE json_extract(record,'$.reference.source')=?1 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([source], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}
pub(crate) fn unset_secret_source(source: &str) -> std::io::Result<()> {
    let db = database().map_err(std::io::Error::other)?;
    db.execute("UPDATE generators SET record=json_remove(json_set(record,'$.revision',revision+1),'$.reference'), revision=revision+1 WHERE json_extract(record,'$.reference.source')=?1",[source]).map_err(std::io::Error::other)?;
    Ok(())
}
const STALE: &str = "This generator changed in another window. Reopen it and try again.";
pub(super) fn save(
    previous: Option<&Generator>,
    mut row: Generator,
    key: Option<Zeroizing<String>>,
) -> Result<Generator, String> {
    let mut db = database()?;
    cleanup_retired(&db);
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let saved: Option<i64> = tx
        .query_row(
            "SELECT revision FROM generators WHERE id=?1",
            [row.id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if saved != previous.map(|g| g.revision) {
        return Err(STALE.into());
    }
    // The immediate transaction orders this check with source removal's UPDATE.
    // Read the atomically replaced sources file without acquiring its writer lock.
    if let Some(reference) = &row.reference {
        if !crate::secret_sources::source_exists(&reference.source).map_err(|e| e.to_string())? {
            return Err("This 1Password source was removed. Choose another source.".into());
        }
    }
    let old = row.key;
    if row.reference.is_some() {
        row.key = None;
    }
    let mut created = None;
    if let Some(key) = key {
        row.key = None;
        if !key.is_empty() {
            let id = Uuid::new_v4();
            entry(id)?
                .set_password(key.as_str())
                .map_err(|_| LOCKED.to_string())?;
            row.key = Some(id);
            created = Some(id);
        }
    }
    row.revision += 1;
    let result = (|| -> Result<(), String> {
        if let Some(old) = old.filter(|id| Some(*id) != row.key) {
            tx.execute(
                "INSERT OR IGNORE INTO retired_keys VALUES(?1)",
                [old.to_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO generators VALUES (?1,?2,?3)",
            params![
                row.id.to_string(),
                row.revision,
                serde_json::to_string(&row).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    })();
    if let Err(error) = result {
        if let Some(id) = created {
            let _ = forget(&db, id);
        }
        return Err(error);
    }
    cleanup_retired(&db);
    Ok(row)
}
pub(super) fn remove(row: &Generator) -> Result<(), String> {
    let mut db = database()?;
    cleanup_retired(&db);
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let removed = tx
        .execute(
            "DELETE FROM generators WHERE id=?1 AND revision=?2",
            params![row.id.to_string(), row.revision],
        )
        .map_err(|e| e.to_string())?;
    if removed == 0 {
        return Err(STALE.into());
    }
    if let Some(key) = row.key {
        tx.execute(
            "INSERT OR IGNORE INTO retired_keys VALUES (?1)",
            [key.to_string()],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    cleanup_retired(&db);
    Ok(())
}
