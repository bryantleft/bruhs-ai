//! The Apps routes over HTTP, through the real router and its guard: an
//! in-memory keychain, a wiremock upstream standing in for every app, and
//! bruh `a`. Every response is read as text and checked for the tokens
//! sent. The bruh route's tests share this fixture.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{http::StatusCode, Extension};
use bruhs_core::apps::{AppHttp, AppsHost};
use bruhs_core::mail::compose::test_support::{ConnectedMail, FakeTransport};
use bruhs_core::mail::gmail::GmailClient;
use bruhs_core::mail::outlook::OutlookClient;
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::AppState;
use bruhs_models::apps::INVALID_APP_ACCOUNT;
use bruhs_models::crew::{
    AccessLevel, AppGrant, BruhAccess, BruhLook, CREW_PROTOCOL, OWNER_ONLY_APPS,
};
use bruhs_models::reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand};
use reqwest::Method;
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A page the shared guard lets through but that isn't the owner's app.
pub(crate) const OTHER_PAGE: &str = "http://127.0.0.1:5173";

/// A random token of the shape the catalog accepts, which no response may
/// hold.
pub(crate) fn canary() -> String {
    format!("tok_{}", uuid::Uuid::new_v4().simple())
}

/// What the server logged while a test ran, from this thread (the server
/// and the test share a current-thread runtime).
#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    /// Captures every event and span at every level until the guard drops.
    pub(crate) fn capture(&self) -> tracing::subscriber::DefaultGuard {
        let logs = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || logs.clone())
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        Self::refresh();
        guard
    }

    /// Callsites another test thread hit first may have cached that no
    /// subscriber wants them; ask every one again.
    pub(crate) fn refresh() {
        tracing::callsite::rebuild_interest_cache();
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// How the fixture's apps host sends email.
#[derive(Default)]
pub(crate) struct MailSetup {
    pub(crate) transport: Option<Arc<FakeTransport>>,
    pub(crate) send_budget: Option<Duration>,
}

pub(crate) struct Fixture {
    pub(crate) state: Arc<AppState>,
    pub(crate) runtime: ReactiveRuntime,
    pub(crate) upstream: MockServer,
    pub(crate) mail: Option<ConnectedMail>,
    pub(crate) base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    /// Bruh `a`, a Claude bruh at Ask first, without mail.
    pub(crate) async fn new() -> Self {
        Self::with_mail(None).await
    }

    /// With `mail`, a synthetic Gmail mailbox whose email goes through its
    /// transport.
    pub(crate) async fn with_mail(mail: Option<MailSetup>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(directory.path().join("automation.sqlite3"))
            .await
            .unwrap();
        let connected = match mail {
            Some(_) => Some(ConnectedMail::gmail().await),
            None => None,
        };
        let connections = connected
            .as_ref()
            .map_or_else(bruhs_core::connections::Connections::in_memory, |mail| {
                mail.connections.clone()
            });
        let state = Arc::new(
            AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(connections)
                .build(),
        );
        runtime.attach_vault(state.secrets().clone()).unwrap();
        let upstream = MockServer::start().await;
        let origin = format!("http://{}", upstream.address());
        let mut apps = AppsHost::new(
            state.secrets().clone(),
            AppHttp::with_upstream(*upstream.address()).unwrap(),
            connected.as_ref().map(|mail| mail.runtime.clone()),
            Arc::new(GmailClient::for_test_origin(&origin).unwrap()),
            Arc::new(OutlookClient::for_test_origin(&origin).unwrap()),
        );
        if let Some(setup) = mail {
            if let Some(transport) = setup.transport {
                apps = apps.with_mail_transport(transport);
            }
            if let Some(budget) = setup.send_budget {
                apps = apps.with_send_budget(budget);
            }
        }
        runtime.attach_apps(apps.clone()).unwrap();
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
        .layer(Extension(runtime.clone()))
        .layer(Extension(apps));
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
        let fixture = Self {
            state,
            runtime,
            upstream,
            mail: connected,
            base,
            client,
            server,
            _directory: directory,
        };
        fixture
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent {
                    agent: ReactiveAgent {
                        id: "a".into(),
                        name: "Pix".into(),
                        instructions: "Triage the issues.".into(),
                        memory: String::new(),
                        model: "claude-sonnet-4-6".into(),
                        project_path: fixture._directory.path().to_string_lossy().into_owned(),
                        enabled: true,
                    },
                },
            })
            .await;
        fixture.set_access(AccessLevel::Ask, false).await;
        fixture
    }

    pub(crate) async fn command(&self, command: ReactiveCommand) {
        self.runtime.command(command).await.unwrap();
    }

    /// Bruh `a`'s access level, and whether it may send as the owner.
    pub(crate) async fn set_access(&self, level: AccessLevel, send_as_me: bool) {
        self.command(ReactiveCommand::SetBruh {
            agent_id: "a".into(),
            look: BruhLook::default(),
            access: BruhAccess {
                level,
                send_as_me,
                ..BruhAccess::default()
            },
            effort: None,
        })
        .await;
    }

    /// `method path` with a JSON or raw body, from a page at `origin`
    /// (none: the desktop's native client or the helper): the status and
    /// the raw response body.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Body>,
        origin: Option<&str>,
    ) -> (StatusCode, String) {
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
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().await.unwrap();
        (response.status(), response.text().await.unwrap())
    }

    pub(crate) async fn post(&self, path: &str, body: Value) -> (StatusCode, String) {
        self.send(Method::POST, path, Some(Body::Json(body)), None)
            .await
    }

    /// Connects a Global GitHub account through the route; its id.
    pub(crate) async fn connected(&self, token: &str) -> String {
        let (status, text) = self.post("/api/apps/accounts", github(token)).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let account: Value = serde_json::from_str(&text).unwrap();
        account["id"].as_str().unwrap().to_owned()
    }

    /// Grants (or takes back) app account `entry_id` to bruh `a`.
    pub(crate) async fn grant_token(&self, entry_id: &str, granted: bool) {
        self.command(ReactiveCommand::SetAppGrant {
            agent_id: "a".into(),
            grant: AppGrant::Token {
                entry_id: entry_id.into(),
            },
            granted,
        })
        .await;
    }

    pub(crate) async fn listing(&self) -> (Value, String) {
        let (status, text) = self.send(Method::GET, "/api/apps", None, None).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        (serde_json::from_str(&text).unwrap(), text)
    }
}

pub(crate) enum Body {
    Json(Value),
    Raw(String),
}

/// A Global GitHub connect body for `token`.
pub(crate) fn github(token: &str) -> Value {
    json!({"app": "github", "scope": {"type": "global"}, "token": token})
}

/// GitHub's `/user` names `login` for `token`.
pub(crate) async fn github_user(server: &MockServer, token: &str, login: &str) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": login})))
        .with_priority(1)
        .mount(server)
        .await;
}

/// Every other `/user` is refused, as GitHub refuses a bad token.
pub(crate) async fn refuse_the_rest(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"message": "Bad credentials"})),
        )
        .with_priority(10)
        .mount(server)
        .await;
}

fn error(text: &str) -> String {
    json!({ "error": text }).to_string()
}

#[tokio::test]
async fn connect_lists_tests_and_disconnects_without_tokens() {
    let fixture = Fixture::new().await;
    let (good, bad) = (canary(), canary());
    github_user(&fixture.upstream, &good, "octo").await;
    refuse_the_rest(&fixture.upstream).await;
    let clean = |text: &str| {
        for token in [&good, &bad] {
            assert!(!text.contains(token.as_str()), "a token came back: {text}");
        }
    };

    let (status, text) = fixture.post("/api/apps/accounts", github(&good)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    clean(&text);
    let account: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(account["app"], "github");
    assert_eq!(account["account"], "octo");
    assert_eq!(account["name"], "GitHub octo");
    let id = account["id"].as_str().unwrap().to_owned();

    let (listing, text) = fixture.listing().await;
    clean(&text);
    assert_eq!(listing["accounts"], json!([account]));
    assert_eq!(listing["mailboxes"], json!([]));
    assert_eq!(listing["protocol"], CREW_PROTOCOL);
    assert_eq!(
        listing["bruhs"],
        json!([{"agent_id": "a", "name": "Pix", "apps": [], "mail": [], "unavailable": null}])
    );

    let (status, text) = fixture
        .send(
            Method::POST,
            &format!("/api/apps/accounts/{id}/test"),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    clean(&text);
    let result: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["account"], "octo");

    // A token GitHub refuses saves nothing and says why.
    let (status, text) = fixture.post("/api/apps/accounts", github(&bad)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    assert_eq!(
        text,
        error("GitHub refused the token. Check it and what it may do.")
    );
    clean(&text);
    let (listing, text) = fixture.listing().await;
    clean(&text);
    assert_eq!(listing["accounts"].as_array().unwrap().len(), 1);

    // Replacing with a refused token is refused too, and keeps the old one.
    let (status, text) = fixture
        .send(
            Method::PUT,
            &format!("/api/apps/accounts/{id}"),
            Some(Body::Json(json!({"token": bad}))),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    clean(&text);
    let kept = fixture
        .state
        .secrets()
        .resolve_app_token(&id)
        .await
        .unwrap();
    assert_eq!(kept.token.as_str(), good);
    drop(kept);
    let (status, text) = fixture
        .send(
            Method::PUT,
            &format!("/api/apps/accounts/{id}"),
            Some(Body::Json(json!({"token": good}))),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    clean(&text);

    // A bruh granted it loses the grant when it is disconnected.
    fixture.grant_token(&id, true).await;
    let (listing, _) = fixture.listing().await;
    assert_eq!(listing["bruhs"][0]["apps"], json!([id]));
    let (status, text) = fixture
        .send(Method::DELETE, &format!("/api/vault/{id}"), None, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{text}");
    let (listing, text) = fixture.listing().await;
    clean(&text);
    assert_eq!(listing["accounts"], json!([]));
    assert_eq!(listing["bruhs"][0]["apps"], json!([]));
    let (status, text) = fixture
        .send(
            Method::POST,
            &format!("/api/apps/accounts/{id}/test"),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert_eq!(text, error(bruhs_models::vault::ENTRY_GONE));
}

#[tokio::test]
async fn a_bruh_account_needs_its_bruh_and_a_codex_bruh_is_told_why() {
    let fixture = Fixture::new().await;
    let token = canary();
    github_user(&fixture.upstream, &token, "octo").await;
    let bruh_scope = |agent_id: &str| json!({"app": "github", "scope": {"type": "bruh", "agent_id": agent_id}, "token": token});
    let (status, text) = fixture
        .post("/api/apps/accounts", bruh_scope("nobody"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(text, error(bruhs_models::vault::BRUH_MISSING));
    assert!(!text.contains(&token));
    // No test call ran for it.
    assert!(fixture
        .upstream
        .received_requests()
        .await
        .unwrap()
        .is_empty());
    let (status, text) = fixture.post("/api/apps/accounts", bruh_scope("a")).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let account: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(account["scope"], json!({"type": "bruh", "agent_id": "a"}));

    fixture.set_access(AccessLevel::ReadOnly, false).await;
    let (listing, _) = fixture.listing().await;
    assert_eq!(
        listing["bruhs"][0]["unavailable"],
        bruhs_models::crew::APPS_NEED_ASK
    );
}

#[tokio::test]
async fn apps_routes_take_the_owner() {
    let fixture = Fixture::new().await;
    let token = canary();
    github_user(&fixture.upstream, &token, "octo").await;
    let id = fixture.connected(&token).await;
    let routes = [
        (Method::GET, "/api/apps".to_owned(), None),
        (
            Method::POST,
            "/api/apps/accounts".to_owned(),
            Some(github(&token)),
        ),
        (
            Method::PUT,
            format!("/api/apps/accounts/{id}"),
            Some(json!({"token": token})),
        ),
        (Method::POST, format!("/api/apps/accounts/{id}/test"), None),
    ];
    let own = fixture.base.clone();
    for (method, path, body) in &routes {
        let (status, text) = fixture
            .send(
                method.clone(),
                path,
                body.clone().map(Body::Json),
                Some(OTHER_PAGE),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
        assert_eq!(text, error(OWNER_ONLY_APPS));
        for origin in [None, Some(own.as_str())] {
            let (status, text) = fixture
                .send(method.clone(), path, body.clone().map(Body::Json), origin)
                .await;
            assert_ne!(status, StatusCode::FORBIDDEN, "{method} {path}: {text}");
            assert!(!text.contains(&token));
        }
    }
    // The second connect of the same account was refused as a duplicate.
    let (listing, _) = fixture.listing().await;
    assert_eq!(listing["accounts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn malformed_bodies_never_echo() {
    let fixture = Fixture::new().await;
    let token = canary();
    github_user(&fixture.upstream, &token, "octo").await;
    let id = fixture.connected(&token).await;
    let bodies = [
        json!({"app": "github", "token": {"x": token}}).to_string(),
        json!({"app": "github", "scope": {"type": "global"}, "token": token, "extra": token})
            .to_string(),
        format!("{{\"app\": \"github\", \"token\": \"{token}"),
    ];
    for body in bodies {
        let (status, text) = fixture
            .send(
                Method::POST,
                "/api/apps/accounts",
                Some(Body::Raw(body)),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        assert_eq!(text, error(INVALID_APP_ACCOUNT));
        let (status, text) = fixture
            .send(
                Method::PUT,
                &format!("/api/apps/accounts/{id}"),
                Some(Body::Raw(json!({"token": {"x": token}}).to_string())),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        assert_eq!(text, error(INVALID_APP_ACCOUNT));
    }
}

#[tokio::test]
async fn vault_check_on_an_app_token_runs_its_test() {
    let fixture = Fixture::new().await;
    let token = canary();
    github_user(&fixture.upstream, &token, "octo").await;
    let id = fixture.connected(&token).await;
    let (status, text) = fixture
        .send(Method::POST, &format!("/api/vault/{id}/check"), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"ok": true, "message": "Connected as octo."})
    );
    // The test call reached the app, with the token.
    let tested = fixture
        .upstream
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/user")
        .count();
    assert_eq!(tested, 2, "the connect and the check");
    // Still the owner's only.
    let (status, _) = fixture
        .send(
            Method::POST,
            &format!("/api/vault/{id}/check"),
            None,
            Some(OTHER_PAGE),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[test]
fn a_keychain_that_needs_the_owner_gets_owner_copy() {
    use bruhs_core::apps::AppsError;
    use bruhs_core::secrets::SecretError;
    let owner = |error| match super::owner_error(error) {
        super::RouteError::App(error) => (error.status(), error.to_string()),
        super::RouteError::Status(status, message) => (status, message),
    };
    let (status, message) = owner(AppsError::Unavailable(
        bruhs_models::crew::KEYCHAIN_NEEDS_OWNER,
    ));
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(message, SecretError::Storage.to_string());
    assert_eq!(
        owner(AppsError::Vault(SecretError::NotSaved)),
        (
            StatusCode::NOT_FOUND,
            bruhs_models::vault::ENTRY_GONE.to_owned()
        )
    );
    assert_eq!(
        owner(AppsError::TestFailed("No.".into())),
        (StatusCode::UNPROCESSABLE_ENTITY, "No.".to_owned())
    );
    assert_eq!(
        owner(AppsError::TooLarge("Big.")).0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
