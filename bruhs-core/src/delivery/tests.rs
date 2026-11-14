use super::*;
use std::process::Command as Process;

pub(super) fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["init", "-q", "--initial-branch=main"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["config", "user.name", "Fixture"],
    ] {
        assert!(Process::new("git")
            .args(arguments)
            .current_dir(directory.path())
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(directory.path().join("sample.txt"), "before\n").unwrap();
    assert!(Process::new("git")
        .args(["add", "sample.txt"])
        .current_dir(directory.path())
        .status()
        .unwrap()
        .success());
    assert!(Process::new("git")
        .args(["commit", "-qm", "fixture"])
        .current_dir(directory.path())
        .status()
        .unwrap()
        .success());
    directory
}

fn request(repository: &Path, action: DeliveryAction) -> PrepareRequest {
    PrepareRequest {
        forge: Forge::GitHub,
        path: repository.to_string_lossy().into_owned(),
        action,
        timeout_seconds: 5,
    }
}

#[tokio::test]
async fn an_unrepresentable_timeout_is_rejected_before_starting_any_process() {
    let directory = tempfile::tempdir().unwrap();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    let error = service
        .prepare(PrepareRequest {
            forge: Forge::GitHub,
            path: String::new(),
            action: DeliveryAction::RebaseStack { number: 1 },
            timeout_seconds: u64::MAX,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timer range"));
    assert!(service.snapshot().await.actions.is_empty());
}

#[tokio::test]
async fn preparation_freezes_multiline_input_without_executing_it() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    let body = "first line\n$(touch NEVER) `touch NEVER`\nlast line";
    let prepared = service
        .prepare(request(
            repository.path(),
            DeliveryAction::Comment {
                number: 42,
                body: body.into(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(prepared.commands[0].program, "gh");
    assert_eq!(
        prepared.commands[0].args,
        ["pr", "comment", "42", "--body-file", "-"]
    );
    assert_eq!(prepared.commands[0].stdin.as_deref(), Some(body));
    assert_eq!(
        service.snapshot().await.actions[0].status,
        ActionStatus::Prepared
    );
    assert!(!repository.path().join("NEVER").exists());
    service.cancel(&prepared.id).await.unwrap();
    assert!(service.execute(&prepared.id).await.is_err());
}

#[tokio::test]
async fn a_changed_checkout_prevents_the_reviewed_operation() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    let prepared = service
        .prepare(request(
            repository.path(),
            DeliveryAction::Comment {
                number: 42,
                body: "reviewed".into(),
            },
        ))
        .await
        .unwrap();
    assert!(Process::new("git")
        .args(["switch", "-qc", "another"])
        .current_dir(repository.path())
        .status()
        .unwrap()
        .success());
    let (_sender, cancelled) = watch::channel(false);
    service
        .mutate(|state| {
            state.actions[0].status = ActionStatus::Running;
            Ok(())
        })
        .await
        .unwrap();
    let error = service.run_action(&prepared, cancelled).await.unwrap_err();
    assert!(error.to_string().contains("changed after review"));
}

#[tokio::test]
async fn scripts_are_frozen_and_do_not_run_during_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    std::fs::create_dir(repository.path().join(".bruhs")).unwrap();
    std::fs::write(
        repository.path().join(".bruhs/config.json"),
        r#"{"setup":[{"program":"touch","args":["MUST_NOT_EXIST"]}]}"#,
    )
    .unwrap();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    assert_eq!(
        service
            .scripts(repository.path())
            .await
            .unwrap()
            .setup
            .len(),
        1
    );
    let prepared = service
        .prepare(request(
            repository.path(),
            DeliveryAction::Lifecycle {
                phase: LifecyclePhase::Setup,
            },
        ))
        .await
        .unwrap();
    assert_eq!(prepared.commands[0].args, ["MUST_NOT_EXIST"]);
    assert!(!repository.path().join("MUST_NOT_EXIST").exists());
}

#[tokio::test]
async fn selected_files_cannot_escape_the_repository_or_include_directories() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    for file in ["../secret", "/etc/passwd", ".git/config", ".git"] {
        let action = DeliveryAction::CommitPushPull {
            files: vec![file.into()],
            message: "message".into(),
            title: "title".into(),
            body: String::new(),
            base: "other".into(),
            head: "main".into(),
            draft: true,
        };
        assert!(
            service
                .prepare(request(repository.path(), action))
                .await
                .is_err(),
            "{file}"
        );
    }
}

#[tokio::test]
async fn an_explicit_tracked_file_deletion_can_be_prepared_for_commit() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    std::fs::remove_file(repository.path().join("sample.txt")).unwrap();
    let prepared = service
        .prepare(request(
            repository.path(),
            DeliveryAction::CommitPushPull {
                files: vec!["sample.txt".into()],
                message: "Remove sample".into(),
                title: "Remove sample".into(),
                body: String::new(),
                base: "other".into(),
                head: "main".into(),
                draft: true,
            },
        ))
        .await
        .unwrap();
    assert_eq!(prepared.commands[0].args, ["add", "--", "sample.txt"]);
    assert!(prepared.worktree_fingerprint.is_some());
}

#[tokio::test]
async fn started_actions_are_never_replayed_after_reopening_the_store() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    let prepared = service
        .prepare(request(
            repository.path(),
            DeliveryAction::Comment {
                number: 42,
                body: "reviewed".into(),
            },
        ))
        .await
        .unwrap();
    service
        .mutate(|state| {
            state.actions[0].status = ActionStatus::Running;
            Ok(())
        })
        .await
        .unwrap();
    drop(service);
    let restored = DeliveryService::open(directory.path()).await.unwrap();
    assert_eq!(
        restored.snapshot().await.actions[0].status,
        ActionStatus::Interrupted
    );
    assert!(restored.execute(&prepared.id).await.is_err());
}

#[test]
fn own_comments_do_not_wake_watchers_and_new_failures_do() {
    let mut before = Pull {
        number: 42,
        title: "fixture".into(),
        body: String::new(),
        url: "https://github.com/test/repo/pull/42".into(),
        state: "OPEN".into(),
        draft: false,
        base: "main".into(),
        head: "feature".into(),
        head_oid: "a".repeat(40),
        head_in_repository: true,
        base_oid: "b".repeat(40),
        merge_commit_oid: None,
        updated_at: "before".into(),
        mergeable: "MERGEABLE".into(),
        review_decision: None,
        checks: Vec::new(),
        comments: Vec::new(),
        reviews: Vec::new(),
        files: Vec::new(),
    };
    let mut after = before.clone();
    after.comments.push(PullComment {
        id: "own".into(),
        author: "owner".into(),
        body: "note".into(),
        url: None,
        state: None,
        path: None,
        line: None,
    });
    assert!(watching::reasons(&before, &after, "owner").is_empty());
    before = after.clone();
    after.checks.push(Check {
        name: "test".into(),
        state: "COMPLETED".into(),
        conclusion: Some("FAILURE".into()),
        url: None,
    });
    assert_eq!(
        watching::reasons(&before, &after, "owner"),
        ["Check needs attention: test"]
    );
    assert!(watching::reasons(&after, &after, "owner").is_empty());
}

#[tokio::test]
async fn executing_a_reviewed_script_runs_once_and_saves_its_actual_result() {
    let directory = tempfile::tempdir().unwrap();
    let repository = repository();
    std::fs::create_dir(repository.path().join(".bruhs")).unwrap();
    std::fs::write(
        repository.path().join(".bruhs/config.json"),
        r#"{"setup":[{"program":"sh","args":["-c","printf ran >> receipts.txt; printf done"]}]}"#,
    )
    .unwrap();
    let service = DeliveryService::open(directory.path()).await.unwrap();
    let prepared = service
        .prepare(request(
            repository.path(),
            DeliveryAction::Lifecycle {
                phase: LifecyclePhase::Setup,
            },
        ))
        .await
        .unwrap();
    assert!(!repository.path().join("receipts.txt").exists());
    std::fs::write(
        repository.path().join(".bruhs/config.json"),
        r#"{"setup":[{"program":"sh","args":["-c","touch UNREVIEWED"]}]}"#,
    )
    .unwrap();
    service.execute(&prepared.id).await.unwrap();
    assert!(service.execute(&prepared.id).await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = service.snapshot().await.actions[0].clone();
            if receipt.status != ActionStatus::Running {
                assert_eq!(
                    receipt.status,
                    ActionStatus::Completed,
                    "{:?}",
                    receipt.error
                );
                assert_eq!(receipt.completed_commands, 1);
                assert!(receipt.output.contains("done"));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(repository.path().join("receipts.txt")).unwrap(),
        "ran"
    );
    assert!(!repository.path().join("UNREVIEWED").exists());
}
