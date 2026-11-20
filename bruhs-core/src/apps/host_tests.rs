//! `AppsHost` against an in-memory vault and wiremock upstreams: a token is
//! saved only after its test call passes, and no listing, result or error
//! holds it.

use super::*;
use crate::connections::Connections;
use crate::secrets::SecretStore;
use bruhs_models::apps::{AppCall, ACCOUNT_CONNECTED, ANOTHER_ACCOUNT, UNKNOWN_APP};
use bruhs_models::crew::KEYCHAIN_NEEDS_OWNER;
use bruhs_models::vault::{
    CredentialScope, VaultCreate, VaultKind, VaultSecretInput, APPS_NOT_PROJECTS,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A random token of the shape the catalog accepts.
fn canary() -> String {
    format!("tok_{}", uuid::Uuid::new_v4().simple())
}

struct Fixture {
    server: MockServer,
    host: AppsHost,
}

async fn fixture_with(vault: SecretManager, mail: Option<MailRuntime>) -> Fixture {
    let server = MockServer::start().await;
    let http = AppHttp::with_upstream(*server.address()).unwrap();
    let origin = format!("http://{}", server.address());
    let gmail = Arc::new(GmailClient::for_test_origin(&origin).unwrap());
    let outlook = Arc::new(OutlookClient::for_test_origin(&origin).unwrap());
    let host = AppsHost::new(vault, http, mail, gmail, outlook);
    Fixture { server, host }
}

async fn fixture() -> Fixture {
    fixture_with(SecretManager::new(SecretStore::in_memory()), None).await
}

/// GitHub's `/user` names `login` for `token` and refuses every other one.
async fn github_user(server: &MockServer, token: &str, login: &str) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": login})))
        .with_priority(1)
        .mount(server)
        .await;
}

async fn refuse_the_rest(server: &MockServer) {
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"message": "Bad credentials"})),
        )
        .with_priority(10)
        .mount(server)
        .await;
}

fn github(token: &str) -> AppConnect {
    AppConnect {
        app: "github".into(),
        name: None,
        scope: CredentialScope::Global,
        params: BTreeMap::new(),
        token: token.into(),
        key: None,
    }
}

fn replace(token: &str) -> AppReplace {
    AppReplace {
        token: token.into(),
        key: None,
    }
}

async fn resolved(host: &AppsHost, id: &str) -> String {
    let secret = host.vault().resolve_app_token(id).await.unwrap();
    secret.token.as_str().to_owned()
}

fn failed(error: AppsError) -> String {
    match error {
        AppsError::TestFailed(message) => message,
        other => panic!("not a failed test: {other:?}"),
    }
}

#[tokio::test]
async fn connect_tests_first_and_saves_only_a_working_token() {
    let Fixture { server, host } = fixture().await;
    let (good, bad) = (canary(), canary());
    github_user(&server, &good, "octo").await;
    refuse_the_rest(&server).await;

    // A padded paste is stored trimmed.
    let entry = host.connect(github(&format!("  {good}\n"))).await.unwrap();
    assert_eq!(entry.name, "GitHub octo");
    let VaultKind::AppToken(meta) = &entry.kind else {
        panic!("{:?}", entry.kind);
    };
    assert_eq!(
        (meta.app.as_str(), meta.account.as_str()),
        ("github", "octo")
    );
    assert_eq!(resolved(&host, &entry.id).await, good);
    assert!(!json!(entry).to_string().contains(&good));

    let refused = host.connect(github(&bad)).await.unwrap_err();
    assert_eq!(
        failed(refused),
        "GitHub refused the token. Check it and what it may do."
    );
    assert_eq!(host.vault().app_tokens().await.unwrap().len(), 1);

    // The same account at the same scope twice is refused after its test.
    let again = host.connect(github(&good)).await.unwrap_err();
    assert!(
        matches!(again, AppsError::Invalid(message) if message == ACCOUNT_CONNECTED),
        "{again:?}"
    );
    // At another scope it is a new account, under the owner's name.
    let mut own = github(&good);
    own.scope = CredentialScope::Bruh {
        agent_id: "a".into(),
    };
    own.name = Some(" Octo for Ann ".into());
    let own = host.connect(own).await.unwrap();
    assert_eq!(own.name, "Octo for Ann");

    // Slack answers 200 with ok false for a bad token.
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok": false, "error": "invalid_auth"})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    let mut slack = github(&canary());
    slack.app = "slack".into();
    assert_eq!(
        failed(host.connect(slack).await.unwrap_err()),
        "Slack refused the token. Check it and what it may do."
    );

    // A project scope is refused before any call.
    let calls = server.received_requests().await.unwrap().len();
    let mut project = github(&good);
    project.scope = CredentialScope::Project {
        path: "/tmp/x".into(),
    };
    let error = host.connect(project).await.unwrap_err();
    assert!(
        matches!(error, AppsError::Invalid(message) if message == APPS_NOT_PROJECTS),
        "{error:?}"
    );
    // So are params the app doesn't take, and a short token.
    let mut stray = github(&good);
    stray.params.insert("site".into(), "acme".into());
    assert!(matches!(
        host.connect(stray).await.unwrap_err(),
        AppsError::Invalid(_)
    ));
    assert!(matches!(
        host.connect(github("short")).await.unwrap_err(),
        AppsError::Invalid(_)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), calls);
    assert_eq!(host.vault().app_tokens().await.unwrap().len(), 2);
}

#[tokio::test]
async fn failures_say_whether_the_app_answered() {
    let Fixture { server, host } = fixture().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    assert_eq!(
        failed(host.connect(github(&canary())).await.unwrap_err()),
        "GitHub answered in a way Bruhs didn't expect. Check the token's account and try again."
    );
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .mount(&server)
        .await;
    assert_eq!(
        failed(host.connect(github(&canary())).await.unwrap_err()),
        "GitHub answered in a way Bruhs didn't expect. Check the token's account and try again."
    );
    // A long label from the app is cut to fit, so the save still works.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "o".repeat(500)})))
        .mount(&server)
        .await;
    let entry = host.connect(github(&canary())).await.unwrap();
    let VaultKind::AppToken(meta) = &entry.kind else {
        panic!();
    };
    assert_eq!(
        meta.account.chars().count(),
        bruhs_models::vault::MAX_APP_ACCOUNT_CHARS
    );
    // A port nothing listens on: wiremock pools its servers, so a dropped
    // one would still answer.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let origin = format!("http://{}", server.address());
    let unreachable = AppsHost::new(
        SecretManager::new(SecretStore::in_memory()),
        AppHttp::with_upstream(closed).unwrap(),
        None,
        Arc::new(GmailClient::for_test_origin(&origin).unwrap()),
        Arc::new(OutlookClient::for_test_origin(&origin).unwrap()),
    );
    assert_eq!(
        failed(unreachable.connect(github(&canary())).await.unwrap_err()),
        "Bruhs couldn't reach GitHub. Check your connection and try again."
    );
}

#[tokio::test]
async fn replace_keeps_the_account_and_tests_the_new_token() {
    let Fixture { server, host } = fixture().await;
    let (old, refused, new, other) = (canary(), canary(), canary(), canary());
    github_user(&server, &old, "octo").await;
    github_user(&server, &new, "octo").await;
    github_user(&server, &other, "someone-else").await;
    refuse_the_rest(&server).await;
    let entry = host.connect(github(&old)).await.unwrap();

    assert_eq!(
        failed(
            host.replace(&entry.id, replace(&refused))
                .await
                .unwrap_err()
        ),
        "GitHub refused the token. Check it and what it may do."
    );
    assert_eq!(resolved(&host, &entry.id).await, old);

    assert_eq!(
        failed(host.replace(&entry.id, replace(&other)).await.unwrap_err()),
        ANOTHER_ACCOUNT
    );
    assert_eq!(resolved(&host, &entry.id).await, old);

    let replaced = host.replace(&entry.id, replace(&new)).await.unwrap();
    assert_eq!(
        (replaced.id.as_str(), replaced.kind),
        (entry.id.as_str(), entry.kind)
    );
    assert_eq!(resolved(&host, &entry.id).await, new);

    // Only an app token is replaced here.
    let key = host
        .vault()
        .vault_create(VaultCreate {
            name: "Key".into(),
            scope: CredentialScope::Global,
            secret: VaultSecretInput::Value {
                value: Some(canary()),
            },
        })
        .await
        .unwrap();
    let error = host.replace(&key.id, replace(&new)).await.unwrap_err();
    assert!(
        matches!(error, AppsError::Invalid(UNKNOWN_APP)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_trello_replace_keeps_its_key() {
    let Fixture { server, host } = fixture().await;
    let (key, old, new) = (canary(), canary(), canary());
    Mock::given(method("GET"))
        .and(path("/1/members/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"username": "ann"})))
        .mount(&server)
        .await;
    let mut connect = github(&old);
    connect.app = "trello".into();
    connect.key = Some(key.clone());
    let entry = host.connect(connect).await.unwrap();
    host.replace(&entry.id, replace(&new)).await.unwrap();
    let secret = host.vault().resolve_app_token(&entry.id).await.unwrap();
    assert_eq!(secret.token.as_str(), new);
    assert_eq!(
        secret.key.as_deref().map(String::as_str),
        Some(key.as_str())
    );
    // The replacement's test call carried the kept key.
    let last = server.received_requests().await.unwrap().pop().unwrap();
    let auth = last.headers.get("authorization").unwrap().to_str().unwrap();
    assert!(auth.contains(&key) && auth.contains(&new), "{auth}");
}

#[tokio::test]
async fn a_saved_account_tests_without_showing_its_token() {
    let vault =
        SecretManager::new(SecretStore::in_memory()).with_read_wait(Duration::from_millis(200));
    let Fixture { server, host } = fixture_with(vault.clone(), None).await;
    let good = canary();
    github_user(&server, &good, "octo").await;
    refuse_the_rest(&server).await;
    let entry = host.connect(github(&good)).await.unwrap();

    let result = host.test_saved(&entry.id).await.unwrap();
    assert_eq!(
        result,
        AppTestResult {
            ok: true,
            account: Some("octo".into()),
            message: "Connected as octo.".into(),
        }
    );
    assert!(!json!(result).to_string().contains(&good));

    // A token the app now refuses tests as refused, not as an error.
    server.reset().await;
    refuse_the_rest(&server).await;
    let result = host.test_saved(&entry.id).await.unwrap();
    assert_eq!(
        (result.ok, result.account, result.message.as_str()),
        (
            false,
            None,
            "GitHub refused the token. Check it and what it may do."
        )
    );

    // A keychain read that outlasts the vault's wait needs the owner.
    let held = vault.hold_store_for_tests().unwrap();
    let error = host.test_saved(&entry.id).await.unwrap_err();
    assert!(
        matches!(error, AppsError::Unavailable(message) if message == KEYCHAIN_NEEDS_OWNER),
        "{error:?}"
    );
    drop(held);
    assert!(matches!(
        host.test_saved("v1-missing").await.unwrap_err(),
        AppsError::Vault(_)
    ));
}

#[tokio::test]
async fn request_resolves_the_token_and_calls_the_declared_host() {
    let Fixture { server, host } = fixture().await;
    let good = canary();
    github_user(&server, &good, "octo").await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"full_name": "o/r"})))
        .mount(&server)
        .await;
    let entry = host.connect(github(&good)).await.unwrap();
    let call = AppCall {
        app: "github".into(),
        account: None,
        method: "GET".into(),
        path: "/repos/o/r".into(),
        query: BTreeMap::new(),
        body: None,
    };
    let reply = host.request(&entry, &call).await.unwrap();
    assert_eq!(
        (reply.status, reply.body.clone()),
        (200, json!({"full_name": "o/r"}))
    );
    let sent = server.received_requests().await.unwrap().pop().unwrap();
    assert_eq!(sent.url.path(), "/repos/o/r");
    assert_eq!(
        sent.headers.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {good}")
    );
    assert_eq!(
        sent.headers
            .get("x-bruhs-app-host")
            .unwrap()
            .to_str()
            .unwrap(),
        "api.github.com"
    );

    // An endpoint the allowlist doesn't name never leaves.
    let calls = server.received_requests().await.unwrap().len();
    let mut keys = call.clone();
    keys.method = "POST".into();
    keys.path = "/user/keys".into();
    assert!(matches!(
        host.request(&entry, &keys).await.unwrap_err(),
        AppsError::Invalid(_)
    ));
    // Nor does a call for another app on this token.
    let mut slack = call.clone();
    slack.app = "slack".into();
    assert!(matches!(
        host.request(&entry, &slack).await.unwrap_err(),
        AppsError::Invalid(UNKNOWN_APP)
    ));
    // An entry of another kind is no app.
    let key = host
        .vault()
        .vault_create(VaultCreate {
            name: "Key".into(),
            scope: CredentialScope::Global,
            secret: VaultSecretInput::Value {
                value: Some(canary()),
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        host.request(&key, &call).await.unwrap_err(),
        AppsError::Invalid(UNKNOWN_APP)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), calls);
}

#[tokio::test]
async fn the_listing_names_accounts_and_mailboxes_only() {
    let mail_token = canary();
    let connections = Connections::test_connected(ServiceProvider::Gmail, &mail_token).await;
    let folder = tempfile::tempdir().unwrap();
    let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let session = runtime
        .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap();
    let mailbox_id = session.authorization().mailbox().as_str().to_owned();
    drop(session);
    let vault = SecretManager::new(SecretStore::in_memory()).with_connections(connections);
    let Fixture { server, host } = fixture_with(vault, Some(runtime.clone())).await;
    let good = canary();
    github_user(&server, &good, "octo").await;
    let entry = host.connect(github(&good)).await.unwrap();
    host.vault()
        .vault_create(VaultCreate {
            name: "Key".into(),
            scope: CredentialScope::Global,
            secret: VaultSecretInput::Value {
                value: Some(canary()),
            },
        })
        .await
        .unwrap();

    let accounts = host.accounts().await.unwrap();
    assert_eq!(
        accounts,
        [AppAccount {
            id: entry.id.clone(),
            app: "github".into(),
            name: "GitHub octo".into(),
            account: "octo".into(),
            scope: CredentialScope::Global,
            state: entry.state,
            attention: None,
        }]
    );
    let mailboxes = host.mailboxes().await.unwrap();
    assert_eq!(mailboxes.len(), 1);
    assert_eq!(mailboxes[0].mailbox_id, mailbox_id);
    assert_eq!(mailboxes[0].provider, ServiceProvider::Gmail);
    assert!(!mailboxes[0].account.is_empty());
    let shown = format!("{}{}", json!(accounts), json!(mailboxes));
    assert!(
        !shown.contains(&good) && !shown.contains(&mail_token),
        "{shown}"
    );

    // A mailbox whose connection is gone isn't offered.
    let gone =
        SecretManager::new(SecretStore::in_memory()).with_connections(Connections::in_memory());
    let Fixture { host: other, .. } = fixture_with(gone, Some(runtime)).await;
    assert!(other.mailboxes().await.unwrap().is_empty());
    // With no mail runtime there are none.
    let Fixture { host: bare, .. } = fixture().await;
    assert!(bare.mailboxes().await.unwrap().is_empty());
}
