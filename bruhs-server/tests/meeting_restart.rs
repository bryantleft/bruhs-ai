//! Real HTTP against a disposable server/database. Never uses the user's API port or providers.
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::{Child, Command};

struct Server {
    child: Child,
    url: String,
}

impl Server {
    async fn start(database: &Path, client: &Client) -> Self {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("reserve an isolated port");
        let port = listener
            .local_addr()
            .expect("read the reserved port")
            .port();
        drop(listener);
        let child = Command::new(env!("CARGO_BIN_EXE_bruhs-server"))
            .env("BRUHS_SERVER_PORT", port.to_string())
            .env("BRUHS_MEETING_DB", database)
            .env("BRUHS_CAPTURE_URL", "http://127.0.0.1:0")
            // Never read the real daemon token: point at a file that is absent.
            .env(
                "BRUHS_CAPTURE_TOKEN_FILE",
                database.with_extension("no-daemon-token"),
            )
            .env_remove("BRUHS_GOOGLE_CLIENT_ID")
            .env_remove("BRUHS_GOOGLE_CLIENT_SECRET")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn the disposable server");
        let mut server = Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
        };
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                assert!(
                    server
                        .child
                        .try_wait()
                        .expect("check server liveness")
                        .is_none(),
                    "Disposable server exited during startup"
                );
                if client
                    .get(format!("{}/api/people", server.url))
                    .send()
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("Disposable server did not become ready");
        server
    }

    async fn kill(&mut self) {
        self.child.kill().await.expect("kill the owned server");
        self.child.wait().await.expect("reap the owned server");
    }
}

async fn create(client: &Client, server: &Server, id: &str, name: &str) -> Value {
    let response = client
        .post(format!("{}/api/people", server.url))
        .json(&json!({"id":id, "display_name":name, "created_at":1, "updated_at":1}))
        .send()
        .await
        .expect("send a fixture contact");
    assert_eq!(response.status(), StatusCode::CREATED);
    response
        .json()
        .await
        .expect("decode the saved fixture contact")
}

#[tokio::test]
async fn acknowledged_merge_survives_process_kill_and_replay_without_resurrecting_source() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("server.sqlite3");
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let mut server = Server::start(&database, &client).await;
    create(&client, &server, "source", "Original source").await;
    create(&client, &server, "target", "Original target").await;
    create(&client, &server, "other", "Other").await;
    let response = client
        .patch(format!("{}/api/people/source/tracked", server.url))
        .json(&json!({"tracked":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let merge_url = format!("{}/api/people/source/merge_into/target", server.url);
    let response = client.post(merge_url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let merged: Value = response.json().await.unwrap();
    assert_eq!(merged["tracked"], false);
    assert!(merged["aliases"]
        .as_array()
        .unwrap()
        .contains(&json!("Original source")));
    server.kill().await;

    let mut reopened = Server::start(&database, &client).await;
    let versions = client
        .get(format!("{}/api/people/versions", reopened.url))
        .send()
        .await
        .unwrap();
    assert_eq!(versions.status(), StatusCode::OK);
    let versions: Value = versions.json().await.unwrap();
    assert_eq!(versions["entries"].as_array().unwrap().len(), 2);
    assert_eq!(versions["next_after"], Value::Null);
    let batch = client
        .post(format!("{}/api/people/profiles", reopened.url))
        .json(&json!({"ids":["target","source","unknown"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(batch.status(), StatusCode::OK);
    assert_eq!(batch.json::<Value>().await.unwrap(), json!([merged]));
    let bad = client
        .post(format!("{}/api/people/profiles", reopened.url))
        .json(&json!({"ids":["target","target"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let indexed = client
        .get(format!("{}/api/people/target", reopened.url))
        .send()
        .await
        .unwrap();
    assert_eq!(indexed.status(), StatusCode::OK);
    assert_eq!(indexed.json::<Value>().await.unwrap(), merged);
    let missing = client
        .get(format!("{}/api/people/source", reopened.url))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let resolved = client
        .post(format!("{}/api/people/resolve-merges", reopened.url))
        .json(&json!({"ids":["source","unknown","target"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    assert_eq!(
        resolved.json::<Value>().await.unwrap(),
        json!([{"source_id":"source", "target":merged}])
    );
    let response = client
        .post(format!(
            "{}/api/people/source/merge_into/target",
            reopened.url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json::<Value>().await.unwrap(), merged);
    let conflicting = client
        .post(format!(
            "{}/api/people/source/merge_into/other",
            reopened.url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(conflicting.status(), StatusCode::CONFLICT);
    let resurrect = client.post(format!("{}/api/people", reopened.url))
        .json(&json!({"id":"source", "display_name":"Stale source", "created_at":1, "updated_at":i64::MAX}))
        .send().await.unwrap();
    assert_eq!(resurrect.status(), StatusCode::CONFLICT);
    let renamed = client
        .patch(format!("{}/api/people/target", reopened.url))
        .json(&json!({"display_name":"  Renamed target  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::OK);
    let renamed: Value = renamed.json().await.unwrap();
    assert_eq!(renamed["display_name"], "Renamed target");
    assert_eq!(renamed["aliases"], merged["aliases"]);
    assert_eq!(renamed["tracked"], false);
    assert!(renamed["updated_at"].as_i64().unwrap() > merged["updated_at"].as_i64().unwrap());
    reopened.kill().await;

    let mut final_server = Server::start(&database, &client).await;
    let people: Vec<Value> = client
        .get(format!("{}/api/people", final_server.url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(people.len(), 2);
    assert!(!people.iter().any(|person| person["id"] == "source"));
    assert_eq!(
        people.iter().find(|person| person["id"] == "target"),
        Some(&renamed)
    );
    final_server.kill().await;
}
