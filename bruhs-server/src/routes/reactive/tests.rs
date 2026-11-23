use super::*;
use serde_json::Value;
use std::sync::Arc;

struct Fixture {
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

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(directory.path().join("automation.sqlite3"))
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
        .layer(Extension(runtime.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            runtime,
            base,
            client,
            server,
            directory,
        }
    }

    async fn command(&self, value: Value) -> Value {
        let response = self
            .client
            .post(format!("{}/api/reactive/commands", self.base))
            .json(&value)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert!(status.is_success(), "{status}: {body}");
        body
    }

    async fn configured(&self) {
        self.command(json!({"action":"apply","change":{"type":"save_agent","agent":{"id":"bills","name":"Bills","instructions":"Extract a payment draft. Never pay.","model":"sonnet","project_path":self.directory.path(),"enabled":true}}})).await;
        self.command(json!({"action":"apply","change":{"type":"save_source","source":{"id":"scans","name":"Scans","enabled":true,"kind":{"type":"webhook"}}}})).await;
        self.command(json!({"action":"apply","change":{"type":"save_subscription","subscription":{"id":"scan-bills","source_id":"scans","agent_id":"bills","instructions":"Summarize the scan","enabled":true}}})).await;
    }
}

#[tokio::test]
async fn hook_requires_its_token_and_deduplicates_real_http_deliveries() {
    let fixture = Fixture::new().await;
    fixture.configured().await;
    let hook = format!("{}/api/reactive/hooks/scans", fixture.base);
    let missing = fixture
        .client
        .post(&hook)
        .json(&json!({"file":"bill.pdf"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    let wrong = fixture
        .client
        .post(&hook)
        .bearer_auth("wrong")
        .json(&json!({"file":"bill.pdf"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    assert!(fixture.runtime.snapshot().await.unwrap().events.is_empty());

    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    for _ in 0..2 {
        let accepted = fixture
            .client
            .post(&hook)
            .bearer_auth(&token)
            .header("x-event-id", "scanner-delivery-1")
            .json(&json!({"file":"bill.pdf"}))
            .send()
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    }
    let snapshot = fixture.runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].status,
        bruhs_models::reactive::RunStatus::AwaitingReview
    );
    assert!(!serde_json::to_string(&snapshot).unwrap().contains(&token));
}

#[tokio::test]
async fn configuration_proposal_is_reviewable_and_never_applies_on_submission() {
    let fixture = Fixture::new().await;
    let snapshot = fixture.command(json!({"action":"propose","change":{"type":"save_source","source":{"id":"timer","name":"Timer","enabled":false,"kind":{"type":"schedule","every_ms":60_000}}}})).await;
    assert!(snapshot["sources"].as_array().unwrap().is_empty());
    assert_eq!(snapshot["proposals"][0]["status"], "pending");
    assert_eq!(
        snapshot["proposals"][0]["change"]["source"]["kind"]["every_ms"],
        60_000
    );
    let id = snapshot["proposals"][0]["id"].as_str().unwrap();
    let applied = fixture
        .command(json!({"action":"decide_proposal","id":id,"approve":true}))
        .await;
    assert_eq!(applied["sources"][0]["id"], "timer");
    assert_eq!(applied["proposals"][0]["status"], "approved");
}

#[tokio::test]
async fn one_setup_review_activates_the_complete_automation_and_one_switch_pauses_it() {
    let fixture = Fixture::new().await;
    let proposal = fixture.command(json!({"action":"propose","change":{"type":"save_automation","automation":{
        "agent":{"id":"clerk","name":"Scan assistant","instructions":"Extract a draft","model":"sonnet","project_path":fixture.directory.path(),"enabled":true},
        "source":{"id":"scans","name":"Scan events","enabled":true,"kind":{"type":"custom"}},
        "subscription":{"id":"scan-automation","source_id":"scans","agent_id":"clerk","instructions":"Summarize","enabled":true,"auto_run":true}
    }}})).await;
    assert!(proposal["agents"].as_array().unwrap().is_empty());
    assert!(proposal["sources"].as_array().unwrap().is_empty());
    assert!(proposal["subscriptions"].as_array().unwrap().is_empty());
    let proposal_id = proposal["proposals"][0]["id"].as_str().unwrap();
    let active = fixture
        .command(json!({"action":"decide_proposal","id":proposal_id,"approve":true}))
        .await;
    assert_eq!(active["agents"][0]["enabled"], true);
    assert_eq!(active["sources"][0]["enabled"], true);
    assert_eq!(active["subscriptions"][0]["enabled"], true);
    assert_eq!(active["subscriptions"][0]["auto_run"], true);
    let event = fixture.command(json!({"action":"publish","source_id":"scans","key":"first","payload":{"fixture":true}})).await;
    assert_eq!(event["runs"][0]["status"], "queued");
    let paused = fixture
        .command(json!({"action":"set_automation_enabled","id":"scan-automation","enabled":false}))
        .await;
    assert_eq!(paused["subscriptions"][0]["enabled"], false);
    assert_eq!(paused["agents"][0]["enabled"], true);
    assert_eq!(paused["runs"][0]["status"], "cancelled");
    assert!(fixture.runtime.source_configs().await.unwrap().is_empty());
    fixture
        .command(json!({"action":"set_automation_enabled","id":"scan-automation","enabled":true}))
        .await;
    let resumed = fixture.command(json!({"action":"publish","source_id":"scans","key":"second","payload":{"fixture":true}})).await;
    assert_eq!(resumed["runs"][0]["status"], "queued");
}

#[tokio::test]
async fn foreign_pages_cannot_read_or_configure_local_automations() {
    let fixture = Fixture::new().await;
    let response = fixture
        .client
        .get(format!("{}/api/reactive", fixture.base))
        .header("origin", "https://untrusted.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = fixture
        .client
        .post(format!("{}/api/reactive/commands", fixture.base))
        .header("origin", "https://untrusted.example")
        .json(&json!({"action":"publish","source_id":"missing","key":"x","payload":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(fixture.runtime.snapshot().await.unwrap().events.is_empty());
}

/// Imports final review B I1: a page that isn't the owner's never reads a
/// bruh's memory or a pending memory proposal, and can't approve or apply
/// one; its pause keeps the memory it was sent blank.
#[tokio::test]
async fn other_pages_never_read_or_approve_a_bruhs_memory() {
    const OTHER_PAGE: &str = "http://localhost:5173";
    const STORED: &str = "Owner notes canary 41c2.";
    const IMPORTED: &str = "Imported memory canary 9d7e.";
    let fixture = Fixture::new().await;
    fixture.configured().await;
    let memory = |text: &str| json!({"type":"set_memory","agent_id":"bills","memory":text});
    fixture
        .command(json!({"action":"apply","change":memory(STORED)}))
        .await;
    let proposed = fixture
        .command(json!({"action":"propose","change":memory(IMPORTED)}))
        .await;
    let proposal = proposed["proposals"][0]["id"].as_str().unwrap().to_owned();
    let other = |method: reqwest::Method, path: &str| {
        fixture
            .client
            .request(method, format!("{}{path}", fixture.base))
            .header("origin", OTHER_PAGE)
    };
    let stored = || async { fixture.runtime.snapshot().await.unwrap().agents[0].clone() };

    let read = other(reqwest::Method::GET, "/api/reactive")
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let text = read.text().await.unwrap();
    assert!(text.contains(&proposal), "{text}");
    assert!(!text.contains(IMPORTED) && !text.contains(STORED), "{text}");

    for command in [
        json!({"action":"decide_proposal","id":proposal,"approve":true}),
        json!({"action":"apply","change":memory("Other.")}),
    ] {
        let refused = other(reqwest::Method::POST, "/api/reactive/commands")
            .json(&command)
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{command}");
        let body: Value = refused.json().await.unwrap();
        assert_eq!(body["error"], bruhs_models::imports::OWNER_ONLY_MEMORY);
        assert_eq!(stored().await.memory, STORED);
    }

    let mut paused = serde_json::to_value(stored().await).unwrap();
    paused["memory"] = json!("");
    paused["enabled"] = json!(false);
    let pause = other(reqwest::Method::POST, "/api/reactive/commands")
        .json(&json!({"action":"apply","change":{"type":"save_agent","agent":paused}}))
        .send()
        .await
        .unwrap();
    assert_eq!(pause.status(), StatusCode::OK);
    let text = pause.text().await.unwrap();
    assert!(!text.contains(IMPORTED) && !text.contains(STORED), "{text}");
    let agent = stored().await;
    assert!(!agent.enabled);
    assert_eq!(agent.memory, STORED);

    // The owner's own app (no Origin, as the desktop sends) approves it.
    let approved = fixture
        .command(json!({"action":"decide_proposal","id":proposal,"approve":true}))
        .await;
    assert_eq!(approved["agents"][0]["memory"], IMPORTED);
}

fn bruh(id: &str, name: &str, dir: &std::path::Path) -> Value {
    json!({"action":"create_bruh","agent":{"id":id,"name":name,"instructions":"Help.","model":"sonnet","project_path":dir,"enabled":true}})
}

#[tokio::test]
async fn the_reactive_stream_sends_bruh_states() {
    let fixture = Fixture::new().await;
    fixture
        .command(bruh("pip", "Pip", fixture.directory.path()))
        .await;
    let mut response = fixture
        .client
        .get(format!("{}/api/reactive/stream", fixture.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut text = String::new();
    let frame = loop {
        let chunk = response.chunk().await.unwrap().expect("stream ended");
        text.push_str(&String::from_utf8_lossy(&chunk));
        if let Some(data) = text.lines().find_map(|line| line.strip_prefix("data:")) {
            break data.trim().to_string();
        }
    };
    assert!(
        !text.contains("event:"),
        "frames are unnamed on this stream: {text}"
    );
    let signal: bruhs_models::reactive::OutboxSignal = serde_json::from_str(&frame).unwrap();
    assert_eq!(signal.protocol, bruhs_models::crew::CREW_PROTOCOL);
    assert_eq!(signal.bruhs.len(), 1);
    assert_eq!(signal.bruhs[0].agent_id, "pip");
    assert_eq!(signal.bruhs[0].name, "Pip");
}

#[tokio::test]
async fn agent_runs_route_pages_one_agents_runs() {
    let fixture = Fixture::new().await;
    for (id, name) in [("a", "A"), ("b", "B")] {
        fixture
            .command(bruh(id, name, fixture.directory.path()))
            .await;
    }
    for (agent, message) in [("a", "one"), ("b", "other"), ("a", "two")] {
        fixture
            .command(json!({"action":"chat","agent_id":agent,"message":message}))
            .await;
    }
    let get = |query: String| {
        let url = format!("{}/api/reactive/agents/a/runs{query}", fixture.base);
        let client = fixture.client.clone();
        async move {
            let response = client.get(url).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[axum::http::header::CACHE_CONTROL],
                "no-store"
            );
            response
                .json::<Vec<bruhs_models::reactive::ReactiveRun>>()
                .await
                .unwrap()
        }
    };
    let page = get(String::new()).await;
    assert_eq!(page.len(), 2);
    assert!(page.iter().all(|run| run.agent_id == "a"));
    assert_eq!(page[0].owner_message.as_deref(), Some("two"));
    let older = get(format!("?before={}", page[0].id)).await;
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].owner_message.as_deref(), Some("one"));
}

#[tokio::test]
async fn foreign_pages_cannot_read_bruh_runs() {
    let fixture = Fixture::new().await;
    fixture
        .command(bruh("a", "A", fixture.directory.path()))
        .await;
    for path in ["agents/a/runs", "stream"] {
        let response = fixture
            .client
            .get(format!("{}/api/reactive/{path}", fixture.base))
            .header("origin", "https://untrusted.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

fn room_command(id: &str) -> Value {
    json!({"action":"create_room","room":{"id":id,"name":"Room","members":["a"]}})
}

#[tokio::test]
async fn room_messages_route_pages_one_rooms_messages() {
    let fixture = Fixture::new().await;
    fixture
        .command(bruh("a", "A", fixture.directory.path()))
        .await;
    fixture.command(room_command("r")).await;
    for n in 0..51 {
        fixture
            .command(json!({"action":"post_room","room_id":"r","text":format!("m{n}")}))
            .await;
    }
    let get = |room: &'static str, query: String| {
        let url = format!("{}/api/reactive/rooms/{room}/messages{query}", fixture.base);
        let client = fixture.client.clone();
        async move {
            let response = client.get(url).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[axum::http::header::CACHE_CONTROL],
                "no-store"
            );
            response
                .json::<Vec<bruhs_models::crew::room::RoomMessage>>()
                .await
                .unwrap()
        }
    };
    let page = get("r", String::new()).await;
    assert_eq!(page.len(), 50);
    assert!(page.iter().all(|message| message.room_id == "r"));
    assert_eq!(page[0].text, "m50");
    let older = get("r", format!("?before={}", page[49].id)).await;
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].text, "m0");
    assert!(get("nobody", String::new()).await.is_empty());
    let native =
        crate::desktop_server_client::ServerClient::new(&fixture.base, "http://127.0.0.1:9")
            .unwrap();
    let first = native.room_messages("r", None).await.unwrap();
    assert_eq!(first, page);
    let older = native.room_messages("r", Some(&page[49].id)).await.unwrap();
    assert_eq!(older.len(), 1);
}

#[tokio::test]
async fn foreign_pages_cannot_read_room_messages() {
    let fixture = Fixture::new().await;
    let response = fixture
        .client
        .get(format!("{}/api/reactive/rooms/r/messages", fixture.base))
        .header("origin", "https://untrusted.example")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_reactive_stream_sends_room_states() {
    let fixture = Fixture::new().await;
    fixture
        .command(bruh("a", "A", fixture.directory.path()))
        .await;
    fixture.command(room_command("r")).await;
    let mut response = fixture
        .client
        .get(format!("{}/api/reactive/stream", fixture.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut text = String::new();
    let frame = loop {
        let chunk = response.chunk().await.unwrap().expect("stream ended");
        text.push_str(&String::from_utf8_lossy(&chunk));
        if let Some(data) = text.lines().find_map(|line| line.strip_prefix("data:")) {
            break data.trim().to_string();
        }
    };
    let signal: bruhs_models::reactive::OutboxSignal = serde_json::from_str(&frame).unwrap();
    assert_eq!(signal.rooms.len(), 1);
    assert_eq!(signal.rooms[0].room.id, "r");
}

#[tokio::test]
async fn oversized_hook_does_not_advance_an_event_receipt() {
    let fixture = Fixture::new().await;
    fixture.configured().await;
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    let response = fixture
        .client
        .post(format!("{}/api/reactive/hooks/scans", fixture.base))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(format!("\"{}\"", "x".repeat(MAX_PAYLOAD_BYTES)))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(fixture.runtime.snapshot().await.unwrap().events.is_empty());
}

#[tokio::test]
async fn weekly_report_preset_and_run_retry_use_the_same_http_contract_for_native() {
    use bruhs_models::weekly_reports::*;
    let fixture = Fixture::new().await;
    let native =
        crate::desktop_server_client::ServerClient::new(&fixture.base, "http://127.0.0.1:9")
            .unwrap();
    let mut preset = WeeklyReportPreset {
        id: uuid::Uuid::new_v4().to_string(),
        project_path: fixture.directory.path().display().to_string(),
        model: "claude-haiku-4-5".into(),
        local_git: false,
        github_pull_requests: false,
        github_deployments: false,
        scheduled: false,
        schedule: None,
    };
    assert!(matches!(
        native
            .weekly_report_command(&WeeklyReportCommand::SavePreset {
                preset: preset.clone()
            })
            .await,
        Err(crate::desktop_server_client::ServerError::Status {
            code: StatusCode::BAD_REQUEST,
            ..
        })
    ));
    assert!(native
        .weekly_reports(None)
        .await
        .unwrap()
        .presets
        .is_empty());
    preset.local_git = true;
    let saved = native
        .weekly_report_command(&WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        })
        .await
        .unwrap();
    assert_eq!(saved.presets, vec![preset.clone()]);
    let request = WeeklyReportCommand::RunNow {
        preset_id: preset.id.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        window_end_utc: None,
    };
    native.weekly_report_command(&request).await.unwrap();
    native.weekly_report_command(&request).await.unwrap();
    let snapshot = fixture.runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(
        snapshot.runs[0].status,
        bruhs_models::reactive::RunStatus::Queued
    );
    assert!(
        native
            .weekly_reports(None)
            .await
            .unwrap()
            .reports
            .is_empty(),
        "queued runs are not saved reports"
    );
    let response = fixture
        .client
        .get(format!("{}/api/weekly-reports", fixture.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    assert!(native
        .weekly_report("not-a-saved-report")
        .await
        .unwrap_err()
        .is_not_found());
    let hostile = fixture
        .client
        .post(format!("{}/api/weekly-reports/commands", fixture.base))
        .header("Origin", "https://unrelated.example")
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(hostile.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn webhook_filters_use_http_request_metadata_without_persisting_it() {
    let fixture = Fixture::new().await;
    fixture.configured().await;
    fixture.command(json!({"action":"apply","change":{"type":"save_subscription","subscription":{
        "id":"scan-bills","source_id":"scans","agent_id":"bills","instructions":"Review this delivery","enabled":true,
        "webhook_filters":[{"path":"query.queue","op":"equals","value":"ready queue"},{"path":"headers.x-source","op":"exists"},{"path":"body.action","op":"in","value":["opened","reopened"]}]
    }}})).await;
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    for (action, expected) in [("closed", vec!["scan-bills"]), ("opened", vec![])] {
        let response = fixture
            .client
            .post(format!(
                "{}/api/reactive/hooks/scans?queue=other&queue=ready+queue",
                fixture.base
            ))
            .bearer_auth(&token)
            .header("X-Source", "private-source-fixture")
            .json(&json!({"action":action}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let event: ReactiveEvent = response.json().await.unwrap();
        assert!(event.filtered_subscriptions.is_empty());
        let snapshot = fixture.runtime.snapshot().await.unwrap();
        let stored = snapshot
            .events
            .iter()
            .find(|stored| stored.id == event.id)
            .unwrap();
        assert_eq!(stored.filtered_subscriptions, expected);
    }
    let snapshot = fixture.runtime.snapshot().await.unwrap();
    assert_eq!(snapshot.runs.len(), 1);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("private-source-fixture"));
}

#[tokio::test]
async fn webhook_delivery_history_is_scoped_to_the_task_and_records_each_http_attempt() {
    use bruhs_models::reactive::{WebhookDelivery, WebhookDeliverySummary, WebhookOutcome};
    let fixture = Fixture::new().await;
    fixture.configured().await;
    fixture.command(json!({"action":"apply","change":{"type":"save_subscription","subscription":{
        "id":"only-closed","source_id":"scans","agent_id":"bills","instructions":"Review closed items","enabled":true,
        "webhook_filters":[{"path":"body.action","op":"equals","value":"closed"}]
    }}})).await;
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    let hook = format!(
        "{}/api/reactive/hooks/scans?lane=ready&access_token=query-secret",
        fixture.base
    );
    let body =
        json!({"action":"opened","nested":[{"password":"body-secret"}],"text":"東京".repeat(3500)});
    for credential in ["invalid", &token, &token] {
        let response = fixture
            .client
            .post(&hook)
            .bearer_auth(credential)
            .header("x-event-id", "first")
            .header("x-api-key", "header-secret")
            .header("x-source", "scanner")
            .header("x-reflection", &token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if credential == "invalid" {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::ACCEPTED
            }
        );
    }
    let list = |task: &str| {
        fixture
            .client
            .get(format!(
                "{}/api/reactive/tasks/{task}/deliveries",
                fixture.base
            ))
            .send()
    };
    let foreign_origin = fixture
        .client
        .get(format!(
            "{}/api/reactive/tasks/scan-bills/deliveries",
            fixture.base
        ))
        .header("Origin", "https://unrelated.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_origin.status(), StatusCode::FORBIDDEN);
    let response = list("scan-bills").await.unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let items: Vec<WebhookDeliverySummary> = response.json().await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].outcome, WebhookOutcome::Duplicate);
    assert_eq!(items[1].outcome, WebhookOutcome::AwaitingReview);
    let filtered: Vec<WebhookDeliverySummary> =
        list("only-closed").await.unwrap().json().await.unwrap();
    assert_eq!(filtered[1].outcome, WebhookOutcome::Filtered);
    let get = |task: &str, id: &str| {
        fixture
            .client
            .get(format!(
                "{}/api/reactive/tasks/{task}/deliveries/{id}",
                fixture.base
            ))
            .send()
    };
    let detail: WebhookDelivery = get("scan-bills", &items[1].id)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        detail.body_truncated
            && detail.headers.contains("x-source: scanner")
            && detail.query.contains("lane: ready")
    );
    assert!(detail
        .prompt
        .as_ref()
        .unwrap()
        .contains("Summarize the scan"));
    let serialized = serde_json::to_string(&detail).unwrap();
    for secret in [&token, "body-secret", "query-secret", "header-secret"] {
        assert!(!serialized.contains(secret));
    }
    let detail: WebhookDelivery = get("only-closed", &items[1].id)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail.summary.outcome, WebhookOutcome::Filtered);
    assert!(detail.prompt.is_none());
    assert!(detail.error.unwrap().contains("body.action"));
    let conflict = fixture
        .client
        .post(&hook)
        .bearer_auth(&token)
        .header("x-event-id", "first")
        .json(&json!({"action":"different"}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let rejected: Vec<WebhookDeliverySummary> =
        list("scan-bills").await.unwrap().json().await.unwrap();
    assert_eq!(rejected[0].outcome, WebhookOutcome::Rejected);
    let failed: WebhookDelivery = get("scan-bills", &rejected[0].id)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(failed.error.unwrap().contains("different content"));
    assert_eq!(fixture.runtime.snapshot().await.unwrap().runs.len(), 1);
    fixture
        .command(
            json!({"action":"apply","change":{"type":"delete_subscription","id":"only-closed"}}),
        )
        .await;
    assert_eq!(
        list("only-closed").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    fixture.command(json!({"action":"apply","change":{"type":"save_subscription","subscription":{
        "id":"only-closed","source_id":"scans","agent_id":"bills","instructions":"A new task","enabled":true
    }}})).await;
    assert_eq!(
        get("only-closed", &items[1].id).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("scan-bills", &items[1].id).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn webhook_history_keeps_recent_attempts_and_original_task_outcomes_across_restart() {
    use bruhs_models::reactive::{WebhookOutcome, HISTORY_LIMIT};
    let fixture = Fixture::new().await;
    fixture.configured().await;
    for (id, controls) in [
        ("batch", json!({"debounce_ms":60000})),
        ("slow", json!({"throttle_ms":60000})),
        ("paused", json!({"enabled":false})),
    ] {
        let mut sub = json!({"id":id,"source_id":"scans","agent_id":"bills","instructions":"Inspect","enabled":true});
        sub.as_object_mut()
            .unwrap()
            .extend(controls.as_object().unwrap().clone());
        fixture
            .command(
                json!({"action":"apply","change":{"type":"save_subscription","subscription":sub}}),
            )
            .await;
    }
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    for key in ["a", "b"] {
        fixture
            .runtime
            .publish_hook("scans", &token, key, json!({"id":key}))
            .await
            .unwrap();
    }
    for (task, expected) in [
        ("batch", WebhookOutcome::Coalesced),
        ("slow", WebhookOutcome::Throttled),
        ("paused", WebhookOutcome::Paused),
    ] {
        let rows = fixture
            .runtime
            .webhook_deliveries(task.into())
            .await
            .unwrap();
        assert_eq!(rows[0].outcome, expected);
    }
    let oldest = fixture
        .runtime
        .webhook_deliveries("batch".into())
        .await
        .unwrap()
        .pop()
        .unwrap();
    for _ in 0..HISTORY_LIMIT {
        fixture
            .runtime
            .publish_hook("scans", &token, "b", json!({"id":"b"}))
            .await
            .unwrap();
    }
    let db = tempfile::tempdir().unwrap();
    // A consistent copy of a live WAL database is also a fresh runtime with no in-memory history.
    let saved = db.path().join("reopened.sqlite3");
    let source =
        rusqlite::Connection::open(fixture.directory.path().join("automation.sqlite3")).unwrap();
    source
        .execute("VACUUM INTO ?1", [saved.to_str().unwrap()])
        .unwrap();
    let reopened = ReactiveRuntime::open(saved).await.unwrap();
    let rows = reopened.webhook_deliveries("batch".into()).await.unwrap();
    assert_eq!(rows.len(), HISTORY_LIMIT);
    assert!(rows
        .iter()
        .all(|row| row.outcome == WebhookOutcome::Duplicate));
    assert!(matches!(
        reopened.webhook_delivery("batch".into(), oldest.id).await,
        Err(ReactiveError::NotFound(_))
    ));
}

#[tokio::test]
async fn webhook_signature_checks_exact_http_body_and_preserves_shared_source_retries() {
    use bruhs_models::reactive::WebhookOutcome;
    const SIGNATURE: &str =
        "sha256=f165f0f326ac75f6c8ba9f3c3eba3e535cf9cda666ee3a3e048e3504cfb68b30";
    const BODY: &str = "{ \"fixture\": true }";
    let fixture = Fixture::new().await;
    fixture.configured().await;
    let snapshot = fixture.runtime.snapshot().await.unwrap();
    let mut task = serde_json::to_value(&snapshot.subscriptions[0]).unwrap();
    task["webhook_security"] = json!({"signature":{"header":"x-check","encoding":"hex","prefix":"SHA256="},"max_delivery_age_minutes":5});
    task["webhook_prompt"] = json!("Review {{body.fixture}} {{headers.x-check}}.");
    let saved = fixture
        .command(json!({"action":"save_scheduled_task", "enabled":true,
        "automation":{"agent":snapshot.agents[0],"source":snapshot.sources[0],"subscription":task},
        "signing_secret":"fixture-signing-key"}))
        .await;
    assert!(!saved.to_string().contains("fixture-signing-key"));
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    let hook = format!("{}/api/reactive/hooks/scans", fixture.base);
    let deliver = |id, signature, raw| {
        fixture
            .client
            .post(&hook)
            .bearer_auth(&token)
            .header("x-event-id", id)
            .header("x-check", signature)
            .header("content-type", "application/json")
            .body(raw)
            .send()
    };

    assert_eq!(
        deliver("first", SIGNATURE, "{\"fixture\":true}")
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(fixture.runtime.snapshot().await.unwrap().events.is_empty());
    assert_eq!(
        deliver("first", SIGNATURE, BODY).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        deliver("first", SIGNATURE, BODY).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    let history = fixture
        .runtime
        .webhook_deliveries("scan-bills".into())
        .await
        .unwrap();
    assert_eq!(
        history.iter().map(|h| h.outcome).collect::<Vec<_>>(),
        [
            WebhookOutcome::Duplicate,
            WebhookOutcome::AwaitingReview,
            WebhookOutcome::RejectedSignature
        ]
    );
    let detail = fixture
        .runtime
        .webhook_delivery("scan-bills".into(), history[1].id.clone())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&detail).unwrap().contains(SIGNATURE));
    assert_eq!(detail.body, BODY);
    let mut sibling = snapshot.subscriptions[0].clone();
    sibling.id = "unsigned".into();
    fixture
        .command(
            json!({"action":"apply","change":{"type":"save_subscription","subscription":sibling}}),
        )
        .await;
    for (sig, body) in [("invalid", "{}"), (SIGNATURE, BODY), (SIGNATURE, BODY)] {
        assert_eq!(
            deliver("shared", sig, body).await.unwrap().status(),
            StatusCode::ACCEPTED
        );
    }
    let fresh = fixture.runtime.snapshot().await.unwrap();
    assert_eq!(fresh.runs.len(), 3);
    assert!(!serde_json::to_string(&fresh).unwrap().contains(SIGNATURE));
    let signed = fixture
        .runtime
        .webhook_deliveries("scan-bills".into())
        .await
        .unwrap();
    assert_eq!(signed[0].outcome, WebhookOutcome::Duplicate);
    assert_eq!(signed[1].outcome, WebhookOutcome::AwaitingReview);
    assert_eq!(signed[2].outcome, WebhookOutcome::RejectedSignature);
}

#[tokio::test]
async fn raw_webhook_checks_original_bytes_and_renders_decoded_text() {
    use bruhs_models::reactive::WebhookOutcome;
    let fixture = Fixture::new().await;
    fixture.configured().await;
    let snapshot = fixture.runtime.snapshot().await.unwrap();
    let mut task = serde_json::to_value(&snapshot.subscriptions[0]).unwrap();
    task["webhook_security"] =
        json!({"signature":{"header":"x-check","encoding":"hex","prefix":""}});
    task["webhook_prompt"] = json!("Read {{body}} {{body.subject}} {{headers.absent}}");
    fixture
        .command(json!({"action":"save_scheduled_task","enabled":true,
        "automation":{"agent":snapshot.agents[0],"source":snapshot.sources[0],"subscription":task},
        "signing_secret":"raw-body-fixture"}))
        .await;
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    let send = |signature| {
        fixture
            .client
            .post(format!("{}/api/reactive/hooks/scans", fixture.base))
            .bearer_auth(&token)
            .header("content-type", "application/octet-stream")
            .header("x-check", signature)
            .header("x-event-id", "binary")
            .body(b"\xef\xbb\xbfnote:\x80\n".to_vec())
            .send()
    };
    // Digest of the decoded/re-encoded display text, then of the original bytes.
    assert_eq!(
        send("6beb9085a73cd267ddcd90eeaabb26c85569058c360eb27158db34366a0a9bc6")
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send("eecd5bcfe5047b85d18fc80678b4d7e981b0d2e2b11c9643fc73de7700342cb7")
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let rows = fixture
        .runtime
        .webhook_deliveries("scan-bills".into())
        .await
        .unwrap();
    assert_eq!(rows[0].outcome, WebhookOutcome::AwaitingReview);
    assert_eq!(rows[0].missing_field_count, 2);
    let detail = fixture
        .runtime
        .webhook_delivery("scan-bills".into(), rows[0].id.clone())
        .await
        .unwrap();
    assert_eq!(detail.body, "note:�\n");
    assert!(detail.signature_verified);
    assert_eq!(detail.missing_fields, ["body.subject", "headers.absent"]);
    assert!(detail.prompt.unwrap().contains(">note:�\n</"));
    let rejected = fixture
        .runtime
        .webhook_delivery("scan-bills".into(), rows[1].id.clone())
        .await
        .unwrap();
    assert!(!rejected.signature_verified);
}

#[tokio::test]
async fn form_webhook_fields_route_requests_and_redact_all_occurrences_in_history() {
    use bruhs_models::reactive::WebhookOutcome;
    let fixture = Fixture::new().await;
    fixture.configured().await;
    fixture.command(json!({"action":"apply","change":{"type":"save_subscription","subscription":{
        "id":"scan-bills","agent_id":"bills","source_id":"scans","instructions":"Review","enabled":true,
        "webhook_filters":[{"path":"body.state","op":"equals","value":"needs review"}],
        "webhook_prompt":"{{body}} / {{body.title}} / {{body.missing}}",
        "webhook_dedupe":{"replay_key":"{{body.title}}"}
    }}})).await;
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    let post = |data| {
        fixture
            .client
            .post(format!("{}/api/reactive/hooks/scans", fixture.base))
            .bearer_auth(&token)
            .header(
                "content-type",
                "Application/X-WWW-Form-Urlencoded; charset=utf-8",
            )
            .body(data)
            .send()
    };
    let raw =
        "state=held&state=needs+review&title=caf%C3%A9&to%6ben=first-value&token=second-value";
    for data in [raw, raw, "state=held&title=next"] {
        assert_eq!(post(data).await.unwrap().status(), StatusCode::ACCEPTED);
    }
    let snap = fixture.runtime.snapshot().await.unwrap();
    assert_eq!(snap.runs.len(), 1);
    assert!(snap.runs[0].prompt.contains(raw));
    assert!(snap.runs[0].prompt.contains(">café</"));
    let rows = fixture
        .runtime
        .webhook_deliveries("scan-bills".into())
        .await
        .unwrap();
    assert_eq!(rows[0].outcome, WebhookOutcome::Filtered);
    assert!(rows[0]
        .filter_reason
        .as_ref()
        .unwrap()
        .contains("body.state"));
    assert_eq!(rows[1].outcome, WebhookOutcome::Duplicate);
    assert_eq!(rows[2].missing_field_count, 1);
    let detail = fixture
        .runtime
        .webhook_delivery("scan-bills".into(), rows[2].id.clone())
        .await
        .unwrap();
    let preview = serde_json::to_string(&detail).unwrap();
    assert!(!preview.contains("first-value") && !preview.contains("second-value"));
    assert!(
        detail.body.contains("state=needs+review")
            && detail.body.matches("%5Bredacted%5D").count() == 2
    );
    assert_eq!(detail.summary.missing_field_count, 1);
}

#[tokio::test]
async fn opaque_webhook_retries_compare_wire_bytes_and_use_the_wire_size_budget() {
    let fixture = Fixture::new().await;
    fixture.configured().await;
    let token = fixture.runtime.webhook_token("scans").await.unwrap();
    let post = |id, data: Vec<u8>| {
        fixture
            .client
            .post(format!("{}/api/reactive/hooks/scans", fixture.base))
            .bearer_auth(&token)
            .header("content-type", "text/plain")
            .header("x-event-id", id)
            .body(data)
            .send()
    };
    for (bytes, status) in [
        (vec![0x80], StatusCode::ACCEPTED),
        (vec![0x80], StatusCode::ACCEPTED),
        (vec![0x81], StatusCode::CONFLICT),
    ] {
        assert_eq!(post("bad-utf8", bytes).await.unwrap().status(), status);
    }
    assert_eq!(
        post("empty", vec![]).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        post("boundary", vec![b'\n'; MAX_PAYLOAD_BYTES])
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        post("too-large", vec![b'\n'; MAX_PAYLOAD_BYTES + 1])
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(fixture.runtime.snapshot().await.unwrap().events.len(), 3);
}
