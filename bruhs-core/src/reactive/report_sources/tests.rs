use super::*;
use bruhs_models::weekly_reports::WeeklyReportPreset;

fn fixture(path: &Path) -> ReportJob {
    ReportJob {
        preset: WeeklyReportPreset {
            id: uuid::Uuid::new_v4().to_string(),
            project_path: path.display().to_string(),
            model: bruhs_models::agent::DEFAULT_AGENT_MODEL.into(),
            local_git: true,
            github_pull_requests: false,
            github_deployments: false,
            scheduled: false,
            schedule: None,
        },
        window_start: EpochMs::saturating_from_i64(1_000_000_000_000),
        window_end: EpochMs::saturating_from_i64(1_000_604_800_000),
        coverage: None,
    }
}

#[test]
fn origin_links_do_not_keep_credentials_or_accept_unrelated_hosts() {
    for origin in [
        "ssh://git@github.com/owner/repo.git",
        "ssh://git@github.com:22/owner/repo.git",
        "ssh://git@ssh.github.com:443/owner/repo.git",
        "https://GITHUB.com/owner/repo.git",
        "https://token:private@github.com/owner/repo.git",
    ] {
        assert_eq!(
            github_origin(origin),
            Some("https://github.com/owner/repo".into())
        );
    }

    assert_eq!(
        github_origin("git@github.com:owner/repo.git\n"),
        Some("https://github.com/owner/repo".into())
    );
    for value in [
        "https://github.com.evil/owner/repo",
        "https://github.com/owner/repo/extra",
        "$(touch nope)",
    ] {
        assert_eq!(github_origin(value), None);
    }
}

#[tokio::test]
async fn real_git_history_is_windowed_and_repository_text_is_inert() {
    let dir = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(init.success());
    for (time, title) in [
        (999_999_999, "too old"),
        (1_000_000_001, "$(touch injected) is a commit title"),
        (1_000_604_800, "end boundary"),
    ] {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Report Test",
                "-c",
                "user.email=reports@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                title,
            ])
            .env("GIT_AUTHOR_DATE", format!("@{time} +0000"))
            .env("GIT_COMMITTER_DATE", format!("@{time} +0000"))
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
    }
    let job = fixture(dir.path());
    let facts = git(&job).await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].title, "$(touch injected) is a commit title");
    assert!(!dir.path().join("injected").exists());
    let coverage = collect(&job).await;
    assert!(coverage
        .iter()
        .any(|item| matches!(item,ReportCoverage::Unavailable {source,..} if source=="Incidents")));
}

#[tokio::test]
async fn missing_selected_repository_is_unavailable_not_zero_activity() {
    let dir = tempfile::tempdir().unwrap();
    let coverage = collect(&fixture(dir.path())).await;
    assert!(
        matches!(&coverage[0],ReportCoverage::Unavailable {source,..} if source=="Local Git commits")
    );
}

#[test]
fn pull_references_cannot_point_to_a_different_repository() {
    assert!(pull_url(
        "https://github.com/owner/repo",
        42,
        "https://github.com/Owner/Repo/pull/42"
    )
    .is_ok());
    for url in [
        "https://github.com/other/repo/pull/42",
        "https://github.com/owner/repo/pull/43",
        "https://token@github.com/owner/repo/pull/42",
        "https://github.com.evil/owner/repo/pull/42",
    ] {
        assert!(pull_url("https://github.com/owner/repo", 42, url).is_err());
    }
}

#[tokio::test]
async fn over_budget_source_is_explicitly_unavailable() {
    let bytes = vec![b'x'; BODY_BYTES + 1];
    assert!(read_limited(bytes.as_slice(), "fixture")
        .await
        .unwrap_err()
        .contains("source budget"));
}

#[test]
fn deployment_request_needs_a_retained_status_before_the_window_end() {
    let end = timestamp("2026-11-13T15:00:00Z").unwrap();
    let status = status_at_end(
        vec![
            DeploymentStatus {
                state: "success".into(),
                created_at: "2026-11-13T16:00:00Z".into(),
            },
            DeploymentStatus {
                state: "pending".into(),
                created_at: "2026-11-13T14:00:00Z".into(),
            },
        ],
        end,
    )
    .unwrap()
    .unwrap();
    assert_eq!(status.0, "pending");
    assert!(status_at_end(
        vec![DeploymentStatus {
            state: "success".into(),
            created_at: "2026-11-13T16:00:00Z".into()
        }],
        end
    )
    .unwrap()
    .is_none());
    assert!(status_at_end(vec![], end).unwrap().is_none());
    assert!(status_at_end(
        vec![DeploymentStatus {
            state: "success".into(),
            created_at: "invalid".into()
        }],
        end
    )
    .is_err());
}

#[tokio::test]
#[ignore = "explicit read-only GitHub integration replay using the host gh login"]
async fn live_github_report_feeds() {
    let project = std::env::current_dir().unwrap();
    let mut job = fixture(&project);
    job.window_end = EpochMs::now();
    job.window_start = EpochMs::saturating_from_i64(
        job.window_end.as_millis() - bruhs_models::weekly_reports::WEEK_MS,
    );
    job.preset.local_git = false;
    job.preset.github_pull_requests = true;
    job.preset.github_deployments = true;
    let repo = repository(&job).await.unwrap();
    let (pulls, pulls_partial) = pulls(&job).await.expect("accessible GitHub PR feed");
    let (deployments, deployments_partial) = deployments(&job)
        .await
        .expect("accessible GitHub deployment feed");
    let receipt = serde_json::json!({"repository":repo,"window_start":job.window_start,"window_end":job.window_end,"pull_request_events":pulls.len(),"pull_requests_partial":pulls_partial,"deployment_requests":deployments.len(),"deployments_partial":deployments_partial,"scope":"Read-only host gh integration; deployment requests created within window, with retained statuses before its end. Counts are collected evidence rows, not reliability metrics."});
    if let Some(output) = std::env::var_os("BRUHS_REPORT_FEED_RECEIPT") {
        std::fs::write(output, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    }
    println!("{receipt}");
}
