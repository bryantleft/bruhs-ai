//! T3 Code threads, from its state database (`<T3 home>/userdata/
//! statev2.sqlite`, opened read-only). Each T3 thread runs on a Claude Code
//! or Codex session, which the database names (`nativeThreadRef`), so the
//! import resumes that session: the thread's history comes from the
//! session's own transcript, and its title and project from T3.

use super::{claude, codex, parse_time, ImportError, ImportRoots};
use bruhs_models::agent::AgentProvider;
use bruhs_models::logic::portable::{ImportCandidate, ImportSource, PortableMessage};
use bruhs_models::EpochMs;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

fn database(root: &Path) -> PathBuf {
    root.join("userdata").join("statev2.sqlite")
}

fn open(root: &Path) -> Result<Option<Connection>, ImportError> {
    let path = database(root);
    if !path.is_file() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| ImportError::Database("T3 Code", error.to_string()))?;
    // T3 writes while it runs; wait for its write instead of failing.
    let _ = connection.busy_timeout(std::time::Duration::from_secs(2));
    Ok(Some(connection))
}

/// Each live thread with its project root and the provider thread it runs
/// on now (the active one, else the newest).
const THREADS: &str = "
SELECT t.thread_id, t.title, t.updated_at, p.workspace_root,
  (SELECT pt.payload_json FROM orchestration_v2_projection_provider_threads pt
     WHERE pt.thread_id = t.thread_id
     ORDER BY (pt.provider_thread_id = t.active_provider_thread_id) DESC, pt.updated_at DESC
     LIMIT 1)
FROM orchestration_v2_projection_threads t
LEFT JOIN projection_projects p ON p.project_id = t.project_id
WHERE t.deleted_at IS NULL AND t.archived_at IS NULL";

struct Row {
    id: String,
    title: String,
    updated_at: Option<String>,
    root: Option<String>,
    provider_thread: Option<String>,
}

/// The CLI session a provider thread runs on.
fn native_session(payload: &str) -> Option<(AgentProvider, String)> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let native = value.get("nativeThreadRef")?;
    let id = native.get("nativeId")?.as_str()?.to_string();
    let provider = match native.get("driver")?.as_str()? {
        "claudeAgent" => AgentProvider::Claude,
        "codex" => AgentProvider::Codex,
        _ => return None,
    };
    Some((provider, id))
}

fn rows(connection: &Connection, filter: Option<&str>) -> Result<Vec<Row>, ImportError> {
    let sql = match filter {
        Some(_) => format!("{THREADS} AND t.thread_id = ?1"),
        None => THREADS.to_string(),
    };
    let unreadable = |error: rusqlite::Error| {
        ImportError::Database(
            "T3 Code",
            format!("{error} (its database layout may have changed)"),
        )
    };
    let mut statement = connection.prepare(&sql).map_err(unreadable)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(Row {
            id: row.get(0)?,
            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            updated_at: row.get(2)?,
            root: row.get(3)?,
            provider_thread: row.get(4)?,
        })
    };
    let found = match filter {
        Some(id) => statement.query_map([id], map),
        None => statement.query_map([], map),
    }
    .map_err(unreadable)?
    .filter_map(Result::ok)
    .collect();
    Ok(found)
}

fn candidate(row: Row) -> Option<ImportCandidate> {
    let (provider, session) = native_session(row.provider_thread.as_deref()?)?;
    Some(ImportCandidate {
        source: ImportSource::T3,
        id: row.id,
        title: if row.title.trim().is_empty() {
            "T3 thread".to_string()
        } else {
            row.title
        },
        cwd: row.root,
        updated_at: row
            .updated_at
            .as_deref()
            .and_then(parse_time)
            .map_or(EpochMs::UNIX_EPOCH, EpochMs::saturating_from_i64),
        provider: Some(provider),
        session_id: Some(session),
        model: None,
    })
}

pub(super) fn list(
    root: &Path,
    keep: &dyn Fn(&ImportCandidate) -> bool,
) -> Result<Vec<ImportCandidate>, ImportError> {
    let Some(connection) = open(root)? else {
        return Ok(Vec::new());
    };
    Ok(rows(&connection, None)?
        .into_iter()
        .filter_map(candidate)
        .filter(|candidate| keep(candidate))
        .collect())
}

/// The T3 thread `id`: T3's title, the session's transcript, and the
/// session's own working directory (a T3 worktree, when it ran in one).
pub(super) fn load(
    roots: &ImportRoots,
    id: &str,
) -> Result<(ImportCandidate, Vec<PortableMessage>), ImportError> {
    let not_found = || ImportError::not_found(ImportSource::T3, id);
    let connection = open(&roots.t3)?.ok_or_else(not_found)?;
    let thread = rows(&connection, Some(id))?
        .into_iter()
        .find_map(candidate)
        .ok_or_else(not_found)?;
    let session = thread.session_id.clone().unwrap_or_default();
    let (session_thread, messages) = match thread.provider {
        Some(AgentProvider::Codex) => codex::load(&roots.codex, &session)?,
        _ => claude::load(&roots.claude, &session)?,
    };
    Ok((
        ImportCandidate {
            cwd: session_thread.cwd.or(thread.cwd),
            model: session_thread.model,
            updated_at: thread.updated_at.max(session_thread.updated_at),
            ..thread
        },
        messages,
    ))
}
