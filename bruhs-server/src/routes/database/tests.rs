use super::*;
use std::sync::Arc;

#[tokio::test]
async fn database_native_http_and_agent_grants_share_the_readonly_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("fixture.sqlite3");
    let raw = rusqlite::Connection::open(&database).unwrap();
    raw.execute_batch("CREATE TABLE customers(id INTEGER PRIMARY KEY,name TEXT); INSERT INTO customers VALUES(1,'Ada'),(2,'Lin');").unwrap();
    drop(raw);
    let before = std::fs::read(&database).unwrap();
    let host = DatabaseHost::open(
        directory.path().join("connections.sqlite3"),
        bruhs_core::secrets::SecretStore::in_memory(),
    )
    .await
    .unwrap();
    let config = crate::config::Config::from_settings(crate::config::Settings {
        port: Some("0"),
        web_dist: directory.path().join("missing").to_str(),
        ..Default::default()
    })
    .unwrap();
    let app = crate::router(crate::config::ServerState {
        app: Arc::new(
            bruhs_core::AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        ),
        config: Arc::new(config),
        updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
            "", "", "unused",
        )),
    })
    .layer(Extension(host));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = crate::desktop_server_client::ServerClient::new(&base, &base).unwrap();
    let connection = client
        .save_database(&SaveDatabase {
            id: Uuid::new_v4(),
            project_path: directory.path().display().to_string(),
            name: "HTTP fixture".into(),
            target: DatabaseTarget::Sqlite {
                path: database.display().to_string(),
            },
            expected_revision: None,
            password: None,
        })
        .await
        .unwrap();
    assert!(!connection.agent_access);
    assert_eq!(
        client.databases(&connection.project_path).await.unwrap(),
        vec![connection.clone()]
    );
    let tables = client
        .database_tables(&connection.project_path, &connection)
        .await
        .unwrap();
    assert_eq!(tables[0].name, "customers");
    let mut request = ProjectDatabaseQuery {
        project_path: connection.project_path.clone(),
        query: DatabaseQuery {
            id: connection.id,
            revision: connection.revision,
            sql: "SELECT * FROM customers ORDER BY id".into(),
            request_id: Uuid::new_v4(),
        },
    };
    let result = client.database_query(&request).await.unwrap();
    assert_eq!(result.rows[0][1].as_deref(), Some("Ada"));
    request.query.sql = "UPDATE customers SET name='bad' RETURNING *".into();
    request.query.request_id = Uuid::new_v4();
    assert!(client.database_query(&request).await.is_err());
    assert_eq!(before, std::fs::read(&database).unwrap());
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let agent = format!("{base}/api/databases/agent");
    let list = http
        .get(&agent)
        .query(&[("project_path", &connection.project_path)])
        .send()
        .await
        .unwrap();
    assert_eq!(list.headers()["cache-control"], "no-store");
    assert_eq!(
        list.json::<Vec<DatabaseConnection>>().await.unwrap().len(),
        0
    );
    request.query.sql = "SELECT * FROM customers".into();
    request.query.request_id = Uuid::new_v4();
    assert_eq!(
        http.post(format!("{agent}/query"))
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let connection = client
        .database_grant(&DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    request.query.revision = connection.revision;
    let shared = http
        .post(format!("{agent}/query"))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(shared.status(), StatusCode::OK);
    assert_eq!(shared.headers()["cache-control"], "no-store");
    assert_eq!(shared.json::<DatabaseResult>().await.unwrap().rows.len(), 2);
    let browse = ProjectDatabaseBrowse {
        project_path: connection.project_path.clone(),
        browse: BrowseDatabase {
            request_id: Uuid::new_v4(),
            id: connection.id,
            revision: connection.revision,
            schema: "main".into(),
            table: "customers".into(),
            offset: 0,
            sort_column: Some("id".into()),
            descending: true,
            filter_column: Some("name".into()),
            filter: "Ada".into(),
        },
    };
    assert_eq!(client.database_browse(&browse).await.unwrap().rows.len(), 1);
    assert_eq!(
        client.database_describe(&browse).await.unwrap().rows.len(),
        2
    );
    let connection = client
        .database_grant(&DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: false,
        })
        .await
        .unwrap();
    request.query.revision = connection.revision;
    assert_eq!(
        http.post(format!("{agent}/query"))
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let denied = http
        .post(format!("{base}/api/databases/grant"))
        .header("Origin", "https://unrelated.example")
        .json(&DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let malformed = http
        .post(format!("{base}/api/databases"))
        .json(&serde_json::json!({"password":{"private-value":"must-not-echo"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert!(!malformed.text().await.unwrap().contains("must-not-echo"));
    if let Some(executable) = std::env::var_os("BRUHS_DATABASE_MCP_EXECUTABLE") {
        exercise_mcp(executable, &base, &client, &connection).await;
        assert_eq!(
            before,
            std::fs::read(&database).unwrap(),
            "MCP write attempts must leave the database unchanged"
        );
    }
    server.abort();
}

async fn exercise_mcp(
    executable: std::ffi::OsString,
    base: &str,
    client: &crate::desktop_server_client::ServerClient,
    connection: &DatabaseConnection,
) {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut helper = tokio::process::Command::new(executable)
        .arg("--database-mcp")
        .env("BRUHS_REACTIVE_ENDPOINT", base)
        .env("BRUHS_DATABASE_PROJECT", &connection.project_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = helper.stdin.take().unwrap();
    let mut output = BufReader::new(helper.stdout.take().unwrap());
    async fn exchange(
        input: &mut tokio::process::ChildStdin,
        output: &mut BufReader<tokio::process::ChildStdout>,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let request = serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        input.flush().await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            output.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 1);
        assert!(response.get("error").is_none());
        response["result"].clone()
    }
    fn content(value: &serde_json::Value) -> serde_json::Value {
        serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
    }
    let initialized = exchange(&mut input, &mut output, "initialize", serde_json::json!({})).await;
    assert_eq!(initialized["serverInfo"]["name"], "bruhs_database");
    let tools = exchange(&mut input, &mut output, "tools/list", serde_json::json!({})).await;
    let names: Vec<_> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["database_connections", "database_schema", "database_query"]
    );
    let empty = exchange(
        &mut input,
        &mut output,
        "tools/call",
        serde_json::json!({"name":"database_connections","arguments":{}}),
    )
    .await;
    assert_eq!(content(&empty), serde_json::json!([]));
    let allowed = client
        .database_grant(&DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: true,
        })
        .await
        .unwrap();
    let connections = exchange(
        &mut input,
        &mut output,
        "tools/call",
        serde_json::json!({"name":"database_connections","arguments":{}}),
    )
    .await;
    assert_eq!(content(&connections)[0]["id"], allowed.id.to_string());
    let described=exchange(&mut input,&mut output,"tools/call",serde_json::json!({"name":"database_schema","arguments":{"id":allowed.id,"revision":allowed.revision,"schema":"main","table":"customers"}})).await;
    assert_eq!(content(&described)["rows"].as_array().unwrap().len(), 2);
    let read=exchange(&mut input,&mut output,"tools/call",serde_json::json!({"name":"database_query","arguments":{"id":allowed.id,"revision":allowed.revision,"sql":"SELECT * FROM customers ORDER BY id"}})).await;
    assert_eq!(content(&read)["rows"][0][1], "Ada");
    let write=exchange(&mut input,&mut output,"tools/call",serde_json::json!({"name":"database_query","arguments":{"id":allowed.id,"revision":allowed.revision,"sql":"UPDATE customers SET name='bad' RETURNING *"}})).await;
    assert_eq!(write["isError"], true);
    let revoked = client
        .database_grant(&DatabaseGrant {
            id: allowed.id,
            expected_revision: allowed.revision,
            enabled: false,
        })
        .await
        .unwrap();
    let denied=exchange(&mut input,&mut output,"tools/call",serde_json::json!({"name":"database_query","arguments":{"id":revoked.id,"revision":revoked.revision,"sql":"SELECT * FROM customers"}})).await;
    assert_eq!(denied["isError"], true);
    input.shutdown().await.unwrap();
    drop(input);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), helper.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    println!("Production database MCP helper: initialization, registry, opt-in reads, column schema, blocked write and revoked read passed.");
}
