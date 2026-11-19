//! The vault routes over HTTP, through the real router and its guard: an
//! in-memory keychain, a stand-in `op` that is signed out, and bruh `a`.
//! Every response is read as text and checked for the random values sent.

use std::{os::unix::fs::PermissionsExt as _, path::Path, sync::Arc, time::Duration};

use axum::{http::StatusCode, Extension};
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::AppState;
use bruhs_models::reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand};
use bruhs_models::vault::{
    SecretField, VaultAttention, BRUH_MISSING, CHECK_OK, ENTRY_GONE, INVALID_ENTRY,
    PROJECT_MISSING, SIGN_IN_LOCAL,
};
use reqwest::Method;
use serde_json::{json, Value};

struct Fixture {
    state: Arc<AppState>,
    runtime: ReactiveRuntime,
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A stand-in `op` that always answers as a signed-out 1Password does.
fn signed_out_op(folder: &Path) -> std::path::PathBuf {
    let path = folder.join("op");
    std::fs::write(
        &path,
        "#!/bin/sh\necho '[ERROR] You are not currently signed in.' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A random value no response may hold.
fn canary(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(directory.path().join("automation.sqlite3"))
            .await
            .unwrap();
        let state = Arc::new(
            AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .one_password(bruhs_core::secrets::OnePassword::at(
                    signed_out_op(directory.path()),
                    Duration::from_secs(10),
                ))
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        );
        runtime.attach_vault(state.secrets().clone()).unwrap();
        let config = crate::config::Config::from_settings(crate::config::Settings {
            port: Some("0"),
            web_dist: directory.path().join("missing").to_str(),
            ..Default::default()
        })
        .unwrap();
        let app = crate::router(crate::config::ServerState {
            app: Arc::clone(&state),
            config: Arc::new(config),
            updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
                "", "", "unused",
            )),
        })
        .layer(Extension(runtime.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap();
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent {
                    agent: ReactiveAgent {
                        id: "a".into(),
                        name: "Pix".into(),
                        instructions: "Book the flight.".into(),
                        memory: String::new(),
                        model: "claude-sonnet-4-6".into(),
                        project_path: directory.path().to_string_lossy().into_owned(),
                        enabled: true,
                    },
                },
            })
            .await
            .unwrap();
        Self {
            state,
            runtime,
            base,
            client,
            server,
            directory,
        }
    }

    /// `method path` with a JSON or raw text body: the status and the raw
    /// response body.
    async fn send(&self, method: Method, path: &str, body: Option<Body>) -> (StatusCode, String) {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        match body {
            Some(Body::Json(body)) => request = request.json(&body),
            Some(Body::Raw(text)) => {
                request = request
                    .header("content-type", "application/json")
                    .body(text);
            }
            None => {}
        }
        let response = request.send().await.unwrap();
        (response.status(), response.text().await.unwrap())
    }

    async fn create(&self, body: Value) -> (StatusCode, String) {
        self.send(Method::POST, "/api/vault", Some(Body::Json(body)))
            .await
    }

    /// Creates an entry that must save: its listing JSON.
    async fn created(&self, body: Value) -> Value {
        let (status, text) = self.create(body).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    async fn listing(&self) -> (Value, String) {
        let (status, text) = self.send(Method::GET, "/api/vault", None).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        (serde_json::from_str(&text).unwrap(), text)
    }

    async fn check(&self, id: &str) -> (StatusCode, Value) {
        let (status, text) = self
            .send(Method::POST, &format!("/api/vault/{id}/check"), None)
            .await;
        (status, serde_json::from_str(&text).unwrap())
    }
}

enum Body {
    Json(Value),
    Raw(String),
}

fn global_key(name: &str, value: &str) -> Value {
    json!({
        "name": name,
        "scope": {"type": "global"},
        "secret": {"kind": "value", "value": value},
    })
}

fn saved_login(name: &str, scope: Value, url: &str, password: &str) -> Value {
    json!({
        "name": name,
        "scope": scope,
        "secret": {"kind": "login", "url": url, "fields": {
            "source": "saved",
            "username": "owner@shop.test",
            "password": password,
        }},
    })
}

fn error(text: &str) -> String {
    json!({ "error": text }).to_string()
}

#[tokio::test]
async fn vault_routes_create_list_update_and_delete_without_values() {
    let fixture = Fixture::new().await;
    let key = canary("key");
    let password = canary("pw");
    let replaced = canary("new-pw");
    let canaries = [&key, &password, &replaced];
    let clean = |text: &str| {
        for canary in canaries {
            assert!(!text.contains(canary.as_str()), "a value came back: {text}");
        }
    };

    let created = fixture.created(global_key(" Deploy key ", &key)).await;
    clean(&created.to_string());
    let key_id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["name"], "Deploy key");
    let login = fixture
        .created(saved_login(
            "Shop",
            json!({"type": "bruh", "agent_id": "a"}),
            "https://shop.test/in",
            &password,
        ))
        .await;
    clean(&login.to_string());
    let login_id = login["id"].as_str().unwrap().to_owned();
    let reference = fixture
        .created(json!({
            "name": "Stripe",
            "scope": {"type": "global"},
            "secret": {"kind": "one_password", "reference": "op://Private/Stripe/key"},
        }))
        .await;

    let (listing, text) = fixture.listing().await;
    clean(&text);
    let mut names: Vec<&str> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["Deploy key", "Shop", "Stripe"]);
    assert_eq!(listing["bruhs"], json!([{"agent_id": "a", "name": "Pix"}]));
    assert_eq!(listing["one_password"], true);
    assert!(listing["entries"].as_array().unwrap().contains(&reference));

    // A new password: the entry comes back, the value doesn't.
    let (status, text) = fixture
        .send(
            Method::PUT,
            &format!("/api/vault/{login_id}"),
            Some(Body::Json(json!({"secret": {
                "kind": "login",
                "url": "https://shop.test/sign-in",
                "fields": {"source": "saved", "username": "owner@shop.test", "password": replaced},
            }}))),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    clean(&text);
    let updated: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(updated["kind"]["url"], "https://shop.test/sign-in");
    let stored = fixture
        .state
        .secrets()
        .resolve_login(&login_id, SecretField::Password)
        .await
        .unwrap();
    assert_eq!(stored.as_str(), replaced);

    // Deleting a granted entry takes it out of every bruh's grants.
    fixture
        .runtime
        .command(ReactiveCommand::SetVault {
            agent_id: "a".into(),
            entries: vec![key_id.clone()],
        })
        .await
        .unwrap();
    let path = format!("/api/vault/{key_id}");
    let (status, text) = fixture.send(Method::DELETE, &path, None).await;
    assert_eq!((status, text.as_str()), (StatusCode::NO_CONTENT, ""));
    let (listing, text) = fixture.listing().await;
    clean(&text);
    assert!(listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["id"] != key_id.as_str()));
    let snapshot = fixture.runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.bruhs[0].vault, Vec::<String>::new());
    let (status, text) = fixture.send(Method::DELETE, &path, None).await;
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(ENTRY_GONE)));
    let (status, text) = fixture
        .send(
            Method::PUT,
            &path,
            Some(Body::Json(
                json!({"secret": {"kind": "value", "value": replaced}}),
            )),
        )
        .await;
    assert_eq!((status, text), (StatusCode::NOT_FOUND, error(ENTRY_GONE)));
}

#[tokio::test]
async fn malformed_and_invalid_bodies_never_echo() {
    let fixture = Fixture::new().await;
    let value = canary("value");
    let bad = |status: StatusCode, text: &str, expected: &str| {
        assert!(!text.contains(&value), "the body came back: {text}");
        assert_eq!(
            (status, text.to_owned()),
            (StatusCode::BAD_REQUEST, error(expected))
        );
    };

    // Serde's own messages would quote these.
    for body in [
        json!({"name": {"canary": 1}}),
        json!({"name": "Deploy", "scope": value, "secret": {"kind": "value", "value": "v"}}),
        json!({"name": "Deploy", "scope": {"type": "global"}, "secret": {"kind": value}}),
        json!({"name": "Deploy", "scope": {"type": value}, "secret": {"kind": "value"}}),
    ] {
        let (status, text) = fixture.create(body).await;
        bad(status, &text, INVALID_ENTRY);
    }
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/vault",
            Some(Body::Raw(format!("{{\"name\": \"{value}\""))),
        )
        .await;
    bad(status, &text, INVALID_ENTRY);
    let (status, text) = fixture
        .send(
            Method::PUT,
            "/api/vault/any",
            Some(Body::Json(json!({"secret": {"kind": value}}))),
        )
        .await;
    bad(status, &text, INVALID_ENTRY);
    // Bodies past the limit are refused the same way.
    let huge = format!("{value}{}", "x".repeat(80 << 10));
    let (status, text) = fixture.create(global_key("Huge", &huge)).await;
    bad(status, &text, INVALID_ENTRY);

    // Well-formed, but not an entry that can be saved.
    let (status, text) = fixture
        .create(saved_login(
            "Local",
            json!({"type": "global"}),
            "http://localhost/",
            &value,
        ))
        .await;
    bad(status, &text, SIGN_IN_LOCAL);
    let missing = fixture.directory.path().join("missing");
    let (status, text) = fixture
        .create(json!({
            "name": "Deploy",
            "scope": {"type": "project", "path": missing},
            "secret": {"kind": "value", "value": value},
        }))
        .await;
    bad(status, &text, PROJECT_MISSING);
    let file = fixture.directory.path().join("automation.sqlite3");
    let (status, text) = fixture
        .create(json!({
            "name": "Deploy",
            "scope": {"type": "project", "path": file},
            "secret": {"kind": "value", "value": value},
        }))
        .await;
    bad(status, &text, PROJECT_MISSING);
    let (status, text) = fixture
        .create(json!({
            "name": "Deploy",
            "scope": {"type": "bruh", "agent_id": "nobody"},
            "secret": {"kind": "value", "value": value},
        }))
        .await;
    bad(status, &text, BRUH_MISSING);
    let (listing, text) = fixture.listing().await;
    assert!(!text.contains(&value));
    assert_eq!(listing["entries"], json!([]), "nothing was saved");

    // A project entry is kept under the folder's canonical path.
    let project = fixture.directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let entry = fixture
        .created(json!({
            "name": "Deploy",
            "scope": {"type": "project", "path": project.join("..").join("project")},
            "secret": {"kind": "value", "value": value},
        }))
        .await;
    let canonical = std::fs::canonicalize(&project).unwrap();
    assert_eq!(
        entry["scope"],
        json!({"type": "project", "path": canonical.to_str().unwrap()})
    );
}

#[tokio::test]
async fn check_reports_ready_or_why() {
    let fixture = Fixture::new().await;
    let login = fixture
        .created(saved_login(
            "Shop",
            json!({"type": "global"}),
            "https://shop.test/in",
            &canary("pw"),
        ))
        .await;
    let (status, body) = fixture.check(login["id"].as_str().unwrap()).await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, json!({"ok": true, "message": CHECK_OK}))
    );

    let reference = fixture
        .created(json!({
            "name": "Stripe",
            "scope": {"type": "global"},
            "secret": {"kind": "one_password", "reference": "op://Private/Stripe/key"},
        }))
        .await;
    let (status, body) = fixture.check(reference["id"].as_str().unwrap()).await;
    assert_eq!(
        (status, body),
        (
            StatusCode::OK,
            json!({
                "ok": false,
                "message": "1Password needs you. Sign in to the 1Password app, then check again.",
            })
        )
    );
    assert_eq!(
        VaultAttention::OnePasswordNeedsYou.message(),
        "1Password needs you. Sign in to the 1Password app, then check again."
    );
    // The listing shows the failure on its row.
    let (listing, _) = fixture.listing().await;
    let row = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == reference["id"])
        .cloned()
        .unwrap();
    assert_eq!(row["attention"], "one_password_needs_you");

    let (status, body) = fixture.check("v1-gone").await;
    assert_eq!(
        (status, body),
        (StatusCode::NOT_FOUND, json!({ "error": ENTRY_GONE }))
    );
}

/// Ruling F25: the old Credentials routes went with the web page that
/// called them. Settings, Vault manages every entry through `/api/vault`.
/// Each answers as a path that was never a route does: the static files'
/// 404, or their 405 for a method other than GET.
#[tokio::test]
async fn credentials_routes_are_gone() {
    let fixture = Fixture::new().await;
    let (status, text) = fixture.send(Method::GET, "/api/credentials", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    for method in [Method::GET, Method::PUT, Method::DELETE] {
        let body = || (method == Method::PUT).then(|| Body::Json(json!({"value": "unused"})));
        let (status, text) = fixture
            .send(method.clone(), "/api/credentials/v1-any", body())
            .await;
        let (unknown, _) = fixture
            .send(method.clone(), "/api/never-a-route/v1-any", body())
            .await;
        assert!(!status.is_success(), "{method}: {text}");
        assert_eq!(status, unknown, "{method}: {text}");
    }
}
