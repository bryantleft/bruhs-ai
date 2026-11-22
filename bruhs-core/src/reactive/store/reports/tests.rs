use super::*;

fn fixture() -> (
    tempfile::TempDir,
    Connection,
    super::super::RuntimeLock,
    WeeklyReportPreset,
) {
    let dir = tempfile::tempdir().unwrap();
    let (mut connection, lock) = super::super::open(&dir.path().join("reactive.sqlite3")).unwrap();
    let preset = WeeklyReportPreset {
        id: uuid::Uuid::new_v4().to_string(),
        project_path: dir.path().display().to_string(),
        model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
        local_git: true,
        github_pull_requests: false,
        github_deployments: false,
        scheduled: false,
        schedule: None,
    };
    command(
        &mut connection,
        WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        },
    )
    .unwrap();
    (dir, connection, lock, preset)
}

fn queue(connection: &mut Connection, preset: &WeeklyReportPreset) -> ReactiveRun {
    command(
        connection,
        WeeklyReportCommand::RunNow {
            preset_id: preset.id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            window_end_utc: None,
        },
    )
    .unwrap();
    super::super::claim_next(connection).unwrap().unwrap().run
}
fn evidence() -> Vec<ReportCoverage> {
    vec![ReportCoverage::Unavailable {
        source: "Incidents".into(),
        reason: "No incident feed is connected.".into(),
    }]
}
fn finish(connection: &mut Connection, run: &ReactiveRun) {
    set_coverage(connection, &run.id, evidence()).unwrap();
    super::super::finish_run(
        connection,
        &run.id,
        RunStatus::Completed,
        Some("# Report\nSources were reviewed. Incidents are unavailable.".into()),
        None,
        None,
    )
    .unwrap();
}

#[test]
fn retries_keep_one_window_and_one_saved_report_after_reopen() {
    let (dir, mut connection, lock, preset) = fixture();
    let request = WeeklyReportCommand::RunNow {
        preset_id: preset.id.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        window_end_utc: None,
    };
    command(&mut connection, request.clone()).unwrap();
    command(&mut connection, request).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM reactive_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let run = super::super::claim_next(&mut connection)
        .unwrap()
        .unwrap()
        .run;
    let job = prepare(&mut connection, &run).unwrap().unwrap();
    assert_eq!(job.window_end.millis_since(job.window_start), WEEK_MS);
    finish(&mut connection, &run);
    finish(&mut connection, &run);
    let saved = read(&mut connection, &run.id).unwrap();
    assert_eq!(saved.thread_id, run.thread_id);
    assert_eq!(saved.project_path, preset.project_path);
    drop(connection);
    drop(lock);
    let (mut connection, _lock) = super::super::open(&dir.path().join("reactive.sqlite3")).unwrap();
    assert_eq!(read(&mut connection, &run.id).unwrap(), saved);
    assert_eq!(snapshot(&mut connection, None).unwrap().reports.len(), 1);
}

#[test]
fn cancelled_or_failed_runs_do_not_save_reports() {
    for cancelled in [false, true] {
        let (_dir, mut connection, _lock, preset) = fixture();
        let run = queue(&mut connection, &preset);
        set_coverage(&mut connection, &run.id, evidence()).unwrap();
        if cancelled {
            super::super::command(
                &mut connection,
                ReactiveCommand::CancelRun { id: run.id.clone() },
            )
            .unwrap();
        }
        super::super::finish_run(
            &mut connection,
            &run.id,
            if cancelled {
                RunStatus::Completed
            } else {
                RunStatus::Failed
            },
            Some("Partial output".into()),
            None,
            None,
        )
        .unwrap();
        assert!(snapshot(&mut connection, None).unwrap().reports.is_empty());
    }
}

#[test]
fn report_storage_failure_cannot_acknowledge_a_completed_run() {
    let (_dir, mut connection, _lock, preset) = fixture();
    let run = queue(&mut connection, &preset);
    set_coverage(&mut connection, &run.id, evidence()).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_report BEFORE INSERT ON engineering_reports BEGIN SELECT RAISE(ABORT,'synthetic storage failure'); END;").unwrap();
    assert!(super::super::finish_run(
        &mut connection,
        &run.id,
        RunStatus::Completed,
        Some("Full report".into()),
        None,
        None
    )
    .is_err());
    assert_eq!(
        super::super::run_record(&connection, &run.id)
            .unwrap()
            .status,
        RunStatus::Running
    );
    assert!(snapshot(&mut connection, None).unwrap().reports.is_empty());
}

#[test]
fn edits_preserve_evidence_and_reject_stale_versions() {
    let (_dir, mut connection, _lock, preset) = fixture();
    let run = queue(&mut connection, &preset);
    finish(&mut connection, &run);
    let edit = WeeklyReportCommand::EditReport {
        id: run.id.clone(),
        expected_revision: 1,
        title: "Reviewed report".into(),
        body: "Owner edits".into(),
    };
    command(&mut connection, edit.clone()).unwrap();
    assert!(matches!(
        command(&mut connection, edit),
        Err(ReactiveError::Conflict(_))
    ));
    let report = read(&mut connection, &run.id).unwrap();
    assert_eq!(report.body, "Owner edits");
    assert_eq!(report.coverage, evidence());
    assert_eq!(report.revision, 2);
}

#[test]
fn delayed_schedule_uses_the_due_window_not_execution_time() {
    let (_dir, mut connection, _lock, mut preset) = fixture();
    preset.scheduled = true;
    command(
        &mut connection,
        WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        },
    )
    .unwrap();
    let due = EpochMs::saturating_from_i64(EpochMs::now().as_millis() - 100_000);
    let event = super::super::publish(
        &connection,
        &preset.source_id(),
        "scheduled-1",
        serde_json::json!({"scheduled_at_ms":due.as_millis()}),
    )
    .unwrap();
    let run = super::super::claim_next(&mut connection)
        .unwrap()
        .unwrap()
        .run;
    assert_eq!(run.event_id, Some(event.id));
    let job = prepare(&mut connection, &run).unwrap().unwrap();
    assert_eq!(job.window_end, due);
    set_coverage(&mut connection, &run.id, evidence()).unwrap();
    set_coverage(&mut connection, &run.id, Vec::new()).unwrap();
    assert_eq!(
        prepare(&mut connection, &run).unwrap().unwrap().coverage,
        Some(evidence())
    );
}

#[tokio::test]
#[ignore = "explicit live provider replay on synthetic repository data only"]
async fn live_weekly_report_delivery() {
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());
    assert!(std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Report Test",
            "-c",
            "user.email=reports@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "Add synthetic report delivery fixture"
        ])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());
    let path = dir.path().join("reactive.sqlite3");
    let runtime = crate::reactive::ReactiveRuntime::open(&path).await.unwrap();
    let app = Arc::new(
        crate::AppState::builder()
            .secret_store(crate::secrets::SecretStore::in_memory())
            .connections(crate::connections::Connections::in_memory())
            .build(),
    );
    let preset = WeeklyReportPreset {
        id: uuid::Uuid::new_v4().to_string(),
        project_path: project.display().to_string(),
        model: std::env::var("BRUHS_WEEKLY_REPORT_LIVE_MODEL")
            .unwrap_or_else(|_| "claude-haiku-4-5".into()),
        local_git: true,
        github_pull_requests: false,
        github_deployments: false,
        scheduled: false,
        schedule: None,
    };
    runtime
        .weekly_report_command(WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        })
        .await
        .unwrap();
    let request = WeeklyReportCommand::RunNow {
        preset_id: preset.id.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        window_end_utc: None,
    };
    runtime
        .weekly_report_command(request.clone())
        .await
        .unwrap();
    runtime.weekly_report_command(request).await.unwrap();
    let mut changes = runtime.watch_changes();
    let worker = runtime.start(app.clone()).unwrap();
    let report = tokio::time::timeout(std::time::Duration::from_secs(180), async {
        loop {
            let snapshot = runtime.snapshot().await.unwrap();
            assert_eq!(snapshot.runs.len(), 1);
            let run = &snapshot.runs[0];
            if run.status == RunStatus::Completed {
                break runtime.weekly_report(run.id.clone()).await.unwrap();
            }
            assert!(
                !matches!(
                    run.status,
                    RunStatus::Failed | RunStatus::Interrupted | RunStatus::Cancelled
                ),
                "live report failed: {:?}",
                run.error
            );
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("live report completion receipt");
    assert!(!report.body.trim().is_empty());
    assert!(
        matches!(&report.coverage[0],ReportCoverage::Available {facts,..} if facts.len()==1 && facts[0].title=="Add synthetic report delivery fixture")
    );
    if let Some(output) = std::env::var_os("BRUHS_WEEKLY_REPORT_LIVE_OUTPUT") {
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    runtime
        .weekly_report_command(WeeklyReportCommand::EditReport {
            id: report.id.clone(),
            expected_revision: report.revision,
            title: report.title.clone(),
            body: format!("{}\n\nOwner reviewed this report.", report.body),
        })
        .await
        .unwrap();
    worker.abort();
    let _ = worker.await;
    drop(app);
    drop(runtime);
    let reopened = crate::reactive::ReactiveRuntime::open(&path).await.unwrap();
    let saved = reopened.weekly_report(report.id).await.unwrap();
    assert_eq!(saved.revision, 2);
    assert!(saved.body.ends_with("Owner reviewed this report."));
}

#[test]
fn dedicated_preset_owns_configuration_and_preserves_existing_owner_context() {
    let (_dir, mut connection, _lock, mut preset) = fixture();
    let mut legacy: ReactiveAgent =
        required(&connection, "reactive_agents", &preset.agent_id(), "agent").unwrap();
    legacy.memory = "Owner-approved context retained across model changes.".into();
    legacy.instructions = "Owner report instructions".into();
    // Simulate context saved before generic controls became read-only for presets.
    super::super::apply(
        &connection,
        &ReactiveChange::SaveAgent {
            agent: legacy.clone(),
        },
    )
    .unwrap();
    assert!(matches!(
        super::super::set_linked_schedule_enabled(&mut connection, &preset.subscription_id(), true),
        Err(ReactiveError::Conflict(_))
    ));
    assert!(matches!(
        super::super::delete_linked_schedule(&mut connection, preset.subscription_id()),
        Err(ReactiveError::Conflict(_))
    ));
    for command in [
        ReactiveCommand::SetAutomationEnabled {
            id: preset.subscription_id(),
            enabled: true,
        },
        ReactiveCommand::Apply {
            change: ReactiveChange::SetMemory {
                agent_id: preset.agent_id(),
                memory: "replace".into(),
            },
        },
        ReactiveCommand::Propose {
            change: ReactiveChange::SaveAgent {
                agent: legacy.clone(),
            },
        },
    ] {
        assert!(matches!(
            super::super::command(&mut connection, command),
            Err(ReactiveError::Conflict(_))
        ));
    }
    preset.scheduled = true;
    command(
        &mut connection,
        WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        },
    )
    .unwrap();
    let agent: ReactiveAgent =
        required(&connection, "reactive_agents", &preset.agent_id(), "agent").unwrap();
    assert_eq!(agent.memory, legacy.memory);
    assert_eq!(agent.instructions, legacy.instructions);
    let source: EventSource = required(
        &connection,
        "reactive_sources",
        &preset.source_id(),
        "source",
    )
    .unwrap();
    assert!(source.enabled);
    assert!(
        matches!(source.kind,SourceKind::Schedule {every_ms} if every_ms==u64::try_from(WEEK_MS).unwrap())
    );
}

#[test]
fn selected_utc_window_is_frozen_and_idempotency_cannot_change_it() {
    let (_dir, mut connection, _lock, preset) = fixture();
    let request_id = uuid::Uuid::new_v4().to_string();
    let command_value = WeeklyReportCommand::RunNow {
        preset_id: preset.id.clone(),
        request_id: request_id.clone(),
        window_end_utc: Some("2026-01-08T15:00:00Z".into()),
    };
    command(&mut connection, command_value.clone()).unwrap();
    command(&mut connection, command_value).unwrap();
    let run = super::super::claim_next(&mut connection)
        .unwrap()
        .unwrap()
        .run;
    let job = prepare(&mut connection, &run).unwrap().unwrap();
    assert_eq!(
        job.window_end.as_millis(),
        chrono::DateTime::parse_from_rfc3339("2026-01-08T15:00:00Z")
            .unwrap()
            .timestamp_millis()
    );
    assert_eq!(job.window_end.millis_since(job.window_start), WEEK_MS);
    assert!(matches!(
        command(
            &mut connection,
            WeeklyReportCommand::RunNow {
                preset_id: preset.id.clone(),
                request_id,
                window_end_utc: Some("2026-01-09T15:00:00Z".into())
            }
        ),
        Err(ReactiveError::Conflict(_))
    ));
    for invalid in [
        "not a date",
        "2026-01-08T15:00:00-08:00",
        "2999-01-08T15:00:00Z",
    ] {
        assert!(matches!(
            command(
                &mut connection,
                WeeklyReportCommand::RunNow {
                    preset_id: preset.id.clone(),
                    request_id: uuid::Uuid::new_v4().to_string(),
                    window_end_utc: Some(invalid.into())
                }
            ),
            Err(ReactiveError::Invalid(_))
        ));
    }
}

#[test]
fn a_new_request_can_regenerate_the_same_window_with_changed_settings() {
    let (_dir, mut connection, _lock, mut preset) = fixture();
    let first = WeeklyReportCommand::RunNow {
        preset_id: preset.id.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        window_end_utc: Some("2026-01-08T15:00:00Z".into()),
    };
    command(&mut connection, first).unwrap();
    let old = super::super::claim_next(&mut connection)
        .unwrap()
        .unwrap()
        .run;
    finish(&mut connection, &old);
    let saved = read(&mut connection, &old.id).unwrap();
    preset.model = if saved.model == "claude-haiku-4-5" {
        "claude-sonnet-4-6"
    } else {
        "claude-haiku-4-5"
    }
    .into();
    command(
        &mut connection,
        WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        },
    )
    .unwrap();
    let second = WeeklyReportCommand::RunNow {
        preset_id: preset.id.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        window_end_utc: Some("2026-01-08T15:00:00Z".into()),
    };
    command(&mut connection, second.clone()).unwrap();
    command(&mut connection, second).unwrap();
    let new = super::super::claim_next(&mut connection)
        .unwrap()
        .unwrap()
        .run;
    assert_ne!(old.id, new.id);
    assert_eq!(
        prepare(&mut connection, &new)
            .unwrap()
            .unwrap()
            .preset
            .model,
        preset.model
    );
    finish(&mut connection, &new);
    assert_eq!(read(&mut connection, &old.id).unwrap(), saved);
    assert_eq!(snapshot(&mut connection, None).unwrap().reports.len(), 2);
}

#[test]
fn prototype_schema_upgrade_keeps_saved_payloads_and_page_identities() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE weekly_report_jobs(id TEXT PRIMARY KEY,preset_id TEXT NOT NULL,window_end INTEGER NOT NULL,payload TEXT NOT NULL,UNIQUE(preset_id,window_end));CREATE TABLE engineering_reports(id TEXT PRIMARY KEY,preset_id TEXT NOT NULL,window_end INTEGER NOT NULL,payload TEXT NOT NULL,UNIQUE(preset_id,window_end));INSERT INTO engineering_reports(rowid,id,preset_id,window_end,payload) VALUES(42,'saved','preset',1000,'original owner-edited payload');").unwrap();
    initialize(&mut connection).unwrap();
    initialize(&mut connection).unwrap();
    let saved: (i64, String) = connection
        .query_row(
            "SELECT rowid,payload FROM engineering_reports WHERE id='saved'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, (42, "original owner-edited payload".into()));
    connection.execute("INSERT INTO engineering_reports(id,preset_id,window_end,payload) VALUES('new','preset',1000,'new model output')",[]).unwrap();
}

#[test]
fn retrying_a_frozen_model_cannot_replace_the_current_models_chat_session() {
    let (_dir, mut connection, _lock, mut preset) = fixture();
    let run = queue(&mut connection, &preset);
    let old_model = preset.model.clone();
    super::super::finish_run(
        &mut connection,
        &run.id,
        RunStatus::Failed,
        None,
        Some("Retry fixture".into()),
        None,
    )
    .unwrap();
    preset.model = if old_model == "claude-haiku-4-5" {
        "claude-sonnet-4-6"
    } else {
        "claude-haiku-4-5"
    }
    .into();
    assert_ne!(preset.model, old_model);
    command(
        &mut connection,
        WeeklyReportCommand::SavePreset {
            preset: preset.clone(),
        },
    )
    .unwrap();
    super::super::command(
        &mut connection,
        ReactiveCommand::RetryRun { id: run.id.clone() },
    )
    .unwrap();
    let super::super::Claim { run: retry, .. } =
        super::super::claim_next(&mut connection).unwrap().unwrap();
    assert_eq!(
        prepare(&mut connection, &retry)
            .unwrap()
            .unwrap()
            .preset
            .model,
        old_model
    );
    super::super::record_session(
        &mut connection,
        &retry.id,
        "frozen-model-session",
        &old_model,
    )
    .unwrap();
    let current_session: Option<String> = connection
        .query_row(
            "SELECT session_id FROM reactive_agents WHERE id=?1",
            [preset.agent_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(current_session.is_none());
    assert_eq!(
        super::super::run_record(&connection, &retry.id)
            .unwrap()
            .session_id
            .as_deref(),
        Some("frozen-model-session")
    );
}

#[test]
fn saved_report_pagination_keeps_older_reports_reachable_without_sending_bodies() {
    let (_dir, mut connection, _lock, preset) = fixture();
    let run = queue(&mut connection, &preset);
    finish(&mut connection, &run);
    let template = read(&mut connection, &run.id).unwrap();
    for index in 0..HISTORY_LIMIT {
        let mut report = template.clone();
        report.id = format!("report-{index}");
        report.run_id = report.id.clone();
        connection.execute("INSERT INTO engineering_reports(id,preset_id,window_end,payload) VALUES(?1,?2,?3,?4)",params![report.id,report.preset_id,report.window_end.as_millis(),encode(&report).unwrap()]).unwrap();
    }
    let first = snapshot(&mut connection, None).unwrap();
    assert_eq!(first.reports.len(), HISTORY_LIMIT);
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("Sources were reviewed"));
    let cursor = first.next_before.unwrap();
    command(
        &mut connection,
        WeeklyReportCommand::EditReport {
            id: run.id.clone(),
            expected_revision: 1,
            title: "Owner edited oldest report".into(),
            body: "Old report remains reachable".into(),
        },
    )
    .unwrap();
    let second = snapshot(&mut connection, Some(&cursor)).unwrap();
    assert_eq!(second.reports.len(), 1);
    assert!(second.next_before.is_none());
    assert_eq!(second.reports[0].id, run.id);
    assert_eq!(second.reports[0].title, "Owner edited oldest report");
    assert!(first
        .reports
        .iter()
        .all(|report| report.id != second.reports[0].id));
}
