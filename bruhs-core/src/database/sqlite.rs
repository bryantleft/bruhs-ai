use super::*;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::ValueRef,
    OpenFlags,
};

pub(super) async fn query(
    path: String,
    sql: String,
    cancelled: Arc<AtomicBool>,
    metadata: bool,
) -> Result<DatabaseResult, DatabaseError> {
    tokio::task::spawn_blocking(move || run(&path, &sql, cancelled, metadata))
        .await
        .map_err(|_| DatabaseError::Cancelled)?
}

fn run(
    path: &str,
    sql: &str,
    cancelled: Arc<AtomicBool>,
    metadata: bool,
) -> Result<DatabaseResult, DatabaseError> {
    let mut connection = Connection::open_with_flags(
        Path::new(path),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(QUERY_TIMEOUT)?;
    connection.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DQS_DML, false)?;
    connection.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DQS_DDL, false)?;
    connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    let byte_limit = i32::try_from(RESULT_BYTES).map_err(|_| DatabaseError::Changed)?;
    // Per-connection limits avoid changing SQLite's global allocator used by meeting storage.
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, byte_limit)?;
    connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, byte_limit)?;
    let started = Instant::now();
    let signal = Arc::clone(&cancelled);
    connection.progress_handler(
        1,
        Some(move || signal.load(Ordering::Acquire) || started.elapsed() >= QUERY_TIMEOUT),
    )?;
    connection.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Function { function_name }
            if !matches!(
                function_name.to_ascii_lowercase().as_str(),
                "load_extension" | "writefile" | "readfile" | "eval"
            ) =>
        {
            Authorization::Allow
        }
        // Metadata-only pragmas are useful in SQL and power the schema inspector. Every other pragma fails closed.
        AuthAction::Pragma { pragma_name, .. }
            if matches!(
                pragma_name.to_ascii_lowercase().as_str(),
                "table_info" | "table_xinfo" | "index_list" | "index_info" | "foreign_key_list"
            ) =>
        {
            Authorization::Allow
        }
        _ => Authorization::Deny,
    }))?;
    let outcome = (|| {
        let mut statement = connection.prepare(sql)?;
        if !statement.readonly() || statement.column_count() == 0 {
            return Err(DatabaseError::Invalid(
                "Only a single read-only statement returning rows is allowed.".into(),
            ));
        }
        let mut result = empty_result();
        result.columns = statement
            .column_names()
            .map(|name| DatabaseColumn {
                name: name.into(),
                data_type: "SQLite value".into(),
            })
            .collect();
        if serde_json::to_vec(&result)
            .map_err(|_| DatabaseError::Changed)?
            .len()
            + 64
            > RESULT_BYTES
        {
            return Err(DatabaseError::Invalid(
                "The result columns exceed the 32 KiB result budget. Select fewer columns.".into(),
            ));
        }
        let count = statement.column_count();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            if cancelled.load(Ordering::Acquire) || started.elapsed() >= QUERY_TIMEOUT {
                return Err(DatabaseError::Cancelled);
            }
            let cells = (0..count)
                .map(|index| {
                    Ok(match row.get_ref(index)? {
                        ValueRef::Null => None,
                        ValueRef::Integer(value) => Some(value.to_string()),
                        ValueRef::Real(value) => Some(value.to_string()),
                        ValueRef::Text(value) => Some(String::from_utf8_lossy(value).into_owned()),
                        ValueRef::Blob(value) => Some(format!(
                            "0x{}",
                            value
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>()
                        )),
                    })
                })
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            if !push_row(&mut result, cells, metadata)? {
                break;
            }
        }
        Ok(result)
    })();
    if cancelled.load(Ordering::Acquire) || started.elapsed() >= QUERY_TIMEOUT {
        Err(DatabaseError::Cancelled)
    } else {
        outcome
    }
}
