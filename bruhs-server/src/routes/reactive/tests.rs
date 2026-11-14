use super::*;
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
