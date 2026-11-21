#![cfg(unix)]

use super::*;
use crate::delivery::{tests::repository, ActionReceipt, Forge, PrepareRequest};

async fn fixture(script: &str) -> (tempfile::TempDir, tempfile::TempDir, DeliveryService) {
    let repository = tokio::task::spawn_blocking(repository).await.unwrap();
    let state = tempfile::tempdir().unwrap();
    tokio::fs::create_dir(repository.path().join(".bruhs"))
        .await
        .unwrap();
    tokio::fs::write(
        repository.path().join(".bruhs/config.json"),
        serde_json::to_vec(&serde_json::json!({
            "check":[{"program":"/bin/sh","args":["-c",script]}]
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    let service = DeliveryService::open(state.path()).await.unwrap();
    (repository, state, service)
}

async fn prepare(service: &DeliveryService, path: &Path, files: &[&str]) -> PreparedAction {
    service
        .prepare(PrepareRequest {
            forge: Forge::Auto,
            path: path.to_string_lossy().into_owned(),
            action: DeliveryAction::Verify {
                files: files.iter().map(|file| (*file).to_owned()).collect(),
            },
            // Same observation budget as the existing local-delivery test fixture.
            timeout_seconds: 5,
        })
        .await
        .unwrap()
}

async fn settle(service: &DeliveryService, id: &str) -> ActionReceipt {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = service
                .snapshot()
                .await
                .actions
                .into_iter()
                .find(|row| row.prepared.id == id)
                .unwrap();
            if receipt.status != ActionStatus::Running {
                return receipt;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

async fn deliver(
    service: &DeliveryService,
    path: &Path,
    files: &[&str],
    id: Option<&str>,
) -> DeliveryResult<PreparedAction> {
    service
        .prepare(PrepareRequest {
            forge: Forge::GitHub,
            path: path.to_string_lossy().into_owned(),
            timeout_seconds: 5,
            action: DeliveryAction::CommitPushPull {
                files: files.iter().map(|file| (*file).to_owned()).collect(),
                verification_id: id.map(str::to_owned),
                message: "Reviewed payload".into(),
                title: "Reviewed payload".into(),
                body: String::new(),
                base: "other".into(),
                head: "main".into(),
                draft: true,
            },
        })
        .await
}

#[tokio::test]
async fn checks_run_on_selected_payload_in_a_private_checkout_and_persist_proof() {
    let (repo, state, service) = fixture("test \"$(cat sample.txt)\" = after && test ! -e unrelated.txt && test \"$(cat nested/new.txt)\" = new && printf independent-ok").await;
    tokio::fs::create_dir(repo.path().join("nested"))
        .await
        .unwrap();
    tokio::fs::write(repo.path().join("sample.txt"), "after\n")
        .await
        .unwrap();
    tokio::fs::write(repo.path().join("nested/new.txt"), "new\n")
        .await
        .unwrap();
    tokio::fs::write(
        repo.path().join("unrelated.txt"),
        "private original workspace\n",
    )
    .await
    .unwrap();
    let prepared = prepare(&service, repo.path(), &["sample.txt", "nested/new.txt"]).await;
    assert!(!Path::new(prepared.workspace_path.as_ref().unwrap()).exists());
    assert!(service
        .snapshot()
        .await
        .actions
        .iter()
        .all(|row| row.status == ActionStatus::Prepared));
    service.execute(&prepared.id).await.unwrap();
    let receipt = settle(&service, &prepared.id).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert!(receipt.output.contains("independent-ok"));
    assert!(!Path::new(prepared.workspace_path.as_ref().unwrap()).exists());
    assert!(tokio::fs::read_to_string(repo.path().join("unrelated.txt"))
        .await
        .unwrap()
        .contains("private original"));
    let proposal = deliver(
        &service,
        repo.path(),
        &["nested/new.txt", "sample.txt"],
        Some(&prepared.id),
    )
    .await
    .unwrap();
    assert_eq!(
        proposal.verification.as_ref().unwrap().receipt_id,
        prepared.id
    );
    service
        .validate_delivery_verification(&proposal)
        .await
        .unwrap();
    drop(service);
    let reopened = DeliveryService::open(state.path()).await.unwrap();
    reopened
        .validate_delivery_verification(&proposal)
        .await
        .unwrap();
    assert!(reopened
        .snapshot()
        .await
        .actions
        .iter()
        .all(|row| row.status != ActionStatus::Running));
}

#[tokio::test]
async fn an_unselected_tracked_edit_cannot_make_the_selected_payload_pass() {
    let (repo, _state, service) = fixture("test \"$(cat sample.txt)\" = unselected-change").await;
    tokio::fs::write(repo.path().join("sample.txt"), "unselected-change\n")
        .await
        .unwrap();
    tokio::fs::write(repo.path().join("selected.txt"), "selected\n")
        .await
        .unwrap();
    let check = prepare(&service, repo.path(), &["selected.txt"]).await;
    service.execute(&check.id).await.unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert!(
        deliver(&service, repo.path(), &["selected.txt"], Some(&check.id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_checks_do_not_become_evidence_or_start_delivery() {
    let (repo, _state, service) = fixture("printf failed-check >&2; exit 23").await;
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 0);
    assert!(receipt.error.unwrap().contains("23"));
    assert!(
        deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
            .await
            .is_err()
    );
    let proposal = deliver(&service, repo.path(), &["sample.txt"], None)
        .await
        .unwrap();
    assert!(service
        .validate_delivery_verification(&proposal)
        .await
        .unwrap_err()
        .to_string()
        .contains("no independent check"));
}

#[tokio::test]
async fn source_changes_before_and_after_checking_require_fresh_evidence() {
    let (repo, _state, service) = fixture("printf checked").await;
    let stale = prepare(&service, repo.path(), &["sample.txt"]).await;
    tokio::fs::write(repo.path().join("sample.txt"), "changed before running\n")
        .await
        .unwrap();
    service.execute(&stale.id).await.unwrap();
    let receipt = settle(&service, &stale.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 0);
    assert!(receipt
        .error
        .unwrap()
        .contains("changed after check review"));
    let fresh = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&fresh.id).await.unwrap();
    assert_eq!(
        settle(&service, &fresh.id).await.status,
        ActionStatus::Completed
    );
    let proposal = deliver(&service, repo.path(), &["sample.txt"], Some(&fresh.id))
        .await
        .unwrap();
    tokio::fs::write(repo.path().join("sample.txt"), "changed after checking\n")
        .await
        .unwrap();
    assert!(service
        .validate_delivery_verification(&proposal)
        .await
        .unwrap_err()
        .to_string()
        .contains("Source changed"));
    assert!(
        deliver(&service, repo.path(), &["sample.txt"], Some(&fresh.id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn commands_that_rewrite_the_checked_snapshot_cannot_report_a_pass() {
    let (repo, _state, service) = fixture("printf rewritten > sample.txt").await;
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 1);
    assert!(receipt
        .error
        .unwrap()
        .contains("changed the reviewed source snapshot"));
    assert_eq!(
        tokio::fs::read_to_string(repo.path().join("sample.txt"))
            .await
            .unwrap(),
        "before\n"
    );
}

#[tokio::test]
async fn tracked_deletions_are_checked_as_deletions() {
    let (repo, _state, service) = fixture("test ! -e sample.txt").await;
    tokio::fs::remove_file(repo.path().join("sample.txt"))
        .await
        .unwrap();
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    assert_eq!(
        settle(&service, &check.id).await.status,
        ActionStatus::Completed
    );
    assert!(
        deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn another_selection_or_workspace_cannot_borrow_check_evidence() {
    let (repo, _state, service) = fixture("printf checked").await;
    tokio::fs::write(repo.path().join("other.txt"), "other\n")
        .await
        .unwrap();
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    assert_eq!(
        settle(&service, &check.id).await.status,
        ActionStatus::Completed
    );
    assert!(
        deliver(&service, repo.path(), &["other.txt"], Some(&check.id))
            .await
            .is_err()
    );
    let other = tokio::task::spawn_blocking(repository).await.unwrap();
    assert!(
        deliver(&service, other.path(), &["sample.txt"], Some(&check.id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mixed_case_git_internals_are_refused_before_source_reads() {
    let (repo, _state, service) = fixture("printf checked").await;
    let error = service
        .prepare(PrepareRequest {
            forge: Forge::Auto,
            path: repo.path().to_string_lossy().into_owned(),
            action: DeliveryAction::Verify {
                files: vec![".GiT/config".into()],
            },
            timeout_seconds: 5,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("internals"));
    assert!(service.snapshot().await.actions.is_empty());
}

async fn wait_for_marker(check: &PreparedAction) {
    let path = Path::new(check.workspace_path.as_ref().unwrap()).join("CHECK_READY");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !tokio::fs::try_exists(&path).await.unwrap() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_source_edit_during_checks_makes_the_completed_commands_outdated() {
    let (repo,_state,service)=fixture("printf ready > CHECK_READY; while [ ! -e CHECK_CONTINUE ]; do sleep 0.01; done; printf checked").await;
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    wait_for_marker(&check).await;
    tokio::fs::write(repo.path().join("sample.txt"), "concurrent source edit\n")
        .await
        .unwrap();
    tokio::fs::write(
        Path::new(check.workspace_path.as_ref().unwrap()).join("CHECK_CONTINUE"),
        "",
    )
    .await
    .unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 1);
    assert!(receipt
        .error
        .unwrap()
        .contains("Source changed while checks ran"));
    assert!(
        deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_stops_active_checks_cleans_the_snapshot_and_grants_no_evidence() {
    let (repo, _state, service) =
        fixture("printf ready > CHECK_READY; exec tail -f /dev/null").await;
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    wait_for_marker(&check).await;
    service.cancel(&check.id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while tokio::fs::try_exists(Path::new(check.workspace_path.as_ref().unwrap()))
            .await
            .unwrap()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        settle(&service, &check.id).await.status,
        ActionStatus::Cancelled
    );
    assert!(
        deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_explicit_owner_budget_stops_checks_without_recording_a_pass() {
    let (repo, _state, service) =
        fixture("printf ready > CHECK_READY; exec tail -f /dev/null").await;
    let mut check = prepare(&service, repo.path(), &["sample.txt"]).await;
    // Exercise an owner-chosen one-second budget; this is not a product SLA.
    service
        .mutate(|state| {
            state.actions[0].prepared.timeout_seconds = 1;
            Ok(())
        })
        .await
        .unwrap();
    check.timeout_seconds = 1;
    service.execute(&check.id).await.unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert!(receipt
        .error
        .unwrap()
        .contains("exceeded the reviewed 1 second budget"));
    assert_eq!(receipt.completed_commands, 0);
    assert!(!Path::new(check.workspace_path.as_ref().unwrap()).exists());
}

#[tokio::test]
async fn existing_private_workspace_is_preserved_and_cannot_be_adopted() {
    let (repo, _state, service) = fixture("printf checked").await;
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    let path = Path::new(check.workspace_path.as_ref().unwrap());
    tokio::fs::create_dir_all(path).await.unwrap();
    tokio::fs::write(path.join("KEEP"), "not owned by this run")
        .await
        .unwrap();
    service.execute(&check.id).await.unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(
        tokio::fs::read_to_string(path.join("KEEP")).await.unwrap(),
        "not owned by this run"
    );
}

async fn git(path: &Path, args: &[&str]) -> String {
    let output = tokio::process::Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[tokio::test]
async fn staged_new_files_have_the_same_payload_identity_as_unstaged_new_files() {
    let (repo, _state, service) = fixture("test \"$(cat new.txt)\" = selected").await;
    tokio::fs::write(repo.path().join("new.txt"), "selected\n")
        .await
        .unwrap();
    let files = vec!["new.txt".into()];
    let before = service
        .payload_fingerprint(repo.path(), &files)
        .await
        .unwrap();
    git(repo.path(), &["add", "new.txt"]).await;
    assert_eq!(
        before,
        service
            .payload_fingerprint(repo.path(), &files)
            .await
            .unwrap()
    );
    let check = prepare(&service, repo.path(), &["new.txt"]).await;
    service.execute(&check.id).await.unwrap();
    let receipt = settle(&service, &check.id).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert!(receipt.checked_tree.is_some());
}

#[tokio::test]
async fn actual_commit_tree_is_checked_before_push_and_remote_write_is_local_only() {
    let (repo, state, mut service) = fixture("test \"$(cat sample.txt)\" = after").await;
    let bare = tempfile::tempdir().unwrap();
    git(
        repo.path(),
        &["init", "--bare", bare.path().to_str().unwrap()],
    )
    .await;
    // Native Git can only reach this owned local bare repository. If URL
    // rewriting fails, its network protocol refusal stops before any contact.
    git(repo.path(), &["config", "protocol.https.allow", "never"]).await;
    git(repo.path(), &["config", "protocol.ssh.allow", "never"]).await;
    git(
        repo.path(),
        &[
            "config",
            &format!("url.{}.insteadOf", bare.path().display()),
            "https://github.com/fixture/repo.git",
        ],
    )
    .await;
    tokio::fs::write(repo.path().join("sample.txt"), "after\n")
        .await
        .unwrap();
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    assert_eq!(
        settle(&service, &check.id).await.status,
        ActionStatus::Completed
    );
    let proposal = deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
        .await
        .unwrap();
    let gh = state.path().join("fixture-gh");
    tokio::fs::write(
        &gh,
        "#!/bin/sh\nprintf 'https://github.com/fixture/repo/pull/1\\n'\n",
    )
    .await
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700))
        .await
        .unwrap();
    service.gh = std::sync::Arc::new(gh);
    service.execute(&proposal.id).await.unwrap();
    let receipt = settle(&service, &proposal.id).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert_eq!(receipt.completed_commands, proposal.commands.len());
    let commit = receipt.checked_commit.unwrap();
    assert_eq!(
        git(
            repo.path(),
            &[
                "--git-dir",
                bare.path().to_str().unwrap(),
                "rev-parse",
                "refs/heads/main"
            ]
        )
        .await,
        commit
    );
    assert_eq!(
        git(repo.path(), &["rev-parse", &format!("{commit}^{{tree}}")]).await,
        proposal.verification.unwrap().checked_tree
    );
    assert_eq!(
        git(repo.path(), &["config", "branch.main.merge"]).await,
        "refs/heads/main"
    );
}

#[tokio::test]
async fn a_commit_hook_or_concurrent_edit_cannot_publish_a_tree_that_was_not_checked() {
    let (repo, _state, service) = fixture("printf checked").await;
    tokio::fs::write(repo.path().join("sample.txt"), "reviewed\n")
        .await
        .unwrap();
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    assert_eq!(
        settle(&service, &check.id).await.status,
        ActionStatus::Completed
    );
    let proposal = deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
        .await
        .unwrap();
    tokio::fs::write(repo.path().join("sample.txt"), "different commit payload\n")
        .await
        .unwrap();
    git(repo.path(), &["add", "sample.txt"]).await;
    git(repo.path(), &["commit", "-m", "Reviewed payload"]).await;
    let push = proposal
        .commands
        .iter()
        .find(|command| command.args.first().map(String::as_str) == Some("push"))
        .unwrap();
    let error = service
        .resolve_checked_command(&proposal, push)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Nothing was pushed"));
    assert!(service
        .snapshot()
        .await
        .actions
        .into_iter()
        .find(|row| row.prepared.id == proposal.id)
        .unwrap()
        .checked_commit
        .is_none());
}

#[tokio::test]
async fn resolved_push_keeps_the_checked_commit_and_destination_after_branch_and_origin_move() {
    let (repo, _state, service) = fixture("printf checked").await;
    tokio::fs::write(repo.path().join("sample.txt"), "reviewed\n")
        .await
        .unwrap();
    let check = prepare(&service, repo.path(), &["sample.txt"]).await;
    service.execute(&check.id).await.unwrap();
    assert_eq!(
        settle(&service, &check.id).await.status,
        ActionStatus::Completed
    );
    let proposal = deliver(&service, repo.path(), &["sample.txt"], Some(&check.id))
        .await
        .unwrap();
    git(repo.path(), &["add", "sample.txt"]).await;
    git(repo.path(), &["commit", "-m", "Reviewed payload"]).await;
    let commit = git(repo.path(), &["rev-parse", "HEAD"]).await;
    service
        .mutate(|state| {
            state
                .actions
                .iter_mut()
                .find(|row| row.prepared.id == proposal.id)
                .unwrap()
                .status = ActionStatus::Running;
            Ok(())
        })
        .await
        .unwrap();
    let push = proposal
        .commands
        .iter()
        .find(|command| command.args.first().map(String::as_str) == Some("push"))
        .unwrap();
    let resolved = service
        .resolve_checked_command(&proposal, push)
        .await
        .unwrap()
        .unwrap();
    tokio::fs::write(repo.path().join("sample.txt"), "later unrelated work\n")
        .await
        .unwrap();
    git(repo.path(), &["commit", "-am", "Other work"]).await;
    git(
        repo.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/fixture/other.git",
        ],
    )
    .await;
    assert_ne!(git(repo.path(), &["rev-parse", "HEAD"]).await, commit);
    assert_eq!(
        resolved.args,
        vec![
            "push".to_owned(),
            "https://github.com/fixture/repo.git".to_owned(),
            format!("{commit}:refs/heads/main")
        ]
    );
}
