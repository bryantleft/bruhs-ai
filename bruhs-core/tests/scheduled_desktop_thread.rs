//! Task-host discovery and binding against an isolated desktop profile.
#![expect(clippy::unwrap_used, reason = "integration test helpers")]
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_models::{ids::ThreadId, reactive::*};
use serde_json::json;
use std::path::Path;

#[test]
fn scheduled_tasks_bind_only_the_selected_desktop_conversation() {
    const FIXTURE: &str = "BRUHS_TEST_SCHEDULED_DESKTOP_PROFILE";
    if let Some(directory) = std::env::var_os(FIXTURE) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(check(Path::new(&directory)));
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "scheduled_tasks_bind_only_the_selected_desktop_conversation",
            "--nocapture",
        ])
        .env(FIXTURE, directory.path())
        .env(
            "BRUHS_DESKTOP_STATE_FILE",
            directory.path().join("desktop.json"),
        )
        .env("BRUHS_WORKBENCH_DIR", directory.path().join("workbench"))
        .status()
        .unwrap();
    assert!(status.success());
}

async fn check(directory: &Path) {
    let id = ThreadId::new_v4();
    let foreign = ThreadId::new_v4();
    let archived = ThreadId::new_v4();
    let side_chat = ThreadId::new_v4();
    let name = "Déploiement";
    let model = "gpt-5.4";
    let project = directory.to_str().unwrap();
    let chat = |id: &ThreadId, folder: &str| {
        json!({
            "id":id,"title":name,"project_path":folder,"model_id":model,"updated_at":"now",
            "messages":[{"role":"user","content":"Private message"}],"session_id":"private-session","session_provider":"codex"
        })
    };
    let profile = directory.join("desktop.json");
    let mut desktop = json!({"threads":[chat(&id,project),chat(&foreign,"/other")],"archived_threads":[chat(&archived,project)]});
    desktop["threads"]
        .as_array_mut()
        .unwrap()
        .push(chat(&side_chat, project));
    desktop["thread_origins"] = json!({side_chat.as_str():{"side_chat":id}});
    std::fs::write(&profile, desktop.to_string()).unwrap();
    let db = directory.join("reactive.sqlite3");
    let host = ReactiveRuntime::open(&db).await.unwrap();
    let choices = host
        .project_conversations(project.into(), "DÉPLOI".into())
        .await
        .unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].thread_id, id);
    assert_eq!(choices[0].desktop_model.as_deref(), Some(model));
    let metadata = serde_json::to_string(&choices).unwrap();
    assert!(!metadata.contains("Private message") && !metadata.contains("private-session"));
    assert!(
        host.task_threads(project.into(), "".into())
            .await
            .unwrap()
            .is_empty()
    );
    desktop["threads"][0]["model_id"] = json!("");
    desktop["current_model"] = json!("legacy-model");
    std::fs::write(&profile, desktop.to_string()).unwrap();
    assert_eq!(
        host.project_conversations(project.into(), "".into())
            .await
            .unwrap()[0]
            .desktop_model
            .as_deref(),
        Some("legacy-model")
    );
    desktop["threads"][0]["model_id"] = json!(model);
    std::fs::write(&profile, desktop.to_string()).unwrap();
    let setup:AutomationSetup=serde_json::from_value(json!({
        "agent":{"id":id,"name":"Review","instructions":"Summarize","model":model,"project_path":project,"enabled":true},
        "source":{"id":"clock","name":"Monday","kind":{"type":"schedule","every_ms":86400000}},
        "subscription":{"id":"digest","source_id":"clock","agent_id":id,"instructions":"Check the build","task_conversation":"continue","auto_run":true}
    })).unwrap();
    let save = |id: ThreadId| ReactiveCommand::SaveScheduledTask {
        automation: Box::new(setup.clone()),
        enabled: false,
        signing_secret: None,
        destination: Some(TaskDestinationEdit::Desktop { thread_id: id }),
    };
    assert!(host.command(save(foreign)).await.is_err());
    assert!(host.command(save(archived)).await.is_err());
    assert!(host.command(save(side_chat)).await.is_err());
    assert!(host.snapshot().await.unwrap().subscriptions.is_empty());
    desktop["threads"][0]["model_id"] = json!("different-model");
    std::fs::write(&profile, desktop.to_string()).unwrap();
    assert!(host.command(save(id.clone())).await.is_err());
    desktop["threads"][0]["model_id"] = json!(model);
    std::fs::write(&profile, desktop.to_string()).unwrap();
    host.command(save(id.clone())).await.unwrap();
    host.command(save(id.clone())).await.unwrap();
    drop(host);
    let restored = ReactiveRuntime::open(&db).await.unwrap();
    let saved = restored.snapshot().await.unwrap();
    assert_eq!(saved.task_threads["digest"], id);
    assert_eq!(saved.desktop_task_activity[id.as_str()], 0);
    assert_eq!(
        restored
            .project_conversations(project.into(), "".into())
            .await
            .unwrap()
            .len(),
        1
    );
    // Binding checks the settings seen by the picker, but subsequent runs
    // follow the chat. Disable both providers in this isolated test profile.
    let switches = directory.join("workbench/disabled-providers");
    std::fs::create_dir_all(&switches).unwrap();
    for provider in ["codex", "grok"] {
        std::fs::write(switches.join(provider), []).unwrap();
    }
    desktop["threads"][0]["model_id"] = json!("grok/default");
    std::fs::write(profile, desktop.to_string()).unwrap();
    restored
        .command(ReactiveCommand::SetAutomationEnabled {
            id: "digest".into(),
            enabled: true,
        })
        .await
        .unwrap();
    restored
        .command(ReactiveCommand::RunScheduledTask {
            id: "digest".into(),
            run_id: "changed-model".into(),
        })
        .await
        .unwrap();
    let app = bruhs_core::AppState::builder()
        .secret_store(bruhs_core::secrets::SecretStore::in_memory())
        .connections(bruhs_core::connections::Connections::in_memory())
        .build();
    let worker = restored.start(std::sync::Arc::new(app)).unwrap();
    let turn = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(turn) = restored
                .desktop_task_turns(id.clone(), 0)
                .await
                .unwrap()
                .into_iter()
                .find(|t| t.run.id == "changed-model")
            {
                break turn;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    worker.abort();
    assert_eq!(turn.model, "grok/default");
    assert_eq!(turn.run.status, RunStatus::Failed);
    assert!(turn.run.error.unwrap().contains("Grok is disabled"));
    // A later stale link rolls the entire checkout removal back.
    let linked: std::collections::BTreeSet<_> = ["digest".to_owned()].into();
    let missing = ThreadId::try_from("zz-missing").unwrap();
    assert!(
        restored
            .command(ReactiveCommand::DeleteCheckoutTaskThreads {
                threads: [
                    (id.clone(), linked.clone()),
                    (missing, ["moved".to_owned()].into())
                ]
                .into(),
            })
            .await
            .is_err()
    );
    assert_eq!(
        restored.snapshot().await.unwrap().task_threads["digest"],
        id
    );
    restored
        .command(ReactiveCommand::DeleteCheckoutTaskThreads {
            threads: [(id.clone(), linked)].into(),
        })
        .await
        .unwrap();
    let after = restored.snapshot().await.unwrap();
    assert!(!after.subscriptions.iter().any(|sub| sub.id == "digest"));
    assert!(!after.desktop_task_activity.contains_key(id.as_str()));
}
