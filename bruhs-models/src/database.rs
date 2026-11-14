//! Read-only database browsing. Passwords and query history are never saved here.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Half the existing 64 KiB MCP frame leaves room for the tool envelope.
pub const RESULT_BYTES: usize = 32 * 1024;
/// Matches the app's bounded history page; subsequent table pages use an offset.
pub const ROW_LIMIT: usize = crate::reactive::HISTORY_LIMIT;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DatabaseTarget {
    Sqlite {
        path: String,
    },
    Postgres {
        host: String,
        port: u16,
        database: String,
        user: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConnection {
    pub id: Uuid,
    pub project_path: String,
    pub name: String,
    pub target: DatabaseTarget,
    /// Changes invalidate stale queries, results, and sharing controls.
    pub revision: u64,
    pub agent_access: bool,
    pub password_saved: bool,
    /// Opaque keychain version; never a credential value.
    pub credential_version: Uuid,
}

/// Input only. It deliberately has no Debug implementation and is never returned.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDatabase {
    pub id: Uuid,
    pub project_path: String,
    pub name: String,
    pub target: DatabaseTarget,
    pub expected_revision: Option<u64>,
    /// None retains the saved password only when the target is unchanged.
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseGrant {
    pub id: Uuid,
    pub expected_revision: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseQuery {
    pub id: Uuid,
    pub revision: u64,
    pub sql: String,
    pub request_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDatabaseQuery {
    pub project_path: String,
    pub query: DatabaseQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDatabaseBrowse {
    pub project_path: String,
    pub browse: BrowseDatabase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseColumn {
    pub name: String,
    pub data_type: String,
}

/// Text preserves decimal/int64 precision. NULL stays distinct from an empty string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseResult {
    pub connection_id: Uuid,
    pub revision: u64,
    pub columns: Vec<DatabaseColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated: bool,
    pub elapsed_ms: u64,
    /// Table browsing continues after the rows actually returned, including byte-limited pages.
    #[serde(default)]
    pub next_offset: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseTable {
    pub schema: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowseDatabase {
    #[serde(default = "Uuid::new_v4")]
    pub request_id: Uuid,
    pub id: Uuid,
    pub revision: u64,
    pub schema: String,
    pub table: String,
    pub offset: u64,
    pub sort_column: Option<String>,
    pub descending: bool,
    pub filter_column: Option<String>,
    pub filter: String,
}

pub fn browse_criteria_match(
    request: &BrowseDatabase,
    filter_column: &str,
    filter: &str,
    sort_column: &str,
    descending: bool,
) -> bool {
    request.filter_column.as_deref().unwrap_or("") == filter_column
        && request.filter == filter
        && request.sort_column.as_deref().unwrap_or("") == sort_column
        && request.descending == descending
}

/// Explicit selection for an unsent composer attachment; never includes hidden rows.
pub fn selected_result_text(
    result: &DatabaseResult,
    selected: &[usize],
) -> Result<String, &'static str> {
    if selected.is_empty() {
        return Err("Select at least one result row.");
    }
    let rows: Vec<_> = selected
        .iter()
        .map(|index| {
            result
                .rows
                .get(*index)
                .cloned()
                .ok_or("This result changed. Select its rows again.")
        })
        .collect::<Result<_, _>>()?;
    let json = serde_json::to_string_pretty(&serde_json::json!({"connection_id":result.connection_id,"columns":result.columns,"rows":rows,"source_truncated":result.truncated})).map_err(|_| "Could not prepare the selected result.")?;
    Ok(format!(
        "Selected database results (read-only, explicitly attached):\n```json\n{json}\n```"
    ))
}

pub fn describe_table_sql(target: &DatabaseTarget, table: &DatabaseTable) -> String {
    let literal = |value: &str| value.replace('\'', "''");
    match target {
        DatabaseTarget::Sqlite{..}=>format!("PRAGMA \"{}\".table_info('{}')",table.schema.replace('"',"\"\""),literal(&table.name)),
        DatabaseTarget::Postgres{..}=>format!("SELECT column_name,data_type,is_nullable FROM information_schema.columns WHERE table_schema='{}' AND table_name='{}' ORDER BY ordinal_position",literal(&table.schema),literal(&table.name)),
    }
}
