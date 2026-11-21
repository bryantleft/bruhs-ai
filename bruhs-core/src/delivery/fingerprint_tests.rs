use super::*;

const CANARY: &str = "UNREVIEWED_ACTION_RAN";

async fn fixture() -> (tempfile::TempDir, tempfile::TempDir, DeliveryService) {
    let repository = tokio::task::spawn_blocking(tests::repository)
        .await
        .unwrap();
    let state = tempfile::tempdir().unwrap();
    let service = DeliveryService::open(state.path()).await.unwrap();
    (repository, state, service)
}

async fn prepare(service: &DeliveryService, path: &Path, files: &[&str]) -> PreparedAction {
    tokio::fs::create_dir_all(path.join(".bruhs"))
        .await
        .unwrap();
    tokio::fs::write(
        path.join(".bruhs/config.json"),
        r#"{"check":[{"program":"git","args":["diff","--check"]}]}"#,
    )
    .await
    .unwrap();
    let check = service
        .prepare(PrepareRequest {
            forge: Forge::Auto,
            path: path.to_string_lossy().into_owned(),
            timeout_seconds: 5,
            action: DeliveryAction::Verify {
                files: files.iter().map(|file| (*file).to_owned()).collect(),
            },
        })
        .await
        .unwrap();
    service.execute(&check.id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = service
                .snapshot()
                .await
                .actions
                .into_iter()
                .find(|row| row.prepared.id == check.id)
                .unwrap();
            if receipt.status != ActionStatus::Running {
                assert_eq!(
                    receipt.status,
                    ActionStatus::Completed,
                    "{:?}",
                    receipt.error
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut prepared = service
        .prepare(PrepareRequest {
            forge: Forge::GitHub,
            path: path.to_string_lossy().into_owned(),
            action: DeliveryAction::CommitPushPull {
                files: files.iter().map(|file| (*file).to_owned()).collect(),
                verification_id: Some(check.id),
                message: "Reviewed change".into(),
                title: "Reviewed change".into(),
                body: "Reviewed contents".into(),
                base: "release".into(),
                head: "main".into(),
                draft: true,
            },
            // Reuse the existing local delivery-test observation budget.
            timeout_seconds: 5,
        })
        .await
        .unwrap();
    // Exercise the actual pre-execution guard, with a harmless local canary
    // replacing every action command. Even a failing regression cannot push.
    prepared.commands = vec![Command {
        program: "git".into(),
        args: vec![
            "config".into(),
            "--file".into(),
            CANARY.into(),
            "bruhs.canary".into(),
            "ran".into(),
        ],
        stdin: None,
    }];
    prepared
}

async fn rejects(
    service: &DeliveryService,
    path: &Path,
    prepared: &PreparedAction,
) -> DeliveryError {
    start_reviewed_canary(service, prepared).await;
    let (_sender, receiver) = watch::channel(false);
    let result = service.run_action(prepared, receiver).await;
    assert!(
        !tokio::fs::try_exists(path.join(CANARY)).await.unwrap(),
        "Source guard allowed an unreviewed command"
    );
    result.unwrap_err()
}

async fn start_reviewed_canary(service: &DeliveryService, prepared: &PreparedAction) {
    service
        .mutate(|state| {
            let receipt = state
                .actions
                .iter_mut()
                .find(|receipt| receipt.prepared.id == prepared.id)
                .unwrap();
            receipt.prepared = prepared.clone();
            receipt.status = ActionStatus::Running;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn changing_two_untracked_files_cannot_reuse_a_concatenated_fingerprint() {
    let (repository, _state, service) = fixture().await;
    tokio::fs::write(repository.path().join("a"), b"b")
        .await
        .unwrap();
    tokio::fs::write(repository.path().join("b"), b"c")
        .await
        .unwrap();
    let prepared = prepare(&service, repository.path(), &["a", "b"]).await;
    tokio::fs::write(repository.path().join("a"), b"")
        .await
        .unwrap();
    tokio::fs::write(repository.path().join("b"), b"bc")
        .await
        .unwrap();
    let error = rejects(&service, repository.path(), &prepared).await;
    assert!(error.to_string().contains("changed after review"));
}

#[tokio::test]
async fn an_older_unversioned_fingerprint_requires_another_review() {
    let (repository, _state, service) = fixture().await;
    tokio::fs::write(repository.path().join("a"), b"payload")
        .await
        .unwrap();
    let mut prepared = prepare(&service, repository.path(), &["a"]).await;
    // The legacy encoder hashes empty tracked diff + filename + contents.
    // This is the actual old fingerprint of this unchanged fixture.
    prepared.worktree_fingerprint =
        Some("7e6d947cc53f2d3cee29a863ca07ff44ebb985b2cb894ee98078071296338dd3".into());
    let error = rejects(&service, repository.path(), &prepared).await;
    assert!(error.to_string().contains("older source review"));
}

#[cfg(unix)]
#[tokio::test]
async fn replacing_an_untracked_regular_file_with_a_symlink_invalidates_review() {
    let (repository, _state, service) = fixture().await;
    tokio::fs::write(repository.path().join("a"), b"before\n")
        .await
        .unwrap();
    let prepared = prepare(&service, repository.path(), &["a"]).await;
    tokio::fs::remove_file(repository.path().join("a"))
        .await
        .unwrap();
    tokio::fs::symlink("sample.txt", repository.path().join("a"))
        .await
        .unwrap();
    rejects(&service, repository.path(), &prepared).await;
}

#[cfg(unix)]
#[tokio::test]
async fn changing_untracked_executable_permissions_invalidates_review() {
    use std::os::unix::fs::PermissionsExt;
    let (repository, _state, service) = fixture().await;
    tokio::fs::write(repository.path().join("a"), b"payload")
        .await
        .unwrap();
    tokio::fs::set_permissions(
        repository.path().join("a"),
        std::fs::Permissions::from_mode(0o644),
    )
    .await
    .unwrap();
    let prepared = prepare(&service, repository.path(), &["a"]).await;
    tokio::fs::set_permissions(
        repository.path().join("a"),
        std::fs::Permissions::from_mode(0o755),
    )
    .await
    .unwrap();
    rejects(&service, repository.path(), &prepared).await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_replaced_parent_directory_cannot_redirect_selected_file_reads() {
    let (repository, _state, service) = fixture().await;
    let outside = tempfile::tempdir().unwrap();
    tokio::fs::create_dir(repository.path().join("nested"))
        .await
        .unwrap();
    tokio::fs::write(repository.path().join("nested/a"), b"payload")
        .await
        .unwrap();
    tokio::fs::write(outside.path().join("a"), b"payload")
        .await
        .unwrap();
    let prepared = prepare(&service, repository.path(), &["nested/a"]).await;
    tokio::fs::remove_dir_all(repository.path().join("nested"))
        .await
        .unwrap();
    tokio::fs::symlink(outside.path(), repository.path().join("nested"))
        .await
        .unwrap();
    let error = rejects(&service, repository.path(), &prepared).await;
    assert!(error
        .to_string()
        .contains("outside the reviewed repository"));
}

#[tokio::test]
async fn source_fingerprinting_does_not_run_external_diff_helpers() {
    let (repository, _state, service) = fixture().await;
    tokio::fs::write(repository.path().join("sample.txt"), b"changed\n")
        .await
        .unwrap();
    let path = repository.path().to_owned();
    assert!(
        tokio::task::spawn_blocking(move || std::process::Command::new("git")
            .args(["config", "diff.external", "false"])
            .current_dir(path)
            .status()
            .unwrap()
            .success())
        .await
        .unwrap()
    );
    let fingerprint = service
        .fingerprint(repository.path(), &["sample.txt".into()])
        .await
        .unwrap();
    assert!(fingerprint.starts_with("v2:"));
}

#[tokio::test]
async fn unchanged_source_still_allows_the_reviewed_command() {
    let (repository, _state, service) = fixture().await;
    tokio::fs::write(repository.path().join("a"), b"payload")
        .await
        .unwrap();
    let prepared = prepare(&service, repository.path(), &["a"]).await;
    start_reviewed_canary(&service, &prepared).await;
    let (_sender, receiver) = watch::channel(false);
    service.run_action(&prepared, receiver).await.unwrap();
    assert!(tokio::fs::try_exists(repository.path().join(CANARY))
        .await
        .unwrap());
}
