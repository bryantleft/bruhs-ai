//! A fork's gh default must not redirect the reviewed project or an API placeholder.

use super::*;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Command as Process;

fn git(path: &Path, args: &[&str]) {
    let output = Process::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run owned Git fixture command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn service(directory: &Path) -> DeliveryService {
    let executable = directory.join("record-gh");
    std::fs::write(
        &executable,
        "#!/bin/sh\n/bin/cat >/dev/null\nprintf '%s|%s\\n' \"${GH_REPO-unset}\" \"${GH_HOST-unset}\"\nprintf ran >> gh-called\n",
    )
    .expect("write owned recording CLI");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("make owned recording CLI executable");
    let mut service = DeliveryService::open(directory.join("delivery"))
        .await
        .expect("open isolated delivery fixture");
    service.gh = Arc::new(executable);
    service
}

async fn settled(service: &DeliveryService, id: &str) -> ActionReceipt {
    tokio::time::timeout(crate::capture_client::REQUEST_TIMEOUT, async {
        loop {
            let receipt = service
                .snapshot()
                .await
                .actions
                .into_iter()
                .find(|action| action.prepared.id == id)
                .expect("prepared fixture receipt remains available");
            if receipt.status != ActionStatus::Running {
                break receipt;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned delivery fixture must settle within the observation budget")
}

fn comment(path: &Path) -> PrepareRequest {
    PrepareRequest {
        path: path.to_string_lossy().into_owned(),
        forge: Forge::GitHub,
        action: DeliveryAction::Comment {
            number: 42,
            body: "Synthetic fixture only".into(),
        },
        timeout_seconds: 5,
    }
}

#[tokio::test]
async fn github_reads_pin_origin_and_host_despite_a_different_saved_gh_default() {
    let directory = tempfile::tempdir().unwrap();
    let repository = tests::repository();
    git(
        repository.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/other/project.git",
        ],
    );
    git(
        repository.path(),
        &["config", "remote.upstream.gh-resolved", "base"],
    );
    let service = service(directory.path()).await;
    for (origin, selected) in [
        (
            "https://github.com/fixture/repo.git",
            "github.com/fixture/repo|github.com",
        ),
        (
            "https://ghe.example/team/project.git",
            "ghe.example/team/project|ghe.example",
        ),
    ] {
        git(repository.path(), &["remote", "set-url", "origin", origin]);
        let output = service
            .github_read(repository.path(), &["api", "repos/{owner}/{repo}/pulls/42"])
            .await
            .unwrap();
        assert_eq!(output.trim(), selected);
    }
}

#[tokio::test]
async fn reviewed_execution_uses_frozen_origin_after_gh_default_changes() {
    let directory = tempfile::tempdir().unwrap();
    let repository = tests::repository();
    let service = service(directory.path()).await;
    let prepared = service.prepare(comment(repository.path())).await.unwrap();
    git(
        repository.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://github.com/other/project.git",
        ],
    );
    git(
        repository.path(),
        &["config", "remote.upstream.gh-resolved", "base"],
    );
    service.execute(&prepared.id).await.unwrap();
    let receipt = settled(&service, &prepared.id).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert!(
        receipt
            .output
            .contains("github.com/fixture/repo|github.com"),
        "{}",
        receipt.output
    );
    assert_eq!(
        std::fs::read_to_string(repository.path().join("gh-called")).unwrap(),
        "ran"
    );
}

#[tokio::test]
async fn missing_origin_refuses_github_read_and_proposal_without_starting_gh() {
    let directory = tempfile::tempdir().unwrap();
    let repository = tests::repository();
    git(repository.path(), &["remote", "remove", "origin"]);
    let service = service(directory.path()).await;
    let error = service
        .github_read(repository.path(), &["pr", "view", "42"])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("origin"));
    assert!(service
        .prepare(comment(repository.path()))
        .await
        .unwrap_err()
        .to_string()
        .contains("origin"));
    assert!(!repository.path().join("gh-called").exists());
    assert!(service.snapshot().await.actions.is_empty());
}

#[tokio::test]
async fn an_old_unscoped_receipt_cannot_start_a_github_write() {
    let directory = tempfile::tempdir().unwrap();
    let repository = tests::repository();
    let service = service(directory.path()).await;
    let prepared = service.prepare(comment(repository.path())).await.unwrap();
    service
        .mutate(|journal| {
            journal.actions[0].prepared.repository = None;
            Ok(())
        })
        .await
        .unwrap();
    service.execute(&prepared.id).await.unwrap();
    let receipt = settled(&service, &prepared.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert!(receipt.error.unwrap().contains("no reviewed repository"));
    assert!(!repository.path().join("gh-called").exists());
}

#[tokio::test]
async fn an_old_unscoped_publish_is_refused_before_staging_committing_or_pushing() {
    let directory = tempfile::tempdir().unwrap();
    let repository = tests::repository();
    git(repository.path(), &["switch", "-qc", "feature"]);
    std::fs::write(repository.path().join("sample.txt"), "pending change\n").unwrap();
    let mut service = service(directory.path()).await;
    let prepared = service
        .prepare(PrepareRequest {
            path: repository.path().to_string_lossy().into_owned(),
            forge: Forge::GitHub,
            action: DeliveryAction::CommitPushPull {
                files: vec!["sample.txt".into()],
                verification_id: None,
                message: "Fixture change".into(),
                title: "Fixture proposal".into(),
                body: "Never published".into(),
                base: "main".into(),
                head: "feature".into(),
                draft: true,
            },
            timeout_seconds: 5,
        })
        .await
        .unwrap();
    // Never allow this regression test to reach a real remote or change its
    // index. A refused mutation attempt is still a failing test observation.
    let guarded_git = directory.path().join("guarded-git");
    std::fs::write(&guarded_git, "#!/bin/sh\nfor arg in \"$@\"; do\ncase \"$arg\" in add|commit|push) printf attempted > unexpected-write; exit 1;; esac\ndone\nexec /usr/bin/git \"$@\"\n").unwrap();
    std::fs::set_permissions(&guarded_git, std::fs::Permissions::from_mode(0o700)).unwrap();
    service.git = Arc::new(guarded_git);
    service
        .mutate(|journal| {
            journal.actions[0].prepared.repository = None;
            Ok(())
        })
        .await
        .unwrap();
    service.execute(&prepared.id).await.unwrap();
    let receipt = settled(&service, &prepared.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 0);
    assert!(receipt.error.unwrap().contains("no reviewed repository"));
    assert!(!repository.path().join("unexpected-write").exists());
    assert!(!repository.path().join("gh-called").exists());
    git(repository.path(), &["diff", "--cached", "--exit-code"]);
}
