//! OpenCode Go's usage endpoint for clients without Console workspace metadata.
use super::{account, body, error, window, AccountUsage};
use reqwest::{Client, StatusCode};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

type Key = Zeroizing<String>;

pub(super) async fn read() -> io::Result<Option<AccountUsage>> {
    let keys = tokio::task::spawn_blocking(keys)
        .await
        .map_err(|_| error())??;
    if keys.is_empty() {
        return Ok(None);
    }
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| error())?;
    fetch(&client, "https://opencode.ai/zen/go/v1/usage", keys).await
}
fn keys() -> io::Result<Vec<Key>> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".local/share")))
        .ok_or_else(error)?
        .join("opencode");
    let inline = std::env::var("OPENCODE_AUTH_CONTENT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(Zeroizing::new);
    let db = std::env::var_os("OPENCODE_DB")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "opencode.db".into());
    let mut keys = if inline.is_none() && db != ":memory:" {
        stored_keys(&data.join(db))?
    } else {
        Vec::new()
    };
    let auth = match inline {
        Some(value) => Some(value),
        None => match fs::File::open(data.join("auth.json")) {
            Ok(file) => {
                let mut value = Zeroizing::new(String::new());
                file.take(2 * 1024 * 1024 + 1)
                    .read_to_string(&mut value)
                    .map_err(|_| error())?;
                if value.len() > 2 * 1024 * 1024 {
                    return Err(error());
                }
                Some(value)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return Err(error()),
        },
    };
    #[derive(Deserialize)]
    struct File {
        #[serde(rename = "opencode-go")]
        go: Option<Credential>,
    }
    let from_file = auth
        .as_deref()
        .map(|s| serde_json::from_str::<File>(s))
        .transpose()
        .map_err(|_| error())?
        .and_then(|f| f.go)
        .filter(|v| v.kind == "api")
        .and_then(|v| v.key);
    if let Some(key) = from_file.or_else(|| std::env::var("OPENCODE_API_KEY").ok()) {
        push(&mut keys, key);
    }
    Ok(keys)
}

fn push(keys: &mut Vec<Key>, value: String) {
    let value = Zeroizing::new(value);
    let key = value.trim();
    if !key.is_empty() && !keys.iter().any(|old| old.as_str() == key) {
        keys.push(Zeroizing::new(key.to_owned()));
    }
}
#[derive(Deserialize)]
struct Credential {
    #[serde(rename = "type", default)]
    kind: String,
    key: Option<String>,
    metadata: Option<Metadata>,
}
#[derive(Deserialize)]
struct Metadata {
    server: Option<String>,
}

fn stored_keys(path: &Path) -> io::Result<Vec<Key>> {
    let mut keys = Vec::new();
    if !path.try_exists().map_err(|_| error())? {
        return Ok(keys);
    }
    let db =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|_| error())?;
    db.busy_timeout(Duration::from_millis(100))
        .map_err(|_| error())?;
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='credential')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| error())?;
    if !exists {
        return Ok(keys);
    }
    for id in ["opencode", "opencode-go"] {
        let data: Option<String> = db
            .query_row(
                "SELECT CASE WHEN length(value) <= 1048576 THEN value END
             FROM credential WHERE integration_id = ? AND active IS NOT 0
             ORDER BY active DESC, time_created DESC, id DESC LIMIT 1",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|_| error())?
            .flatten();
        let Some(data) = data.map(Zeroizing::new) else {
            continue;
        };
        let Ok(stored) = serde_json::from_str::<Credential>(&data) else {
            continue;
        };
        if stored.kind != "key" {
            continue;
        }
        if let Some(server) = stored.metadata.and_then(|m| m.server) {
            // A credential issued by a different console must stay with its issuer.
            if server.trim_end_matches('/') != "https://opencode.ai/console" {
                continue;
            }
        }
        if let Some(key) = stored.key {
            push(&mut keys, key);
        }
    }
    Ok(keys)
}
#[derive(Deserialize)]
struct Usage {
    usage: Windows,
}
#[derive(Deserialize)]
struct Windows {
    rolling: Meter,
    weekly: Meter,
    monthly: Meter,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meter {
    percent: f64,
    resets_at: String,
}

async fn fetch(
    client: &Client,
    endpoint: &str,
    keys: Vec<Key>,
) -> io::Result<Option<AccountUsage>> {
    let mut keys = keys.into_iter().peekable();
    while let Some(key) = keys.next() {
        let response = client
            .get(endpoint)
            .bearer_auth(key.as_str())
            .send()
            .await
            .map_err(|_| error())?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) && keys.peek().is_some()
        {
            continue;
        }
        if response.status() == StatusCode::FORBIDDEN {
            return Ok(None);
        }
        let Usage { usage } = body(response).await?;
        let mut hash = Sha256::new();
        hash.update(b"opencode-go\0");
        hash.update(key.as_bytes());
        let windows = [
            ("Session", usage.rolling, Some(300)),
            ("Weekly", usage.weekly, Some(10080)),
            ("Monthly", usage.monthly, None),
        ]
        .into_iter()
        .map(|(kind, meter, duration)| {
            Ok(window(
                "OpenCode Go",
                None,
                format!("Go · {kind}"),
                meter.percent,
                duration,
                chrono::DateTime::parse_from_rfc3339(&meter.resets_at)
                    .map_err(|_| error())?
                    .timestamp(),
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
        return Ok(Some(account(
            "OpenCode Go".into(),
            None,
            Some(format!("{:x}", hash.finalize())),
            Ok(windows),
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[test]
    fn legacy_credentials_use_only_the_active_opencode_console_and_go_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        assert!(stored_keys(&path).unwrap().is_empty());
        assert!(!path.exists());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE credential (id INTEGER, integration_id TEXT, active INTEGER, time_created INTEGER, value TEXT)").unwrap();
        for (id, driver, active, key, issuer) in [
            (0, "opencode", 1, "old", "https://opencode.ai/console"),
            (1, "opencode", 1, "local", "https://opencode.ai/console/"),
            (
                2,
                "opencode-go",
                1,
                "foreign",
                "https://example.test/console",
            ),
            (3, "other", 1, "unrelated", "https://opencode.ai/console"),
            (4, "opencode", 0, "inactive", "https://opencode.ai/console"),
        ] {
            let value = serde_json::json!({"type":"key","key":key,"metadata":{"server":issuer}})
                .to_string();
            db.execute(
                "INSERT INTO credential VALUES (?1,?2,?3,?1,?4)",
                rusqlite::params![id, driver, active, value],
            )
            .unwrap();
        }
        let keys = stored_keys(&path).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].as_str(), "local");
    }

    #[tokio::test]
    async fn legacy_go_retries_only_auth_failures_and_reports_numeric_windows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/usage"))
            .and(header("Authorization", "Bearer expired"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let meter = serde_json::json!({"percent":27,"resetsAt":"2030-01-02T03:00:00Z"});
        Mock::given(method("GET"))
            .and(path("/usage"))
            .and(header("Authorization", "Bearer active"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"usage":{"rolling":meter,"weekly":meter,"monthly":meter}}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let report = fetch(
            &Client::new(),
            &format!("{}/usage", server.uri()),
            vec![Key::new("expired".into()), Key::new("active".into())],
        )
        .await
        .unwrap()
        .unwrap();
        assert!(report.external.is_none());
        let rows = report.windows.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].duration_minutes, Some(10080));
        assert_eq!(rows[0].used_percent, 27.);
        assert_eq!(report.identity.fingerprint.unwrap().len(), 64);
    }
}
