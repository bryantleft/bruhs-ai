//! Native-owned database connections. All query paths share the same read boundary.
use bruhs_models::database::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(crate) mod pgwire;
mod postgres;
mod postgres_policy;
mod sqlite;
#[cfg(test)]
mod tests;

/// Fits inside the native HTTP caller's 20s budget, leaving half for transport/cleanup.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("{0}")]
    Invalid(String),
    #[error("The database connection changed. Refresh it before continuing.")]
    Changed,
    #[error("The saved database file now points to a different target. Edit and save the connection before continuing; agent access will need to be enabled again.")]
    Retargeted,
    #[error("Agent access is off for this database connection. Enable it in Databases first.")]
    Denied,
    #[error("The query was cancelled or exceeded the 10 second query budget.")]
    Cancelled,
    #[error("{0}")]
    Storage(String),
    #[error("Database query failed: {0}")]
    Query(String),
}
impl From<rusqlite::Error> for DatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Query(error.to_string())
    }
}
impl From<std::io::Error> for DatabaseError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DatabaseReader {
    User,
    Agent,
}

struct ActiveQuery {
    connection: Uuid,
    cancelled: Arc<AtomicBool>,
    wake: tokio::sync::watch::Sender<bool>,
}
struct Inner {
    store: Mutex<Connection>,
    secrets: crate::secrets::SecretStore,
    active: Mutex<HashMap<Uuid, ActiveQuery>>,
    // Cancels may overtake query HTTP requests; retain them for the native 20s request budget.
    cancelled_before_start: Mutex<HashMap<Uuid, Instant>>,
}
#[derive(Clone)]
pub struct DatabaseHost(Arc<Inner>);

struct QueryGuard {
    host: DatabaseHost,
    id: Uuid,
    cancelled: Arc<AtomicBool>,
}
impl Drop for QueryGuard {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(mut active) = self.host.0.active.lock() {
            active.remove(&self.id);
        }
    }
}

impl DatabaseHost {
    pub fn default_path() -> Result<PathBuf, DatabaseError> {
        crate::SqliteMeetingStore::default_path()
            .map_err(|error| DatabaseError::Storage(error.to_string()))?
            .parent()
            .map(|parent| parent.join("database-connections.sqlite3"))
            .ok_or_else(|| {
                DatabaseError::Storage("The application data directory is unavailable.".into())
            })
    }
    pub async fn open(
        path: PathBuf,
        secrets: crate::secrets::SecretStore,
    ) -> Result<Self, DatabaseError> {
        let store = tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
            options.open(&path)?;
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?; }
            let store = Connection::open(path)?;
            store.busy_timeout(Duration::from_secs(5))?;
            store.execute_batch("CREATE TABLE IF NOT EXISTS database_connections(id TEXT PRIMARY KEY,payload TEXT NOT NULL)")?;
            Ok::<_,DatabaseError>(store)
        }).await.map_err(|_| DatabaseError::Storage("The connection catalog worker stopped.".into()))??;
        Ok(Self(Arc::new(Inner {
            store: Mutex::new(store),
            secrets,
            active: Mutex::default(),
            cancelled_before_start: Mutex::default(),
        })))
    }
    async fn catalog<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, DatabaseError> + Send + 'static,
    ) -> Result<T, DatabaseError> {
        let host = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = host.0.store.lock().map_err(|_| {
                DatabaseError::Storage(
                    "The connection catalog is unavailable. Restart the server.".into(),
                )
            })?;
            f(&mut store)
        })
        .await
        .map_err(|_| DatabaseError::Storage("The connection catalog worker stopped.".into()))?
    }
    pub async fn list(
        &self,
        project: String,
        reader: DatabaseReader,
    ) -> Result<Vec<DatabaseConnection>, DatabaseError> {
        let project = canonical_project(project).await?;
        self.catalog(move |store| {
            let mut query =
                store.prepare("SELECT payload FROM database_connections ORDER BY id")?;
            let rows = query.query_map([], |row| row.get::<_, String>(0))?;
            let mut result = Vec::new();
            for row in rows {
                let connection: DatabaseConnection = serde_json::from_str(&row?).map_err(|_| {
                    DatabaseError::Storage(
                        "A saved database connection is malformed; original data was retained."
                            .into(),
                    )
                })?;
                if connection.project_path == project
                    && (reader == DatabaseReader::User || connection.agent_access)
                {
                    result.push(connection);
                }
            }
            Ok(result)
        })
        .await
    }
    /// Configuration controls are user-only; they are never registered as model tools.
    pub async fn save(
        &self,
        mut request: SaveDatabase,
    ) -> Result<DatabaseConnection, DatabaseError> {
        request.project_path = canonical_project(request.project_path).await?;
        request.name = request.name.trim().into();
        if request.name.is_empty()
            || request.name.len() > RESULT_BYTES
            || request.name.chars().any(char::is_control)
        {
            return Err(DatabaseError::Invalid(
                "Choose a nonempty connection name without control characters.".into(),
            ));
        }
        request.target = validate_target(request.target).await?;
        let previous = self.get(request.id).await?;
        if previous.as_ref().map(|value| value.revision) != request.expected_revision {
            return Err(DatabaseError::Changed);
        }
        if previous
            .as_ref()
            .is_some_and(|value| value.project_path != request.project_path)
        {
            return Err(DatabaseError::Changed);
        }
        let same_target = previous
            .as_ref()
            .is_some_and(|value| value.target == request.target);
        if request.password.is_some() && !matches!(request.target, DatabaseTarget::Postgres { .. })
        {
            return Err(DatabaseError::Invalid(
                "SQLite connections do not use a saved password.".into(),
            ));
        }
        // Saving the keychain value before the record is intentional: no record ever claims an unsaved value.
        // A new credential name per revision keeps concurrent stale saves from replacing a live password.
        let revision = previous.as_ref().map_or(Ok(1), |value| {
            value.revision.checked_add(1).ok_or(DatabaseError::Changed)
        })?;
        let password_saved = request.password.is_some()
            || (same_target && previous.as_ref().is_some_and(|value| value.password_saved));
        let credential_version = if request.password.is_some() || !same_target {
            Uuid::new_v4()
        } else {
            previous
                .as_ref()
                .map_or_else(Uuid::new_v4, |value| value.credential_version)
        };
        if let Some(password) = request.password.take() {
            self.0.secrets.save(&request.project_path, &password_name(request.id, credential_version), zeroize::Zeroizing::new(password)).await
                .map_err(|_| DatabaseError::Storage("Could not save the database password in the OS credential store. Unlock it and retry.".into()))?;
        }
        let connection = DatabaseConnection {
            id: request.id,
            project_path: request.project_path,
            name: request.name,
            target: request.target,
            revision,
            agent_access: same_target && previous.as_ref().is_some_and(|value| value.agent_access),
            password_saved,
            credential_version,
        };
        let saved = connection.clone();
        self.catalog(move |store| {
            let transaction =
                store.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if load(&transaction, saved.id)?
                .as_ref()
                .map(|value| value.revision)
                != request.expected_revision
            {
                return Err(DatabaseError::Changed);
            }
            persist(&transaction, &saved)?;
            transaction.commit()?;
            Ok(())
        })
        .await?;
        self.cancel_connection(connection.id)?;
        Ok(connection)
    }
    pub async fn grant(&self, request: DatabaseGrant) -> Result<DatabaseConnection, DatabaseError> {
        let connection = self
            .catalog(move |store| {
                let transaction =
                    store.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let mut connection =
                    load(&transaction, request.id)?.ok_or(DatabaseError::Changed)?;
                if connection.revision != request.expected_revision {
                    return Err(DatabaseError::Changed);
                }
                // Revision also binds selected UI results, so old data cannot be attached after a target change.
                // Access alone does not change the credential identity.
                connection.agent_access = request.enabled;
                connection.revision = connection
                    .revision
                    .checked_add(1)
                    .ok_or(DatabaseError::Changed)?;
                persist(&transaction, &connection)?;
                transaction.commit()?;
                Ok(connection)
            })
            .await?;
        if !request.enabled {
            self.cancel_connection(request.id)?;
        }
        Ok(connection)
    }
    pub async fn remove(&self, id: Uuid, revision: u64) -> Result<(), DatabaseError> {
        self.catalog(move |store| {
            let transaction =
                store.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if load(&transaction, id)?.is_none_or(|value| value.revision != revision) {
                return Err(DatabaseError::Changed);
            }
            transaction.execute(
                "DELETE FROM database_connections WHERE id=?1",
                [id.to_string()],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await?;
        self.cancel_connection(id)
    }
    fn cancel_connection(&self, id: Uuid) -> Result<(), DatabaseError> {
        let active = self.0.active.lock().map_err(|_| DatabaseError::Cancelled)?;
        for query in active.values().filter(|query| query.connection == id) {
            query.cancelled.store(true, Ordering::Release);
            let _ = query.wake.send(true);
        }
        drop(active);
        Ok(())
    }
    pub fn cancel(&self, id: Uuid) -> Result<(), DatabaseError> {
        let active = self.0.active.lock().map_err(|_| DatabaseError::Cancelled)?;
        if let Some(query) = active.get(&id) {
            query.cancelled.store(true, Ordering::Release);
            let _ = query.wake.send(true);
        } else {
            let mut pending = self
                .0
                .cancelled_before_start
                .lock()
                .map_err(|_| DatabaseError::Cancelled)?;
            pending.retain(|_, created| created.elapsed() < QUERY_TIMEOUT * 2);
            // UUIDs consume 16 bytes; share the 32 KiB capacity contract of a result page.
            if pending.len() >= RESULT_BYTES / 16 {
                return Err(DatabaseError::Invalid("The cancellation queue is full. Wait for pending requests to expire and retry.".into()));
            }
            pending.insert(id, Instant::now());
        }
        drop(active);
        Ok(())
    }
    async fn get(&self, id: Uuid) -> Result<Option<DatabaseConnection>, DatabaseError> {
        self.catalog(move |store| load(store, id)).await
    }
    async fn require(
        &self,
        project: &str,
        id: Uuid,
        revision: u64,
        reader: DatabaseReader,
    ) -> Result<DatabaseConnection, DatabaseError> {
        let value = self
            .get(id)
            .await?
            .filter(|value| value.project_path == project && value.revision == revision)
            .ok_or(DatabaseError::Changed)?;
        if reader == DatabaseReader::Agent && !value.agent_access {
            return Err(DatabaseError::Denied);
        }
        Ok(value)
    }
    pub async fn query(
        &self,
        project: String,
        query: DatabaseQuery,
        reader: DatabaseReader,
    ) -> Result<DatabaseResult, DatabaseError> {
        self.query_inner(project, query, reader, false).await
    }
    async fn query_inner(
        &self,
        project: String,
        query: DatabaseQuery,
        reader: DatabaseReader,
        metadata: bool,
    ) -> Result<DatabaseResult, DatabaseError> {
        let project = canonical_project(project).await?;
        if query.sql.trim().is_empty() || query.sql.len() > RESULT_BYTES {
            return Err(DatabaseError::Invalid(
                "Enter one read-only query up to 32 KiB.".into(),
            ));
        }
        let connection = self
            .require(&project, query.id, query.revision, reader)
            .await?;
        if validate_target(connection.target.clone()).await? != connection.target {
            return Err(DatabaseError::Retargeted);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let (wake, mut revoked) = tokio::sync::watch::channel(false);
        {
            let mut active = self.0.active.lock().map_err(|_| DatabaseError::Cancelled)?;
            let mut pending = self
                .0
                .cancelled_before_start
                .lock()
                .map_err(|_| DatabaseError::Cancelled)?;
            pending.retain(|_, created| created.elapsed() < QUERY_TIMEOUT * 2);
            if pending.remove(&query.request_id).is_some() {
                return Err(DatabaseError::Cancelled);
            }
            drop(pending);
            if active.contains_key(&query.request_id) {
                return Err(DatabaseError::Invalid(
                    "This query is already running. Cancel it before retrying.".into(),
                ));
            }
            active.insert(
                query.request_id,
                ActiveQuery {
                    connection: query.id,
                    cancelled: Arc::clone(&cancelled),
                    wake,
                },
            );
            drop(active);
        }
        let _guard = QueryGuard {
            host: self.clone(),
            id: query.request_id,
            cancelled: Arc::clone(&cancelled),
        };
        self.require(&project, query.id, query.revision, reader)
            .await?;
        let started = Instant::now();
        let execution = async {
            Ok::<_, DatabaseError>(match &connection.target {
                DatabaseTarget::Sqlite { path } => {
                    sqlite::query(path.clone(), query.sql, Arc::clone(&cancelled), metadata).await?
                }
                DatabaseTarget::Postgres { .. } => {
                    postgres::query(
                        &self.0.secrets,
                        &connection,
                        &query.sql,
                        Arc::clone(&cancelled),
                        metadata,
                    )
                    .await?
                }
            })
        };
        let mut result = tokio::select! {biased; _=revoked.changed()=>return Err(DatabaseError::Cancelled), result=execution=>result?};
        if cancelled.load(Ordering::Acquire) {
            return Err(DatabaseError::Cancelled);
        }
        self.require(&project, query.id, query.revision, reader)
            .await?;
        result.connection_id = connection.id;
        result.revision = connection.revision;
        result.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(result)
    }
    pub async fn tables(
        &self,
        project: String,
        id: Uuid,
        revision: u64,
        reader: DatabaseReader,
    ) -> Result<Vec<DatabaseTable>, DatabaseError> {
        let canonical = canonical_project(project).await?;
        let connection = self.require(&canonical, id, revision, reader).await?;
        let sql = match connection.target {
            DatabaseTarget::Sqlite {..} => "SELECT 'main',name,type FROM sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
            DatabaseTarget::Postgres {..} => "SELECT table_schema,table_name,table_type FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema,table_name",
        };
        let result = self
            .query_inner(
                canonical,
                DatabaseQuery {
                    id,
                    revision,
                    sql: sql.into(),
                    request_id: Uuid::new_v4(),
                },
                reader,
                true,
            )
            .await?;
        if result.truncated {
            return Err(DatabaseError::Invalid(
                "The schema exceeds one result page. Use a filtered schema query to inspect it."
                    .into(),
            ));
        }
        result
            .rows
            .into_iter()
            .map(|row| {
                let mut cells = row.into_iter();
                Ok(DatabaseTable {
                    schema: cells.next().flatten().ok_or(DatabaseError::Changed)?,
                    name: cells.next().flatten().ok_or(DatabaseError::Changed)?,
                    kind: cells.next().flatten().ok_or(DatabaseError::Changed)?,
                })
            })
            .collect()
    }
    pub async fn describe(
        &self,
        project: String,
        request: BrowseDatabase,
        reader: DatabaseReader,
    ) -> Result<DatabaseResult, DatabaseError> {
        let connection = self
            .require(
                &canonical_project(project.clone()).await?,
                request.id,
                request.revision,
                reader,
            )
            .await?;
        let sql = describe_table_sql(
            &connection.target,
            &DatabaseTable {
                schema: request.schema,
                name: request.table,
                kind: String::new(),
            },
        );
        self.query_inner(
            project,
            DatabaseQuery {
                id: request.id,
                revision: request.revision,
                sql,
                request_id: request.request_id,
            },
            reader,
            true,
        )
        .await
    }
    pub async fn browse(
        &self,
        project: String,
        request: BrowseDatabase,
        reader: DatabaseReader,
    ) -> Result<DatabaseResult, DatabaseError> {
        let connection = self
            .require(
                &canonical_project(project.clone()).await?,
                request.id,
                request.revision,
                reader,
            )
            .await?;
        if !request.filter.is_empty() && request.filter_column.as_ref().is_none_or(String::is_empty)
        {
            return Err(DatabaseError::Invalid(
                "Choose a filter column before filtering rows.".into(),
            ));
        }
        let quote = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
        if request.schema.is_empty()
            || request.table.is_empty()
            || request.schema.contains('\0')
            || request.table.contains('\0')
        {
            return Err(DatabaseError::Invalid("Choose a schema and table.".into()));
        }
        let mut sql = format!(
            "SELECT * FROM {}.{}",
            quote(&request.schema),
            quote(&request.table)
        );
        if let Some(column) = request.filter_column.filter(|_| !request.filter.is_empty()) {
            let cast = match connection.target {
                DatabaseTarget::Sqlite { .. } => "TEXT",
                DatabaseTarget::Postgres { .. } => "text",
            };
            // Both identifiers and literals are encoded separately; no user SQL fragment is interpolated.
            let literal = request.filter.replace('\'', "''");
            sql.push_str(&format!(
                " WHERE position('{}' in CAST({} AS {cast})) > 0",
                literal,
                quote(&column)
            ));
            if matches!(connection.target, DatabaseTarget::Sqlite { .. }) {
                sql = format!(
                    "SELECT * FROM {}.{} WHERE instr(CAST({} AS TEXT),'{}') > 0",
                    quote(&request.schema),
                    quote(&request.table),
                    quote(&column),
                    literal
                );
            }
        }
        if let Some(column) = request.sort_column {
            sql.push_str(&format!(
                " ORDER BY {} {}",
                quote(&column),
                if request.descending { "DESC" } else { "ASC" }
            ));
        }
        sql.push_str(&format!(
            " LIMIT {} OFFSET {}",
            ROW_LIMIT + 1,
            request.offset
        ));
        let mut result = self
            .query(
                project,
                DatabaseQuery {
                    id: request.id,
                    revision: request.revision,
                    sql,
                    request_id: request.request_id,
                },
                reader,
            )
            .await?;
        result.next_offset = if result.truncated && !result.rows.is_empty() {
            request
                .offset
                .checked_add(u64::try_from(result.rows.len()).map_err(|_| DatabaseError::Changed)?)
        } else {
            None
        };
        Ok(result)
    }
}

pub(crate) fn password_name(id: Uuid, version: Uuid) -> String {
    format!("database/{id}/{version}")
}
async fn canonical_project(project: String) -> Result<String, DatabaseError> {
    tokio::task::spawn_blocking(move || {
        let path = std::fs::canonicalize(project)
            .map_err(|_| DatabaseError::Invalid("Choose an existing project directory.".into()))?;
        if !path.is_dir() {
            return Err(DatabaseError::Invalid(
                "Choose an existing project directory.".into(),
            ));
        }
        path.to_str()
            .map(str::to_string)
            .ok_or_else(|| DatabaseError::Invalid("The project path is not valid UTF-8.".into()))
    })
    .await
    .map_err(|_| DatabaseError::Cancelled)?
}
async fn validate_target(target: DatabaseTarget) -> Result<DatabaseTarget, DatabaseError> {
    match target {
        DatabaseTarget::Sqlite { path } => tokio::task::spawn_blocking(move || {
            // No SQLite URI interpretation: canonical paths cannot inject open flags or another database.
            let path = std::fs::canonicalize(path)
                .map_err(|_| DatabaseError::Invalid("Choose an existing SQLite file.".into()))?;
            let private=directories::ProjectDirs::from("dev","bruhs","bruhs-ai")
                .ok_or_else(||DatabaseError::Storage("Could not resolve the private application data directory.".into()))?;
            let private=std::fs::canonicalize(private.data_local_dir()).unwrap_or_else(|_|private.data_local_dir().to_path_buf());
            let meeting=crate::SqliteMeetingStore::default_path().map_err(|error|DatabaseError::Storage(error.to_string()))?;
            if path.starts_with(private) || std::fs::canonicalize(meeting).is_ok_and(|meeting|path==meeting){
                return Err(DatabaseError::Invalid("Use Bruhs' existing transcript, people and workspace views for private app data. The database browser cannot open the app's private databases or bypass their sharing permissions.".into()));
            }
            if !path.is_file() {
                return Err(DatabaseError::Invalid(
                    "Choose an existing SQLite file.".into(),
                ));
            }
            Ok(DatabaseTarget::Sqlite {
                path: path
                    .to_str()
                    .ok_or_else(|| {
                        DatabaseError::Invalid("The SQLite path is not valid UTF-8.".into())
                    })?
                    .into(),
            })
        })
        .await
        .map_err(|_| DatabaseError::Cancelled)?,
        DatabaseTarget::Postgres {
            host,
            port,
            database,
            user,
        } => {
            let host = host.trim().to_string();
            if port == 0
                || !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1")
                || database.is_empty()
                || user.is_empty()
                || database.chars().chain(user.chars()).any(char::is_control)
            {
                return Err(DatabaseError::Invalid("Use a local Postgres host (localhost, 127.0.0.1 or ::1), port, database and user. Remote databases are outside this version.".into()));
            }
            Ok(DatabaseTarget::Postgres {
                host,
                port,
                database,
                user,
            })
        }
    }
}
fn load(store: &Connection, id: Uuid) -> Result<Option<DatabaseConnection>, DatabaseError> {
    let payload: Option<String> = store
        .query_row(
            "SELECT payload FROM database_connections WHERE id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            serde_json::from_str(&payload).map_err(|_| {
                DatabaseError::Storage(
                    "A saved database connection is malformed; original data was retained.".into(),
                )
            })
        })
        .transpose()
}
fn persist(store: &Connection, value: &DatabaseConnection) -> Result<(), DatabaseError> {
    store.execute("INSERT INTO database_connections(id,payload) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![value.id.to_string(),serde_json::to_string(value).map_err(|_| DatabaseError::Changed)?])?;
    Ok(())
}
fn empty_result() -> DatabaseResult {
    DatabaseResult {
        connection_id: Uuid::nil(),
        revision: 0,
        columns: Vec::new(),
        rows: Vec::new(),
        truncated: false,
        elapsed_ms: 0,
        next_offset: None,
    }
}
fn push_row(
    result: &mut DatabaseResult,
    row: Vec<Option<String>>,
    metadata: bool,
) -> Result<bool, DatabaseError> {
    result.rows.push(row);
    if result.rows.len()
        > if metadata {
            RESULT_BYTES / 2
        } else {
            ROW_LIMIT
        }
        || serde_json::to_vec(result)
            .map_err(|_| DatabaseError::Changed)?
            .len()
            + 64
            > RESULT_BYTES
    {
        result.rows.pop();
        result.truncated = true;
        return Ok(false);
    }
    Ok(true)
}
