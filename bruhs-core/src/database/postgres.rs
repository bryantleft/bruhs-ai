use super::*;
use futures_util::{pin_mut, StreamExt};
use tokio_postgres::SimpleQueryMessage;

pub(super) async fn query(
    secrets: &crate::secrets::SecretStore,
    connection: &DatabaseConnection,
    sql: &str,
    cancelled: Arc<AtomicBool>,
    metadata: bool,
) -> Result<DatabaseResult, DatabaseError> {
    let checked = if metadata {
        None
    } else {
        Some(super::postgres_policy::check(sql)?)
    };
    let run = async {
        let database = secrets
            .database_client(connection, Arc::clone(&cancelled))
            .await?;
        let client = &database.client;
        let map_error = |error: tokio_postgres::Error| {
            DatabaseError::Query(error.as_db_error().map_or_else(||"Postgres connection closed or a value exceeded the result budget. Select smaller values and retry.".into(),|error|format!("{} ({})",error.message(),error.code().code())))
        };
        // Lock the transaction's access mode with a trusted first query. SET/COMMIT are also rejected
        // below because the single prepared statement must return columns.
        client.batch_execute("BEGIN READ ONLY; SET LOCAL statement_timeout = '10s'; SET LOCAL search_path = pg_catalog; SET LOCAL constraint_exclusion = off; SELECT 1;").await.map_err(map_error)?;
        if let Some(checked) = &checked {
            for (schema, relation) in &checked.relations {
                let quote = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
                // Hold the relation lock through validation, preparation and execution. ALTER cannot
                // replace a checked column, type, RLS policy, or relation while this query reads it.
                client
                    .batch_execute(&format!(
                        "LOCK TABLE ONLY {}.{} IN ACCESS SHARE MODE",
                        quote(schema),
                        quote(relation)
                    ))
                    .await
                    .map_err(map_error)?;
                let row=client.query_one("SELECT c.relkind='r' AND a.amname='heap' AND NOT c.relrowsecurity AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhparent=c.oid OR i.inhrelid=c.oid) AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_attribute col WHERE col.attrelid=c.oid AND col.attnum>0 AND NOT col.attisdropped AND (col.attgenerated='v' OR col.atttypid NOT IN (16,17,20,21,23,25,26,114,700,701,1042,1043,1082,1083,1114,1184,1186,1266,1700,2950,3802,1000,1001,1005,1007,1009,1014,1015,1016,1021,1022,1115,1182,1183,1185,1187,1231,1270,199,2951,3807))) AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_index ix JOIN pg_catalog.pg_class ic ON ic.oid=ix.indexrelid JOIN pg_catalog.pg_am ia ON ia.oid=ic.relam WHERE ix.indrelid=c.oid AND (ix.indexprs IS NOT NULL OR ix.indpred IS NOT NULL OR ia.amname NOT IN ('btree','hash','gin','gist','spgist','brin') OR EXISTS(SELECT 1 FROM pg_catalog.unnest(ix.indclass::oid[]) o JOIN pg_catalog.pg_opclass op ON op.oid=o JOIN pg_catalog.pg_namespace ons ON ons.oid=op.opcnamespace WHERE ons.nspname<>'pg_catalog'))) AS allowed FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_am a ON a.oid=c.relam WHERE n.nspname=$1 AND c.relname=$2",&[schema,relation]).await.map_err(map_error)?;
                if !row.get::<_, bool>(0) {
                    return Err(DatabaseError::Invalid("This relation uses a view, RLS, inheritance, a custom type/access method, virtual column or expression/partial index. Those can execute hidden code and are outside this read-only browser's supported table subset.".into()));
                }
            }
        }
        let sql = checked.as_ref().map_or(sql, |checked| checked.sql.as_str());
        let statement = client.prepare(sql).await.map_err(map_error)?;
        if statement.columns().is_empty() {
            return Err(DatabaseError::Invalid(
                "Only a single read-only statement returning rows is allowed.".into(),
            ));
        }
        let mut result = empty_result();
        result.columns = statement
            .columns()
            .iter()
            .map(|column| DatabaseColumn {
                name: column.name().into(),
                data_type: column.type_().name().into(),
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
        // prepare() rejects multi-statement input on the server. The exact same SQL is now streamed
        // as text so numeric/decimal/date values retain their original precision and representation.
        let stream = client.simple_query_raw(sql).await.map_err(map_error)?;
        pin_mut!(stream);
        while let Some(message) = stream.next().await {
            if cancelled.load(Ordering::Acquire) {
                return Err(DatabaseError::Cancelled);
            }
            if let SimpleQueryMessage::Row(row) = message.map_err(map_error)? {
                let cells = (0..row.len())
                    .map(|index| row.try_get(index).map(|value| value.map(str::to_string)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(map_error)?;
                if !push_row(&mut result, cells, metadata)? {
                    break;
                }
            }
        }
        // Dropping this dedicated connection rolls back even on truncation, cancellation, or errors.
        Ok(result)
    };
    tokio::time::timeout(QUERY_TIMEOUT, run)
        .await
        .map_err(|_| DatabaseError::Cancelled)?
}
