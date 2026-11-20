//! The bruh's app and mail route over HTTP: claimed runs whose calls the
//! gate let through (`gate_app_call_for_tests`), a wiremock upstream for
//! every app, and the shared fake mail transport. Nothing reaches a real
//! app or mailbox.

use std::time::Duration;

use axum::http::StatusCode;
use bruhs_core::mail::compose::test_support::{FakeTransport, Mode};
use bruhs_core::PermissionDecision;
use bruhs_models::apps::REDACTED;
use bruhs_models::crew::{
    AccessLevel, AppGrant, APP_NOT_GRANTED, APP_REQUEST_TOOL, BRUH_NO_TOOLS, CALL_FROM_A_PAGE,
    CALL_NOT_APPROVED, KEYCHAIN_NEEDS_OWNER, LIST_APPS_TOOL, MAIL_MAY_HAVE_SENT, MAIL_NOT_GRANTED,
    MAIL_SENT, SEND_MAIL_TOOL,
};
use bruhs_models::reactive::{ReactiveCommand, ReactiveRun};
use bruhs_models::vault::OWNER_ONLY_CARD;
use reqwest::Method;
use serde_json::{json, Value};
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use super::{answer, fits, Budget, RouteError, APP_READ_BUDGET, ROUTE_BUDGET};
use crate::routes::apps::tests::{canary, github_user, Body, Fixture, Logs, MailSetup, OTHER_PAGE};

const ROUTE: &str = "/api/reactive/agents/a/apps";

/// The issues read every app test sends.
fn issues() -> Value {
    json!({"app": "github", "method": "GET", "path": "/repos/o/r/issues", "query": {"per_page": 1}})
}

fn short_name(tool: &str) -> &str {
    tool.trim_start_matches("mcp__bruhs_workbench__")
}

impl Fixture {
    /// A running run of `a`, as the worker would have claimed it.
    async fn running(&self) -> ReactiveRun {
        self.command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Triage the issues".into(),
            run_id: None,
        })
        .await;
        self.runtime.claim_run_for_tests().await.unwrap().unwrap()
    }

    /// The gate's answer to `tool` with `input` from `run`, which leaves
    /// an allowance when it allows the call.
    async fn gate(&self, run: &ReactiveRun, tool: &str, input: &Value) -> PermissionDecision {
        self.runtime
            .gate_app_call_for_tests(run, tool, input)
            .await
            .unwrap()
            .unwrap()
    }

    /// The helper's call of `tool` with `input`, from `origin` (none: the
    /// helper).
    async fn helper(
        &self,
        tool: &str,
        input: &Value,
        origin: Option<&str>,
    ) -> (StatusCode, String) {
        let body = json!({"tool": short_name(tool), "input": input});
        self.send(Method::POST, ROUTE, Some(Body::Json(body)), origin)
            .await
    }

    /// GitHub `/repos/o/r/issues` answers with the Authorization header it
    /// got, in its body and a header.
    async fn echo_issues(&self) {
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues"))
            .respond_with(|request: &wiremock::Request| {
                let auth = request
                    .headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                ResponseTemplate::new(200)
                    .insert_header("link", auth.as_str())
                    .set_body_json(json!([{"title": "Echo", "auth": auth}]))
            })
            .mount(&self.upstream)
            .await;
    }

    /// Grants (or takes back) the synthetic mailbox to `a`.
    async fn grant_mailbox(&self, granted: bool) {
        let mailbox_id = self.mail.as_ref().unwrap().mailbox.as_str().to_owned();
        self.command(ReactiveCommand::SetAppGrant {
            agent_id: "a".into(),
            grant: AppGrant::Mailbox { mailbox_id },
            granted,
        })
        .await;
    }

    /// How many calls the app got, besides the token's test calls.
    async fn app_calls(&self) -> usize {
        self.upstream
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() != "/user")
            .count()
    }

    /// A Global GitHub account for `login`, granted to `a`: its id and
    /// token.
    async fn granted_github(&self, login: &str) -> (String, String) {
        let token = canary();
        github_user(&self.upstream, &token, login).await;
        let id = self.connected(&token).await;
        self.grant_token(&id, true).await;
        (id, token)
    }
}

fn error(text: &str) -> String {
    json!({ "error": text }).to_string()
}

fn value(text: &str) -> Value {
    let reply: Value = serde_json::from_str(text).unwrap();
    assert_eq!(reply["image"], Value::Null, "{text}");
    reply["value"].clone()
}

#[tokio::test]
async fn an_app_call_reaches_the_app_and_its_token_never_comes_back() {
    let logs = Logs::default();
    let _capture = logs.capture();
    let fixture = Fixture::new().await;
    Logs::refresh();
    let (_, token) = fixture.granted_github("octo").await;
    fixture.echo_issues().await;
    let run = fixture.running().await;
    assert_eq!(
        fixture.gate(&run, APP_REQUEST_TOOL, &issues()).await,
        PermissionDecision::Allow
    );

    Logs::refresh();
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(!text.contains(&token), "{text}");
    assert!(text.contains(REDACTED), "{text}");
    let response = value(&text);
    assert_eq!(response["status"], 200);
    assert_eq!(fixture.app_calls().await, 1);
    // The app did get the token.
    let sent = fixture.upstream.received_requests().await.unwrap();
    let call = sent
        .iter()
        .find(|request| request.url.path() == "/repos/o/r/issues")
        .unwrap();
    assert_eq!(call.url.query(), Some("per_page=1"));
    assert_eq!(
        call.headers.get("authorization").unwrap(),
        format!("Bearer {token}").as_str()
    );

    // Not in the stored run, the snapshot or the server's logs.
    let snapshot = serde_json::to_string(&fixture.runtime.snapshot().await.unwrap()).unwrap();
    assert!(!snapshot.contains(&token));
    assert!(snapshot.contains(&run.id), "the run is in the snapshot");
    let logged = logs.text();
    assert!(
        logged.contains(ROUTE),
        "the server logged nothing: {logged}"
    );
    assert!(!logged.contains(&token), "{logged}");
}

#[tokio::test]
async fn the_helper_route_uses_each_allowance_once() {
    let fixture = Fixture::new().await;
    fixture.granted_github("octo").await;
    fixture.echo_issues().await;

    // A call the gate never saw.
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert_eq!(text, error(CALL_NOT_APPROVED));

    let run = fixture.running().await;
    fixture.gate(&run, APP_REQUEST_TOOL, &issues()).await;
    // A page posting the very call it saw is refused and uses nothing up.
    let (status, text) = fixture
        .helper(APP_REQUEST_TOOL, &issues(), Some(OTHER_PAGE))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert_eq!(text, error(CALL_FROM_A_PAGE));
    let (status, text) = fixture
        .helper(APP_REQUEST_TOOL, &issues(), Some(&fixture.base.clone()))
        .await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_FROM_A_PAGE))
    );
    assert_eq!(fixture.app_calls().await, 0);

    // Matched as sent: 1.0 isn't the 1 the gate saw.
    let mut other = issues();
    other["query"]["per_page"] = json!(1.0);
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &other, None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_NOT_APPROVED))
    );
    // Nor is another tool's allowance.
    let (status, text) = fixture.helper(LIST_APPS_TOOL, &issues(), None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_NOT_APPROVED))
    );

    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    // Each allowed call runs once.
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_NOT_APPROVED))
    );
    assert_eq!(fixture.app_calls().await, 1);

    // list_apps through the route: the bruh's accounts, no token or id.
    fixture.gate(&run, LIST_APPS_TOOL, &json!({})).await;
    let (status, text) = fixture.helper(LIST_APPS_TOOL, &json!({}), None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        value(&text),
        json!({"apps": [{"app": "github", "name": "GitHub", "accounts": [
            {"name": "GitHub octo", "account": "octo", "base_url": "https://api.github.com"},
        ]}], "mail": []})
    );
}

#[tokio::test]
async fn a_grant_revoked_after_the_gate_stops_the_call() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.granted_github("octo").await;
    fixture.echo_issues().await;
    let run = fixture.running().await;
    fixture.gate(&run, APP_REQUEST_TOOL, &issues()).await;
    fixture.grant_token(&id, false).await;
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(APP_NOT_GRANTED))
    );
    assert_eq!(fixture.app_calls().await, 0);
}

/// Ruling F10: the gate picked "work"; once grants change, the same input
/// resolves to "octo", and the route sends nothing as octo.
#[tokio::test]
async fn the_route_calls_only_the_account_the_gate_picked() {
    let fixture = Fixture::new().await;
    let (work, _) = fixture.granted_github("work").await;
    fixture.echo_issues().await;
    let run = fixture.running().await;
    fixture.gate(&run, APP_REQUEST_TOOL, &issues()).await;
    let octo = canary();
    github_user(&fixture.upstream, &octo, "octo").await;
    let octo_id = fixture.connected(&octo).await;
    fixture.grant_token(&octo_id, true).await;
    fixture.grant_token(&work, false).await;
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(APP_NOT_GRANTED))
    );
    assert_eq!(fixture.app_calls().await, 0);
    // The refusal spent the allowance: with "work" back, the same call
    // still needs the gate again.
    fixture.grant_token(&octo_id, false).await;
    fixture.grant_token(&work, true).await;
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_NOT_APPROVED))
    );
    assert_eq!(fixture.app_calls().await, 0);
}

/// A lookup that outlasts the read budget sends nothing: the app's
/// account list, then the mailbox list. The budgets are short here.
#[tokio::test]
async fn slow_lookups_send_nothing() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.granted_github("octo").await;
    fixture.echo_issues().await;
    let short = Budget {
        read: Duration::from_millis(300),
        ..Budget::ROUTE
    };
    let held = fixture.state.secrets().hold_store_for_tests().unwrap();
    let started = std::time::Instant::now();
    let refused = answer(
        &fixture.runtime,
        "a",
        APP_REQUEST_TOOL,
        &issues(),
        Some(&id),
        Instant::now(),
        short,
    )
    .await;
    drop(held);
    match refused {
        Err(RouteError::App(error)) => {
            assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(error.to_string(), bruhs_models::apps::APP_UNREACHABLE);
        }
        other => panic!("expected 503, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(fixture.app_calls().await, 0);
    drop(fixture);

    let transport = FakeTransport::new(Mode::Accepted, false);
    let fixture = Fixture::with_mail(Some(MailSetup {
        transport: Some(transport.clone()),
        send_budget: None,
    }))
    .await;
    fixture.set_access(AccessLevel::Full, true).await;
    fixture.grant_mailbox(true).await;
    let mailbox_id = fixture.mail.as_ref().unwrap().mailbox.as_str().to_owned();
    // The send's window leaves the lookup 300 ms.
    let short = Budget {
        mail: ROUTE_BUDGET - Duration::from_millis(300),
        ..Budget::ROUTE
    };
    let held = fixture
        .mail
        .as_ref()
        .unwrap()
        .runtime
        .hold_cache_for_tests()
        .await;
    let started = std::time::Instant::now();
    let refused = answer(
        &fixture.runtime,
        "a",
        SEND_MAIL_TOOL,
        &json!({"to": "ann@x.test", "subject": "Lunch", "body": "See you at 12."}),
        Some(&mailbox_id),
        Instant::now(),
        short,
    )
    .await;
    drop(held);
    match refused {
        Err(RouteError::App(error)) => {
            assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(error.to_string(), bruhs_models::crew::MAIL_NOT_SENT);
        }
        other => panic!("expected 503, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(transport.sends(), 0);
}

/// A lowered level cancels the run, so its allowance goes; the route also
/// runs the gate again at the bruh's stored access, which now refuses.
#[tokio::test]
async fn a_lowered_level_stops_the_call() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.granted_github("octo").await;
    fixture.echo_issues().await;
    let run = fixture.running().await;
    fixture.gate(&run, APP_REQUEST_TOOL, &issues()).await;
    fixture.set_access(AccessLevel::ReadOnly, false).await;
    let (status, text) = fixture.helper(APP_REQUEST_TOOL, &issues(), None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_NOT_APPROVED))
    );
    let regated = answer(
        &fixture.runtime,
        "a",
        APP_REQUEST_TOOL,
        &issues(),
        Some(&id),
        Instant::now(),
        Budget::ROUTE,
    )
    .await;
    match regated {
        Err(RouteError::App(error)) => {
            assert_eq!(error.status(), StatusCode::FORBIDDEN);
            assert_eq!(error.to_string(), bruhs_models::crew::APPS_NEED_ASK);
        }
        other => panic!("expected 403, got {other:?}"),
    }
    assert_eq!(fixture.app_calls().await, 0);
}

#[tokio::test]
async fn send_mail_through_the_route() {
    let lunch = json!({"to": "ann@x.test", "subject": "Lunch", "body": "See you at 12."});
    let sending = |transport| {
        Fixture::with_mail(Some(MailSetup {
            transport: Some(transport),
            send_budget: Some(Duration::from_millis(500)),
        }))
    };
    let transport = FakeTransport::new(Mode::Accepted, false);
    let fixture = sending(transport.clone()).await;
    fixture.set_access(AccessLevel::Full, true).await;
    fixture.grant_mailbox(true).await;
    let run = fixture.running().await;
    assert_eq!(
        fixture.gate(&run, SEND_MAIL_TOOL, &lunch).await,
        PermissionDecision::Allow
    );
    let (status, text) = fixture.helper(SEND_MAIL_TOOL, &lunch, None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let outcome = value(&text);
    assert_eq!(outcome["state"], "accepted");
    assert_eq!(outcome["message"], MAIL_SENT);
    assert_eq!(transport.sends(), 1);
    // Once.
    let (status, text) = fixture.helper(SEND_MAIL_TOOL, &lunch, None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(CALL_NOT_APPROVED))
    );
    assert_eq!(transport.sends(), 1);

    // A mailbox taken back after the gate sends nothing.
    fixture.gate(&run, SEND_MAIL_TOOL, &lunch).await;
    fixture.grant_mailbox(false).await;
    let (status, text) = fixture.helper(SEND_MAIL_TOOL, &lunch, None).await;
    assert_eq!(
        (status, text),
        (StatusCode::FORBIDDEN, error(MAIL_NOT_GRANTED))
    );
    assert_eq!(transport.sends(), 1);
    drop(fixture);

    // A send slower than its budget answers in time that it may have gone.
    let slow = FakeTransport::slow_send(Mode::Accepted, Duration::from_secs(10));
    let fixture = sending(slow.clone()).await;
    fixture.set_access(AccessLevel::Full, true).await;
    fixture.grant_mailbox(true).await;
    let run = fixture.running().await;
    fixture.gate(&run, SEND_MAIL_TOOL, &lunch).await;
    let started = std::time::Instant::now();
    let (status, text) = fixture.helper(SEND_MAIL_TOOL, &lunch, None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(value(&text)["message"], MAIL_MAY_HAVE_SENT);
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(slow.sends(), 1);
}

/// Ruling F11: a call whose whole timeout no longer fits in the route's
/// budget is never sent. The budget is short here; the route has 55 s.
#[tokio::test]
async fn a_call_that_no_longer_fits_is_never_sent() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.granted_github("octo").await;
    fixture.echo_issues().await;
    let short = Budget {
        total: Duration::from_secs(1),
        ..Budget::ROUTE
    };
    let refused = answer(
        &fixture.runtime,
        "a",
        APP_REQUEST_TOOL,
        &issues(),
        Some(&id),
        Instant::now(),
        short,
    )
    .await;
    match refused {
        Err(RouteError::App(error)) => {
            assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(error.to_string(), KEYCHAIN_NEEDS_OWNER);
        }
        other => panic!("expected 503, got {other:?}"),
    }
    assert_eq!(fixture.app_calls().await, 0);
    // With the route's budget it goes.
    let sent = answer(
        &fixture.runtime,
        "a",
        APP_REQUEST_TOOL,
        &issues(),
        Some(&id),
        Instant::now(),
        Budget::ROUTE,
    )
    .await
    .unwrap();
    assert_eq!(sent["status"], 200);
    assert_eq!(fixture.app_calls().await, 1);

    // A bruh the runtime doesn't have gets no tools.
    let missing = answer(
        &fixture.runtime,
        "nobody",
        LIST_APPS_TOOL,
        &json!({}),
        None,
        Instant::now(),
        Budget::ROUTE,
    )
    .await;
    match missing {
        Err(RouteError::Status(status, message)) => {
            assert_eq!(
                (status, message.as_str()),
                (StatusCode::NOT_FOUND, BRUH_NO_TOOLS)
            );
        }
        other => panic!("expected 404, got {other:?}"),
    }
}

/// Ruling F11 in paused time: the reads may take their whole budget and a
/// call still fits; past the last moment it doesn't start.
#[tokio::test(start_paused = true)]
async fn a_call_starts_only_while_its_whole_timeout_fits() {
    let started = Instant::now();
    let deadline = started + ROUTE_BUDGET;
    let call = Budget::ROUTE.call;
    assert!(ROUTE_BUDGET < bruhs_core::capture_client::REQUEST_TIMEOUT);
    assert!(APP_READ_BUDGET + call <= ROUTE_BUDGET);
    assert!(Budget::ROUTE.mail < ROUTE_BUDGET);
    tokio::time::advance(APP_READ_BUDGET).await;
    assert!(fits(Instant::now(), deadline, call));
    // 32 s left: the call's 30 s and the 2 s margin.
    tokio::time::advance(ROUTE_BUDGET - APP_READ_BUDGET - call).await;
    assert!(fits(Instant::now(), deadline, call));
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(!fits(Instant::now(), deadline, call));
    tokio::time::advance(ROUTE_BUDGET).await;
    assert!(!fits(Instant::now(), deadline, call));
}

/// A Task 12: deciding an app call's card takes the owner's own app.
#[tokio::test]
async fn an_app_card_is_answered_only_from_bruhs_own_pages() {
    let fixture = Fixture::new().await;
    let run = fixture.running().await;
    let card = json!({
        "app": "github", "method": "POST", "path": "/repos/o/r/issues",
        "body": {"title": "Bug"}, "as": "octo", "as_id": "entry",
    });
    let item = fixture
        .runtime
        .open_approval_for_tests(&run, APP_REQUEST_TOOL, &card)
        .await
        .unwrap();
    let decide = json!({"action": "decide_approval", "id": item, "approve": true});
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/reactive/commands",
            Some(Body::Json(decide.clone())),
            Some(OTHER_PAGE),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(text.contains(OWNER_ONLY_CARD), "{text}");
    let own = fixture.base.clone();
    let (status, text) = fixture
        .send(
            Method::POST,
            "/api/reactive/commands",
            Some(Body::Json(decide)),
            Some(&own),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
}
