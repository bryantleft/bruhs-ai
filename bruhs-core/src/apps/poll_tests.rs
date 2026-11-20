//! App polls against wiremock upstreams and a synthetic Gmail connection:
//! a first poll only records where things stand, later ones emit what is
//! new, oldest first, at most `APP_POLL_MAX_ITEMS` at a time.

use super::*;
use crate::apps::AppHttp;
use crate::mail::compose::test_support::ConnectedMail;
use crate::mail::gmail::GmailClient;
use crate::mail::outlook::OutlookClient;
use crate::secrets::{SecretManager, SecretStore};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bruhs_models::vault::{VaultCreate, VaultSecretInput};
use std::sync::Arc;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn canary() -> String {
    format!("tok_{}", uuid::Uuid::new_v4().simple())
}

struct Github {
    server: MockServer,
    host: AppsHost,
    account: String,
    token: String,
}

fn host_at(server: &MockServer, vault: SecretManager, mail: Option<MailRuntime>) -> AppsHost {
    let http = AppHttp::with_upstream(*server.address()).unwrap();
    let origin = format!("http://{}", server.address());
    let gmail = Arc::new(GmailClient::for_test_origin(&origin).unwrap());
    let outlook = Arc::new(OutlookClient::for_test_origin(&origin).unwrap());
    AppsHost::new(vault, http, mail, gmail, outlook)
}

/// A host with one Global GitHub account, whose token the upstream takes.
async fn github() -> Github {
    let server = MockServer::start().await;
    let host = host_at(&server, SecretManager::new(SecretStore::in_memory()), None);
    let token = canary();
    let account = host
        .vault()
        .vault_create(VaultCreate {
            name: "GitHub octo".into(),
            scope: CredentialScope::Global,
            secret: VaultSecretInput::AppToken {
                app: "github".into(),
                account: "octo".into(),
                params: BTreeMap::new(),
                token: Some(token.clone()),
                key: None,
            },
        })
        .await
        .unwrap()
        .id;
    Github {
        server,
        host,
        account,
        token,
    }
}

fn item(number: u64, pull: bool) -> Value {
    let mut item = json!({
        "number": number,
        "title": format!("Item {number}\u{202e}"),
        "html_url": format!("https://github.com/o/r/issues/{number}"),
        "user": {"login": "ann"},
        "created_at": "2026-11-19T10:00:00Z",
        "labels": [{"name": "bug"}],
        "body": "b".repeat(800),
    });
    if pull {
        item["pull_request"] = json!({"url": "https://api.github.com/x"});
    }
    item
}

fn issues(numbers: &[u64]) -> Value {
    Value::Array(numbers.iter().map(|number| item(*number, false)).collect())
}

/// The upstream lists `items` at `list` (for `o/r` unless the path says
/// otherwise), newest first, for the account's token only.
async fn lists(fixture: &Github, list: &str, items: Value) {
    fixture.server.reset().await;
    Mock::given(method("GET"))
        .and(path(list))
        .and(query_param("state", "all"))
        .and(query_param("sort", "created"))
        .and(query_param("direction", "desc"))
        .and(query_param("per_page", "50"))
        .and(header(
            "authorization",
            format!("Bearer {}", fixture.token).as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(items))
        .mount(&fixture.server)
        .await;
}

fn watch(repo: &str) -> AppQuery {
    AppQuery::GithubIssues { repo: repo.into() }
}

async fn poll(fixture: &Github, query: &AppQuery, cursor: Option<Value>) -> PollOutcome {
    fixture
        .host
        .poll(
            "github",
            &fixture.account,
            query,
            cursor,
            &format!("{query:?}"),
        )
        .await
        .unwrap()
}

fn keys(outcome: &PollOutcome) -> Vec<&str> {
    outcome.events.iter().map(|(key, _)| key.as_str()).collect()
}

#[tokio::test]
async fn the_first_github_poll_records_a_baseline_and_emits_nothing() {
    let fixture = github().await;
    lists(
        &fixture,
        "/repos/o/r/issues",
        json!([item(8, true), item(7, false), item(6, false)]),
    )
    .await;
    let first = poll(&fixture, &watch("o/r"), None).await;
    assert!(first.events.is_empty());
    assert_eq!(first.cursor["newest"], 7);
    assert_eq!(first.status, None);
}

#[tokio::test]
async fn new_issues_emit_once_oldest_first() {
    let fixture = github().await;
    let query = watch("o/r");
    lists(&fixture, "/repos/o/r/issues", issues(&[7, 6])).await;
    let baseline = poll(&fixture, &query, None).await;

    lists(
        &fixture,
        "/repos/o/r/issues",
        json!([
            item(10, false),
            item(9, true),
            item(8, false),
            item(7, false)
        ]),
    )
    .await;
    let next = poll(&fixture, &query, Some(baseline.cursor)).await;
    assert_eq!(keys(&next), ["github:o/r:issue:8", "github:o/r:issue:10"]);
    let (_, payload) = &next.events[0];
    assert_eq!(payload["event"], "github_issue_opened");
    assert_eq!(payload["repo"], "o/r");
    assert_eq!(payload["number"], 8);
    // Controls are shown, never applied.
    assert_eq!(payload["title"], "Item 8\\u{202e}");
    assert_eq!(payload["url"], "https://github.com/o/r/issues/8");
    assert_eq!(payload["author"], "ann");
    assert_eq!(payload["created_at"], "2026-11-19T10:00:00Z");
    assert_eq!(payload["labels"], json!(["bug"]));
    assert_eq!(payload["excerpt"].as_str().unwrap().chars().count(), 500);
    assert_eq!(next.cursor["newest"], 10);

    let again = poll(&fixture, &query, Some(next.cursor.clone())).await;
    assert!(again.events.is_empty());
    assert_eq!(again.cursor, next.cursor);

    // 25 new issues: the oldest 20 now, the rest on the next poll.
    let many: Vec<u64> = (11..=35).rev().collect();
    lists(&fixture, "/repos/o/r/issues", issues(&many)).await;
    let burst = poll(&fixture, &query, Some(again.cursor)).await;
    assert_eq!(burst.events.len(), APP_POLL_MAX_ITEMS);
    assert_eq!(burst.events[0].0, "github:o/r:issue:11");
    assert_eq!(burst.events[19].0, "github:o/r:issue:30");
    assert_eq!(burst.cursor["newest"], 30);
    let rest = poll(&fixture, &query, Some(burst.cursor.clone())).await;
    assert_eq!(rest.events.len(), 5);
    assert_eq!(rest.events[0].0, "github:o/r:issue:31");

    // No event or cursor holds the token.
    for outcome in [&next, &burst, &rest] {
        let text = json!([outcome.events, outcome.cursor]).to_string();
        assert!(!text.contains(&fixture.token));
    }
}

#[tokio::test]
async fn a_changed_definition_starts_a_new_baseline() {
    let fixture = github().await;
    lists(&fixture, "/repos/o/r/issues", issues(&[7])).await;
    let baseline = poll(&fixture, &watch("o/r"), None).await;
    lists(&fixture, "/repos/o/other/issues", issues(&[12, 11, 3])).await;
    let moved = poll(&fixture, &watch("o/other"), Some(baseline.cursor.clone())).await;
    assert!(moved.events.is_empty());
    assert_eq!(moved.cursor["newest"], 12);
    // A cursor this build can't read starts over the same way.
    let unreadable = poll(
        &fixture,
        &watch("o/other"),
        Some(json!({"newest": 1, "extra": true})),
    )
    .await;
    assert!(unreadable.events.is_empty());
    assert_eq!(unreadable.cursor["newest"], 12);
}

#[tokio::test]
async fn pull_polls_read_pulls() {
    let fixture = github().await;
    let query = AppQuery::GithubPulls { repo: "o/r".into() };
    lists(
        &fixture,
        "/repos/o/r/pulls",
        json!([item(5, true), item(4, true)]),
    )
    .await;
    let baseline = poll(&fixture, &query, None).await;
    assert_eq!(baseline.cursor["newest"], 5);
    lists(
        &fixture,
        "/repos/o/r/pulls",
        json!([item(6, true), item(5, true)]),
    )
    .await;
    let next = poll(&fixture, &query, Some(baseline.cursor)).await;
    assert_eq!(keys(&next), ["github:o/r:pull:6"]);
    assert_eq!(next.events[0].1["event"], "github_pull_opened");
}

#[tokio::test]
async fn github_polls_take_only_a_global_github_account() {
    let fixture = github().await;
    let gone = |error: AppsError| matches!(error, AppsError::Refused(APP_POLL_ACCOUNT_GONE));
    let poll = |account: String| {
        let host = fixture.host.clone();
        async move {
            host.poll("github", &account, &watch("o/r"), None, "d")
                .await
                .err()
                .unwrap()
        }
    };
    assert!(gone(poll("v1-missing".into()).await));
    let own = fixture
        .host
        .vault()
        .vault_create(VaultCreate {
            name: "Ann's GitHub".into(),
            scope: CredentialScope::Bruh {
                agent_id: "a".into(),
            },
            secret: VaultSecretInput::AppToken {
                app: "github".into(),
                account: "ann".into(),
                params: BTreeMap::new(),
                token: Some(canary()),
                key: None,
            },
        })
        .await
        .unwrap();
    assert!(gone(poll(own.id).await));
    // A refused token is the test call's copy, without the token.
    fixture.server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string(fixture.token.clone()))
        .mount(&fixture.server)
        .await;
    let refused = poll(fixture.account.clone()).await.to_string();
    assert_eq!(
        refused,
        "GitHub refused the token. Check it and what it may do."
    );
    // Mail polls take mail, and GitHub polls GitHub.
    let mismatch = fixture
        .host
        .poll("mail", &fixture.account, &watch("o/r"), None, "d")
        .await
        .err()
        .unwrap();
    assert!(matches!(mismatch, AppsError::Invalid(POLL_WATCH_MISMATCH)));
}

/// A Gmail message as the API returns it, at `at_ms`.
fn gmail_message(id: &str, thread: &str, labels: &[&str], at_ms: i64, text: &str) -> Value {
    json!({
        "id": id,
        "threadId": thread,
        "internalDate": at_ms.to_string(),
        "labelIds": labels,
        "payload": {
            "partId": "",
            "mimeType": "text/plain",
            "headers": [
                {"name": "From", "value": "Bo Lee <bo@example.test>"},
                {"name": "To", "value": "you@example.test"},
                {"name": "Subject", "value": format!("About {thread}")},
                {"name": "Message-ID", "value": format!("<{id}@example.test>")},
                {"name": "Content-Type", "value": "text/plain; charset=utf-8"}
            ],
            "body": {"size": text.len(), "data": URL_SAFE_NO_PAD.encode(text)}
        }
    })
}

async fn serves_thread(server: &MockServer, id: &str, messages: Vec<Value>) {
    Mock::given(method("GET"))
        .and(path(format!("/gmail/v1/users/me/threads/{id}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": id, "messages": messages})),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn new_mail_syncs_then_emits_unread_threads() {
    let mail = ConnectedMail::gmail().await;
    let server = MockServer::start().await;
    let host = host_at(
        &server,
        SecretManager::new(SecretStore::in_memory()).with_connections(mail.connections.clone()),
        Some(mail.runtime.clone()),
    );
    let mailbox = mail.mailbox.as_str().to_owned();
    let now = EpochMs::now().as_millis();
    let inbox = ["INBOX", "UNREAD"];

    // The first poll syncs the mailbox's first page and starts at its
    // newest unread thread.
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/profile"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"emailAddress": "you@example.test", "historyId": "100"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/threads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"threads": [{"id": "old"}]})))
        .mount(&server)
        .await;
    let old_ms = now - 3_600_000;
    serves_thread(
        &server,
        "old",
        vec![gmail_message("old-m", "old", &inbox, old_ms, "Old")],
    )
    .await;
    let query = AppQuery::NewMail;
    let first = host
        .poll("mail", &mailbox, &query, None, "mail")
        .await
        .unwrap();
    assert!(first.events.is_empty());
    assert_eq!(first.status, None);
    assert_eq!(first.cursor["after_ms"], old_ms);

    // Then the history brings new mail: one unread thread, and others
    // that read, filed or answered by the owner don't count.
    let later = now + 60_000;
    let added: Vec<Value> = ["fresh", "seen", "filed", "mine"]
        .iter()
        .map(|thread| json!({"message": {"id": format!("{thread}-m"), "threadId": thread}}))
        .collect();
    Mock::given(method("GET"))
        .and(path("/gmail/v1/users/me/history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "historyId": "105",
            "history": [{"messagesAdded": added}]
        })))
        .mount(&server)
        .await;
    let preview = format!("Hello\n{}", "p".repeat(400));
    serves_thread(
        &server,
        "fresh",
        vec![gmail_message("fresh-m", "fresh", &inbox, later, &preview)],
    )
    .await;
    serves_thread(
        &server,
        "seen",
        vec![gmail_message("seen-m", "seen", &["INBOX"], later, "Read")],
    )
    .await;
    serves_thread(
        &server,
        "filed",
        vec![gmail_message(
            "filed-m",
            "filed",
            &["UNREAD"],
            later,
            "Filed",
        )],
    )
    .await;
    serves_thread(
        &server,
        "mine",
        vec![
            gmail_message("mine-m", "mine", &inbox, later, "Question"),
            gmail_message("mine-r", "mine", &["SENT"], later + 1_000, "Answer"),
        ],
    )
    .await;
    let next = host
        .poll("mail", &mailbox, &query, Some(first.cursor), "mail")
        .await
        .unwrap();
    assert_eq!(next.status, None);
    let fresh = mail.mailbox.message("fresh-m").unwrap().canonical_id;
    assert_eq!(
        keys(&next),
        [format!("mail:{mailbox}:fresh:{later}").as_str()]
    );
    let payload = &next.events[0].1;
    assert_eq!(payload["event"], "new_mail");
    assert_eq!(payload["mailbox"], mailbox.as_str());
    assert_eq!(payload["account"], "you@example.test");
    assert_eq!(payload["external_thread_id"], "fresh");
    assert_eq!(payload["message_id"], fresh.as_str());
    assert_eq!(payload["sender"], "Bo Lee");
    assert_eq!(payload["subject"], "About fresh");
    let shown = payload["preview"].as_str().unwrap();
    assert!(shown.starts_with("Hello\\np"), "{shown}");
    assert_eq!(shown.chars().count(), 300);
    assert_eq!(payload["message_at_ms"], later);
    assert_eq!(next.cursor["after_ms"], later);

    // Nothing new: nothing emitted.
    let again = host
        .poll("mail", &mailbox, &query, Some(next.cursor.clone()), "mail")
        .await
        .unwrap();
    assert!(again.events.is_empty());
    assert_eq!(again.cursor, next.cursor);

    // A failed sync still reads the cache, and its copy is the status.
    server.reset().await;
    let failed = host
        .poll("mail", &mailbox, &query, Some(next.cursor), "mail")
        .await
        .unwrap();
    assert!(failed.events.is_empty());
    assert!(failed.status.is_some());

    // A mailbox that isn't connected is gone.
    let gone = host
        .poll("mail", "gmail:nobody", &query, None, "mail")
        .await
        .err()
        .unwrap();
    assert!(matches!(gone, AppsError::Refused(APP_POLL_ACCOUNT_GONE)));
}

#[test]
fn another_sync_saving_first_is_no_problem() {
    assert_eq!(
        sync_problem(MailError::Cache(CacheError::RevisionConflict)),
        None
    );
    assert_eq!(
        sync_problem(MailError::Storage),
        Some(MailError::Storage.to_string())
    );
}
