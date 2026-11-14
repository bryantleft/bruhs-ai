use super::*;

async fn fixture() -> (tempfile::TempDir, DatabaseHost, DatabaseConnection) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.sqlite3");
    let database = Connection::open(&path).unwrap();
    database.execute_batch("CREATE TABLE customers(id INTEGER PRIMARY KEY, name TEXT, subscription TEXT); INSERT INTO customers VALUES(1,'Ada',NULL),(2,'Lin','pro');").unwrap();
    drop(database);
    let host = DatabaseHost::open(
        directory.path().join("catalog.sqlite3"),
        crate::secrets::SecretStore::in_memory(),
    )
    .await
    .unwrap();
    let saved = host
        .save(SaveDatabase {
            id: Uuid::new_v4(),
            project_path: directory.path().display().to_string(),
            name: "Fixture".into(),
            target: DatabaseTarget::Sqlite {
                path: path.display().to_string(),
            },
            expected_revision: None,
            password: None,
        })
        .await
        .unwrap();
    (directory, host, saved)
}
fn request(connection: &DatabaseConnection, sql: &str) -> DatabaseQuery {
    DatabaseQuery {
        id: connection.id,
        revision: connection.revision,
        sql: sql.into(),
        request_id: Uuid::new_v4(),
    }
}

#[tokio::test]
async fn sqlite_blocks_writes_and_attaches_without_changing_file() {
    let (_directory, host, connection) = fixture().await;
    let DatabaseTarget::Sqlite { path } = &connection.target else {
        panic!()
    };
    let before = std::fs::read(path).unwrap();
    for sql in [
        "INSERT INTO customers VALUES(3,'bad','x') RETURNING *",
        "UPDATE customers SET subscription='x' RETURNING *",
        "DELETE FROM customers RETURNING *",
        "DROP TABLE customers",
        "CREATE TEMP TABLE stolen(x)",
        "PRAGMA query_only=OFF",
        "ATTACH DATABASE ':memory:' AS other",
        "VACUUM",
        "SELECT load_extension('bad')",
        "SELECT 1; UPDATE customers SET name='bad'",
        "WITH x AS (SELECT 1) UPDATE customers SET name='bad' RETURNING *",
    ] {
        assert!(
            host.query(
                connection.project_path.clone(),
                request(&connection, sql),
                DatabaseReader::User
            )
            .await
            .is_err(),
            "allowed {sql}"
        );
    }
    let result = host
        .query(
            connection.project_path.clone(),
            request(
                &connection,
                "SELECT id,name,subscription FROM customers ORDER BY id",
            ),
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert_eq!(
        result.rows,
        vec![
            vec![Some("1".into()), Some("Ada".into()), None],
            vec![Some("2".into()), Some("Lin".into()), Some("pro".into())]
        ]
    );
    assert_eq!(before, std::fs::read(path).unwrap());
    assert_eq!(
        host.tables(
            connection.project_path.clone(),
            connection.id,
            connection.revision,
            DatabaseReader::User
        )
        .await
        .unwrap()[0]
            .name,
        "customers"
    );
    let metadata = host
        .query(
            connection.project_path.clone(),
            request(&connection, "PRAGMA table_info('customers')"),
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert_eq!(metadata.rows.len(), 3);
}

#[tokio::test]
async fn agent_opt_in_is_per_connection_and_target_changes_revoke_it() {
    let (directory, host, connection) = fixture().await;
    assert!(host
        .list(connection.project_path.clone(), DatabaseReader::Agent)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        host.query(
            connection.project_path.clone(),
            request(&connection, "SELECT * FROM customers"),
            DatabaseReader::Agent
        )
        .await,
        Err(DatabaseError::Denied)
    ));
    let connection = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    assert_eq!(
        host.query(
            connection.project_path.clone(),
            request(&connection, "SELECT * FROM customers"),
            DatabaseReader::Agent
        )
        .await
        .unwrap()
        .rows
        .len(),
        2
    );
    let next = directory.path().join("other.sqlite3");
    Connection::open(&next)
        .unwrap()
        .execute_batch("CREATE TABLE other(x)")
        .unwrap();
    let changed = host
        .save(SaveDatabase {
            id: connection.id,
            project_path: connection.project_path.clone(),
            name: connection.name.clone(),
            target: DatabaseTarget::Sqlite {
                path: next.display().to_string(),
            },
            expected_revision: Some(connection.revision),
            password: None,
        })
        .await
        .unwrap();
    assert!(!changed.agent_access);
    assert!(matches!(
        host.query(
            connection.project_path.clone(),
            request(&connection, "SELECT * FROM customers"),
            DatabaseReader::User
        )
        .await,
        Err(DatabaseError::Changed)
    ));
    assert!(matches!(
        host.query(
            changed.project_path.clone(),
            request(&changed, "SELECT * FROM other"),
            DatabaseReader::Agent
        )
        .await,
        Err(DatabaseError::Denied)
    ));
    let reopened = DatabaseHost::open(
        directory.path().join("catalog.sqlite3"),
        crate::secrets::SecretStore::in_memory(),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened
            .list(changed.project_path.clone(), DatabaseReader::User)
            .await
            .unwrap(),
        vec![changed]
    );
}

#[tokio::test]
async fn browse_quotes_identifiers_and_filter_and_selection_is_explicit() {
    let (_directory, host, connection) = fixture().await;
    let mut query = BrowseDatabase {
        request_id: uuid::Uuid::new_v4(),
        id: connection.id,
        revision: connection.revision,
        schema: "main".into(),
        table: "customers".into(),
        offset: 0,
        sort_column: Some("id".into()),
        descending: true,
        filter_column: Some("name".into()),
        filter: "Ada".into(),
    };
    let result = host
        .browse(
            connection.project_path.clone(),
            query.clone(),
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][1].as_deref(), Some("Ada"));
    query.filter = "' OR 1=1 --".into();
    assert!(host
        .browse(connection.project_path.clone(), query, DatabaseReader::User)
        .await
        .unwrap()
        .rows
        .is_empty());
    let mut invalid = BrowseDatabase {
        request_id: Uuid::new_v4(),
        id: connection.id,
        revision: connection.revision,
        schema: "main".into(),
        table: "customers".into(),
        offset: 0,
        sort_column: None,
        descending: false,
        filter_column: Some("missing".into()),
        filter: "missing".into(),
    };
    assert!(host
        .browse(
            connection.project_path.clone(),
            invalid.clone(),
            DatabaseReader::User
        )
        .await
        .is_err());
    invalid.filter.clear();
    invalid.filter_column = None;
    invalid.sort_column = Some("missing".into());
    invalid.request_id = Uuid::new_v4();
    assert!(host
        .browse(
            connection.project_path.clone(),
            invalid,
            DatabaseReader::User
        )
        .await
        .is_err());
    let text = selected_result_text(&result, &[0]).unwrap();
    assert!(text.contains("Ada"));
    assert!(!text.contains("Lin"));
    assert!(selected_result_text(&result, &[]).is_err());
    assert!(selected_result_text(&result, &[10]).is_err());
}

#[tokio::test]
async fn query_budget_and_cancellation_bound_work() {
    let (_directory, host, connection) = fixture().await;
    let result=host.query(connection.project_path.clone(),request(&connection,"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT x FROM n"),DatabaseReader::User).await.unwrap();
    assert!(result.truncated);
    assert_eq!(result.rows.len(), ROW_LIMIT);
    assert!(serde_json::to_vec(&result).unwrap().len() <= RESULT_BYTES);
    assert!(host
        .query(
            connection.project_path.clone(),
            request(&connection, "SELECT zeroblob(1000000000)"),
            DatabaseReader::User
        )
        .await
        .is_err());
    let connection = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    let query=request(&connection,"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n");
    let h = host.clone();
    let c = connection.clone();
    let task =
        tokio::spawn(async move { h.query(c.project_path, query, DatabaseReader::Agent).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !host.0.active.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let _revoked = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: false,
        })
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}

#[tokio::test]
async fn project_isolation_and_stale_configuration_fail_closed() {
    let (_directory, host, connection) = fixture().await;
    let other = tempfile::tempdir().unwrap();
    assert!(host
        .query(
            other.path().display().to_string(),
            request(&connection, "SELECT 1"),
            DatabaseReader::Agent
        )
        .await
        .is_err());
    assert!(host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: 0,
            enabled: true
        })
        .await
        .is_err());
    assert!(host
        .save(SaveDatabase {
            id: connection.id,
            project_path: connection.project_path.clone(),
            name: "stale".into(),
            target: connection.target.clone(),
            expected_revision: None,
            password: None
        })
        .await
        .is_err());
}

#[tokio::test]
async fn byte_limited_table_pages_continue_without_skipping_and_cancel_can_arrive_first() {
    let (_directory, host, connection) = fixture().await;
    let DatabaseTarget::Sqlite { path } = &connection.target else {
        panic!()
    };
    let raw = Connection::open(path).unwrap();
    raw.execute_batch("CREATE TABLE wide(id INTEGER PRIMARY KEY,body TEXT); WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<80) INSERT INTO wide SELECT x,printf('%02000d',x) FROM n;").unwrap();
    drop(raw);
    let mut browse = BrowseDatabase {
        request_id: Uuid::new_v4(),
        id: connection.id,
        revision: connection.revision,
        schema: "main".into(),
        table: "wide".into(),
        offset: 0,
        sort_column: Some("id".into()),
        descending: false,
        filter_column: None,
        filter: String::new(),
    };
    let mut ids = Vec::new();
    loop {
        browse.request_id = Uuid::new_v4();
        let result = host
            .browse(
                connection.project_path.clone(),
                browse.clone(),
                DatabaseReader::User,
            )
            .await
            .unwrap();
        assert!(result.rows.len() < ROW_LIMIT);
        assert!(serde_json::to_vec(&result).unwrap().len() <= RESULT_BYTES);
        ids.extend(
            result
                .rows
                .iter()
                .map(|row| row[0].as_ref().unwrap().parse::<i64>().unwrap()),
        );
        if let Some(offset) = result.next_offset {
            assert_eq!(
                offset,
                browse.offset + u64::try_from(result.rows.len()).unwrap()
            );
            browse.offset = offset;
        } else {
            break;
        }
    }
    assert_eq!(ids, (1..=80).collect::<Vec<_>>());
    let query = request(&connection, "SELECT * FROM customers");
    host.cancel(query.request_id).unwrap();
    assert!(matches!(
        host.query(connection.project_path.clone(), query, DatabaseReader::User)
            .await,
        Err(DatabaseError::Cancelled)
    ));
}

#[tokio::test]
async fn a_successful_revoke_invalidates_an_older_save() {
    let (_directory, host, connection) = fixture().await;
    let allowed = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    let revoked = host
        .grant(DatabaseGrant {
            id: allowed.id,
            expected_revision: allowed.revision,
            enabled: false,
        })
        .await
        .unwrap();
    assert!(revoked.revision > allowed.revision);
    assert!(matches!(
        host.save(SaveDatabase {
            id: allowed.id,
            project_path: allowed.project_path.clone(),
            name: "Older window".into(),
            target: allowed.target,
            expected_revision: Some(allowed.revision),
            password: None
        })
        .await,
        Err(DatabaseError::Changed)
    ));
    assert!(
        !host
            .list(revoked.project_path, DatabaseReader::User)
            .await
            .unwrap()[0]
            .agent_access
    );
}

#[tokio::test]
async fn schema_listing_is_not_limited_to_a_data_grid_page() {
    let (_directory, host, connection) = fixture().await;
    let DatabaseTarget::Sqlite { path } = &connection.target else {
        panic!()
    };
    let raw = Connection::open(path).unwrap();
    for index in 0..ROW_LIMIT + 1 {
        raw.execute_batch(&format!("CREATE TABLE synthetic_{index}(id INTEGER)"))
            .unwrap();
    }
    drop(raw);
    assert_eq!(
        host.tables(
            connection.project_path.clone(),
            connection.id,
            connection.revision,
            DatabaseReader::User
        )
        .await
        .unwrap()
        .len(),
        ROW_LIMIT + 2
    );
}

#[cfg(unix)]
#[tokio::test]
async fn saved_file_cannot_silently_retarget_an_existing_agent_grant() {
    let (directory, host, connection) = fixture().await;
    let connection = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    let DatabaseTarget::Sqlite { path } = &connection.target else {
        panic!()
    };
    let other = directory.path().join("not-shared.sqlite3");
    let raw = Connection::open(&other).unwrap();
    raw.execute_batch("CREATE TABLE not_shared(value TEXT); INSERT INTO not_shared VALUES('separate synthetic dataset');").unwrap();
    drop(raw);
    std::fs::rename(path, directory.path().join("original.sqlite3")).unwrap();
    std::os::unix::fs::symlink(other, path).unwrap();
    assert!(matches!(
        host.query(
            connection.project_path.clone(),
            request(&connection, "SELECT name FROM sqlite_schema"),
            DatabaseReader::Agent
        )
        .await,
        Err(DatabaseError::Retargeted)
    ));
}

/// Uses only the explicitly selected, disposable Docker fixture. No app/user credentials.
#[tokio::test]
#[ignore = "requires BRUHS_DATABASE_TEST_FIXTURE pointing at the isolated local Postgres fixture"]
async fn live_postgres_readonly_boundary() {
    let fixture = std::env::var("BRUHS_DATABASE_TEST_FIXTURE")
        .expect("Choose the disposable Postgres fixture JSON path");
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
    assert!(config["name"]
        .as_str()
        .unwrap()
        .starts_with("bruhs-ticket-pg-"));
    assert_eq!(config["database"], "bruhs_readonly_test");
    assert!(
        config.get("state").is_none_or(|state| state == "ready"),
        "Fixture startup is incomplete; stop the recovery fixture before retrying."
    );
    let directory = tempfile::tempdir().unwrap();
    let secrets = crate::secrets::SecretStore::in_memory();
    let host = DatabaseHost::open(directory.path().join("catalog.sqlite3"), secrets.clone())
        .await
        .unwrap();
    let connection = host
        .save(SaveDatabase {
            id: Uuid::new_v4(),
            project_path: directory.path().display().to_string(),
            name: "Disposable local Postgres".into(),
            expected_revision: None,
            password: Some(config["admin_password"].as_str().unwrap().into()),
            target: DatabaseTarget::Postgres {
                host: "127.0.0.1".into(),
                port: u16::try_from(config["port"].as_u64().unwrap()).unwrap(),
                database: config["database"].as_str().unwrap().into(),
                user: config["admin_user"].as_str().unwrap().into(),
            },
        })
        .await
        .unwrap();
    let read = "SELECT id,name,subscription FROM public.customers ORDER BY id";
    let before = host
        .query(
            connection.project_path.clone(),
            request(&connection, read),
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert_eq!(before.rows.len(), 2);
    assert_eq!(before.rows[0][2], None);
    let attacks=["INSERT INTO public.customers VALUES(3,'bad','x') RETURNING *", "UPDATE public.customers SET name='bad' RETURNING *", "DELETE FROM public.customers RETURNING *", "CREATE TABLE public.bad(x int)", "CREATE TEMP TABLE bad(x int)", "DROP TABLE public.customers", "TRUNCATE public.customers", "COMMIT", "SET TRANSACTION READ WRITE", "SELECT pg_catalog.set_config('transaction_read_only','off',true)", "SELECT 1; UPDATE public.customers SET name='bad'", "WITH changed AS (UPDATE public.customers SET name='bad' RETURNING *) SELECT * FROM changed", "EXPLAIN ANALYZE DELETE FROM public.customers", "SELECT public.test_write()", "SELECT pg_catalog.lo_create(0)"];
    for sql in attacks {
        assert!(
            host.query(
                connection.project_path.clone(),
                request(&connection, sql),
                DatabaseReader::User
            )
            .await
            .is_err(),
            "allowed {sql}"
        );
    }
    let after = host
        .query(
            connection.project_path.clone(),
            request(&connection, read),
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert_eq!(before.rows, after.rows);
    let precision=host.query(connection.project_path.clone(),request(&connection,"SELECT 9223372036854775807::bigint AS big, 1.12345678901234567890::numeric AS decimal, DATE '2026-11-13' AS date"),DatabaseReader::User).await.unwrap();
    assert_eq!(precision.rows[0][0].as_deref(), Some("9223372036854775807"));
    assert_eq!(
        precision.rows[0][1].as_deref(),
        Some("1.12345678901234567890")
    );
    assert!(host
        .query(
            connection.project_path.clone(),
            request(&connection, "SELECT repeat('x',1000000)"),
            DatabaseReader::User
        )
        .await
        .is_err());
    let paged = host
        .query(
            connection.project_path.clone(),
            request(&connection, "SELECT n FROM generate_series(1,100000) n"),
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert_eq!(paged.rows.len(), ROW_LIMIT);
    assert!(paged.truncated);
    let tables = host
        .tables(
            connection.project_path.clone(),
            connection.id,
            connection.revision,
            DatabaseReader::User,
        )
        .await
        .unwrap();
    assert!(tables
        .iter()
        .any(|table| table.schema == "public" && table.name == "customers"));
    let connection = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    let h = host.clone();
    let c = connection.clone();
    let task = tokio::spawn(async move {
        h.query(
            c.project_path.clone(),
            request(&c, "SELECT pg_sleep(30)"),
            DatabaseReader::Agent,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !host.0.active.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let connection = host
        .grant(DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: false,
        })
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    let renamed = host
        .save(SaveDatabase {
            id: connection.id,
            project_path: connection.project_path.clone(),
            name: "Renamed".into(),
            target: connection.target.clone(),
            expected_revision: Some(connection.revision),
            password: None,
        })
        .await
        .unwrap();
    assert_eq!(renamed.credential_version, connection.credential_version);
    let reopened = DatabaseHost::open(directory.path().join("catalog.sqlite3"), secrets)
        .await
        .unwrap();
    assert_eq!(
        reopened
            .query(
                renamed.project_path.clone(),
                request(&renamed, read),
                DatabaseReader::User
            )
            .await
            .unwrap()
            .rows,
        before.rows
    );
    let catalog = std::fs::read(directory.path().join("catalog.sqlite3")).unwrap();
    assert!(!catalog
        .windows(config["admin_password"].as_str().unwrap().len())
        .any(|bytes| bytes == config["admin_password"].as_str().unwrap().as_bytes()));
    println!(
        "{}",
        serde_json::json!({"postgres_engine":18,"blocked_mutation_attempts":attacks.len(),"rows_unchanged":true,"precision_preserved":true,"oversized_frame_rejected":true,"revocation_cancels_in_flight":true,"saved_password_reused_after_catalog_reopen":true})
    );
}
