//! Live eval of how often reactive agents interrupt the owner.
//!
//! Each scenario sends synthetic events through the real worker and a
//! signed-in local provider, then checks the run's outcome (silent, notify or
//! ask), memory and follow-ups. It measures the article's "start quiet, earn
//! attention" rule: routine results stay silent, failures notify, ambiguity
//! asks, and nothing already told is repeated.
//!
//! `cargo run -p bruhs-core --example reactive_outcome_eval -- --run-live [--models a,b] [--out results.json]`
//! It makes real model requests and is excluded from ordinary tests. Fixtures
//! contain no personal data and request no writes.

use bruhs_core::{reactive::ReactiveRuntime, AppState};
use bruhs_models::{reactive::*, EpochMs};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

const RUN_DEADLINE: Duration = Duration::from_secs(300);
type Error = Box<dyn std::error::Error + Send + Sync>;

struct Worker(tokio::task::JoinHandle<()>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expect {
    Outcome(RunOutcome),
    Remembers(&'static str),
    FollowUp,
}

struct Scenario {
    name: &'static str,
    instructions: &'static str,
    memory: &'static str,
    /// Events sent one after another; only the last run is scored.
    events: &'static [&'static str],
    expect: Expect,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "routine success stays silent",
        instructions: "Watch CI results for the main branch. Tell the owner only when a build fails.",
        memory: "",
        events: &[r#"{"pipeline":"main","status":"passed","duration_s":312}"#],
        expect: Expect::Outcome(RunOutcome::Silent),
    },
    Scenario {
        name: "failure notifies",
        instructions: "Watch CI results for the main branch. Tell the owner only when a build fails.",
        memory: "",
        events: &[r#"{"pipeline":"main","status":"failed","failed_step":"integration tests","commit":"a1b2c3d"}"#],
        expect: Expect::Outcome(RunOutcome::Notify),
    },
    Scenario {
        name: "the same failure is not repeated",
        instructions: "Watch CI results for the main branch. Tell the owner only when a build fails. Do not repeat a failure you already reported.",
        memory: "",
        events: &[
            r#"{"pipeline":"main","status":"failed","failed_step":"integration tests","commit":"a1b2c3d"}"#,
            r#"{"pipeline":"main","status":"failed","failed_step":"integration tests","commit":"a1b2c3d","retry":1}"#,
        ],
        expect: Expect::Outcome(RunOutcome::Silent),
    },
    Scenario {
        name: "changed bank details ask first",
        instructions: "Prepare payment notes for incoming invoices. If the payee's bank details differ from the ones on record, ask the owner before preparing anything.",
        memory: "On record: ACME Ltd bank account ends in 1234.",
        events: &[r#"{"vendor":"ACME Ltd","amount":"420.00 EUR","due":"2026-11-30","iban":"DE00 0000 0000 0000 9999"}"#],
        expect: Expect::Outcome(RunOutcome::Ask),
    },
    Scenario {
        name: "a stated preference is remembered",
        instructions: "Help schedule the owner's meetings. Keep track of their scheduling preferences.",
        memory: "",
        events: &[r#"{"message_from_owner":"From now on, keep my meetings to 25 minutes, never back to back."}"#],
        expect: Expect::Remembers("25"),
    },
    Scenario {
        name: "a promised reply gets a follow-up",
        instructions: "Track vendor emails. When a vendor promises to reply by a date, schedule a follow-up to check whether they did.",
        memory: "",
        events: &[r#"{"from":"vendor@example.com","subject":"Re: quote","body":"Thanks, we will send the revised quote within two days."}"#],
        expect: Expect::FollowUp,
    },
];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().collect();
    if !args.iter().any(|arg| arg == "--run-live") {
        return Err("Pass --run-live to make real model requests with synthetic fixtures.".into());
    }
    let output = args
        .windows(2)
        .find(|pair| pair[0] == "--out")
        .map(|pair| pair[1].clone());
    let models = args
        .windows(2)
        .find(|pair| pair[0] == "--models")
        .map_or_else(
            || vec!["claude-haiku-4-5".to_owned()],
            |pair| pair[1].split(',').map(str::to_owned).collect(),
        );
    let mut results = Vec::new();
    let mut passed = 0;
    for model in &models {
        for scenario in SCENARIOS {
            eprintln!("{model}: {}", scenario.name);
            let result = match run_scenario(model, scenario).await {
                Ok(result) => result,
                Err(error) => {
                    json!({"model":model,"scenario":scenario.name,"passed":false,"error":error.to_string()})
                }
            };
            if result["passed"] == true {
                passed += 1;
            }
            eprintln!(
                "  -> {}",
                if result["passed"] == true {
                    "pass"
                } else {
                    "FAIL"
                }
            );
            results.push(result);
        }
    }
    let total = models.len() * SCENARIOS.len();
    let report = json!({
        "tested_at": EpochMs::now(),
        "passed": passed,
        "total": total,
        "results": results,
        "personal_data_used": false,
        "writes_requested": false,
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    if let Some(output) = output {
        if let Some(parent) = Path::new(&output).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(output, &encoded).await?;
    }
    println!("{encoded}");
    if passed == total {
        Ok(())
    } else {
        Err(format!("{passed} of {total} scenarios passed").into())
    }
}

async fn run_scenario(model: &str, scenario: &Scenario) -> Result<Value, Error> {
    let directory = tempfile::tempdir()?;
    let runtime = ReactiveRuntime::open(directory.path().join("reactive.sqlite3")).await?;
    runtime.set_notifier(Arc::new(|_: &str, _: &str| Ok(())));
    let automation = AutomationSetup {
        agent: ReactiveAgent {
            id: "eval-agent".into(),
            name: "Eval agent".into(),
            instructions: scenario.instructions.into(),
            memory: scenario.memory.into(),
            model: model.into(),
            project_path: directory.path().to_string_lossy().into_owned(),
            enabled: true,
        },
        source: EventSource {
            id: "eval-source".into(),
            name: "Synthetic events".into(),
            enabled: true,
            kind: SourceKind::Custom,
        },
        subscription: Subscription {
            id: "eval-subscription".into(),
            source_id: "eval-source".into(),
            agent_id: "eval-agent".into(),
            instructions: "Handle this event.".into(),
            enabled: true,
            auto_run: true,
            debounce_ms: 0,
            throttle_ms: 0,
            on_busy: BusyPolicy::Queue,
        },
    };
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation {
                automation: Box::new(automation),
            },
        })
        .await?;
    let app = Arc::new(
        AppState::builder()
            .secret_store(bruhs_core::secrets::SecretStore::in_memory())
            .connections(bruhs_core::connections::Connections::in_memory())
            .build(),
    );
    let _worker = Worker(runtime.start(app)?);
    let mut last = None;
    for (index, event) in scenario.events.iter().enumerate() {
        let payload: Value = serde_json::from_str(event)?;
        runtime
            .publish("eval-source", &format!("event-{index}"), payload)
            .await?;
        last = Some(wait_completed(&runtime, index + 1).await?);
    }
    let run = last.ok_or("No events in scenario")?;
    let snapshot = runtime.snapshot().await?;
    let outcome = run
        .report
        .as_ref()
        .map_or(RunOutcome::Silent, |report| report.outcome);
    let passed = match scenario.expect {
        Expect::Outcome(expected) => outcome == expected,
        Expect::Remembers(needle) => snapshot
            .memories
            .iter()
            .any(|entry| entry.text.contains(needle)),
        Expect::FollowUp => snapshot
            .follow_ups
            .iter()
            .any(|item| item.status == FollowUpStatus::Pending),
    };
    Ok(json!({
        "model": model,
        "scenario": scenario.name,
        "expected": format!("{:?}", scenario.expect),
        "outcome": outcome,
        "passed": passed,
        "report_warning": run.report_warning,
        "memories": snapshot.memories.len(),
        "follow_ups": snapshot.follow_ups.len(),
        "context_tokens": run.context_tokens,
    }))
}

async fn wait_completed(runtime: &ReactiveRuntime, finished: usize) -> Result<ReactiveRun, Error> {
    tokio::time::timeout(RUN_DEADLINE, async {
        loop {
            let snapshot = runtime.snapshot().await?;
            let done: Vec<_> = snapshot
                .runs
                .iter()
                .filter(|run| run.status == RunStatus::Completed)
                .collect();
            if let Some(failed) = snapshot.runs.iter().find(|run| {
                matches!(
                    run.status,
                    RunStatus::Failed | RunStatus::Interrupted | RunStatus::Cancelled
                )
            }) {
                return Err(format!(
                    "Run ended {:?}: {}",
                    failed.status,
                    failed.error.as_deref().unwrap_or("no detail")
                )
                .into());
            }
            if done.len() >= finished {
                return Ok(done
                    .iter()
                    .max_by_key(|run| run.updated_at)
                    .map(|run| (*run).clone())
                    .ok_or("missing run")?);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .map_err(|_| -> Error { "The provider exceeded the five-minute fixture deadline".into() })?
}
