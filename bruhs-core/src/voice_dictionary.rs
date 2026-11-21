//! Preferred spellings owned by a host, with project-specific overrides.
use bruhs_models::voice_dictionary::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("The host dictionary store is unavailable.")]
    Storage(#[from] rusqlite::Error),
    #[error("The host dictionary could not be read.")]
    Json(#[from] serde_json::Error),
    #[error("The host dictionary folder is unavailable.")]
    Io(#[from] io::Error),
}
fn file() -> Result<PathBuf, Error> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|d| d.data_dir().join("voice-dictionaries.sqlite3"))
        .ok_or_else(|| io::Error::other("No host data directory").into())
}
pub async fn execute(req: DictionaryRequest) -> Result<HostDictionaries, Error> {
    tokio::task::spawn_blocking(move || execute_in(&file()?, req))
        .await
        .map_err(io::Error::other)?
}
fn current(conn: &Connection, scope: &str) -> Result<VoiceDictionary, Error> {
    let data: Option<String> = conn
        .query_row(
            "SELECT words FROM voice_dictionary WHERE scope=?1",
            [scope],
            |row| row.get(0),
        )
        .optional()?;
    Ok(data
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default())
}
fn execute_in(path: &Path, req: DictionaryRequest) -> Result<HostDictionaries, Error> {
    if req.project.as_deref().is_some_and(|p| {
        !Path::new(p).is_absolute() || p.len() > 4096 || p.chars().any(char::is_control)
    }) {
        return Err(Error::Invalid("Select an absolute project folder.".into()));
    }
    if req.edit.as_ref().is_some_and(|e| e.for_project) && req.project.is_none() {
        return Err(Error::Invalid("Select a project dictionary.".into()));
    }
    if req.edit.is_none() && !path.try_exists()? {
        return Ok(HostDictionaries::default());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS voice_dictionary(scope TEXT PRIMARY KEY,words TEXT NOT NULL)",
    )?;
    let tx = conn.transaction_with_behavior(if req.edit.is_some() {
        rusqlite::TransactionBehavior::Immediate
    } else {
        rusqlite::TransactionBehavior::Deferred
    })?;
    let mut data = HostDictionaries {
        environment: current(&tx, "")?,
        project: if let Some(project) = &req.project {
            current(&tx, project)?
        } else {
            VoiceDictionary::default()
        },
    };
    if let Some(edit) = req.edit {
        let (scope, dict) = if edit.for_project {
            (req.project.as_deref().unwrap_or(""), &mut data.project)
        } else {
            ("", &mut data.environment)
        };
        match edit.operation {
            DictionaryOperation::Add { words, alias_for } => dict
                .add(&words, alias_for.as_deref())
                .map_err(Error::Invalid)?,
            DictionaryOperation::Remove { term } => {
                dict.0.remove(&term);
            }
        }
        if dict.0.is_empty() {
            tx.execute("DELETE FROM voice_dictionary WHERE scope=?1", [scope])?;
        } else {
            tx.execute("INSERT INTO voice_dictionary(scope,words) VALUES(?1,?2) ON CONFLICT(scope) DO UPDATE SET words=excluded.words",params![scope,serde_json::to_string(dict)?])?;
        }
    }
    tx.commit()?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_leave_new_hosts_empty_and_failed_edits_preserve_other_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite3");
        let project = dir.path().join("Orchard").to_string_lossy().into_owned();
        let request = |project: Option<&str>, edit| DictionaryRequest {
            project: project.map(str::to_string),
            edit,
        };
        let add = |for_project, words: &str| {
            Some(DictionaryEdit {
                for_project,
                operation: DictionaryOperation::Add {
                    words: words.into(),
                    alias_for: None,
                },
            })
        };
        assert!(
            execute_in(&path, request(Some(&project), None))
                .unwrap()
                .project
                .0
                .is_empty()
        );
        assert!(!path.exists());
        assert!(matches!(
            execute_in(&path, request(None, add(true, "Apple"))),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            execute_in(&path, request(Some("relative"), None)),
            Err(Error::Invalid(_))
        ));
        execute_in(&path, request(None, add(false, "Orchard"))).unwrap();
        execute_in(&path, request(Some(&project), add(true, "Apple"))).unwrap();
        assert!(execute_in(&path, request(Some(&project), add(true, ""))).is_err());
        let local = execute_in(&path, request(Some(&project), None)).unwrap();
        assert!(local.project.0.contains_key("Apple"));
        assert!(local.environment.0.contains_key("Orchard"));
        assert!(
            execute_in(&path, request(None, None))
                .unwrap()
                .project
                .0
                .is_empty()
        );
    }
}
