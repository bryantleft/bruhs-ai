//! Pull requests in the current repo, via `gh` argv. No shell.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

const GH_FIELDS: &str = "number,title,url,state,isDraft";
const GH_TIMEOUT: Duration = Duration::from_secs(8);

/// One pull request as attached to a composer chip.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PullRef {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `open`, `draft`, `merged`, or `closed`, frozen at lookup time.
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullList {
    pub pulls: Vec<PullRef>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct GhPull {
    number: u64,
    title: String,
    url: String,
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
}

/// Newest pull requests, filtered by `query`.
///
/// Digits keep recent numbers that contain those digits, and an exact
/// number is loaded even when it is older than the recent list. Any other
/// query searches by text. A missing `gh` is an empty list plus `error`.
pub async fn list_pulls(cwd: &Path, query: &str) -> PullList {
    let query = query.trim();
    if query.is_empty() {
        return recent(cwd).await;
    }
    if query.bytes().all(|byte| byte.is_ascii_digit()) {
        return by_number(cwd, query).await;
    }
    match gh(
        cwd,
        &[
            "pr", "list", "--search", query, "--json", GH_FIELDS, "--limit", "30",
        ],
    )
    .await
    {
        Ok(stdout) => match parse_pulls(&stdout) {
            Ok(pulls) => PullList { pulls, error: None },
            Err(error) => PullList {
                pulls: Vec::new(),
                error: Some(error),
            },
        },
        Err(error) => PullList {
            pulls: Vec::new(),
            error: Some(error),
        },
    }
}

async fn recent(cwd: &Path) -> PullList {
    match gh(cwd, &["pr", "list", "--json", GH_FIELDS, "--limit", "30"]).await {
        Ok(stdout) => match parse_pulls(&stdout) {
            Ok(pulls) => PullList { pulls, error: None },
            Err(error) => PullList {
                pulls: Vec::new(),
                error: Some(error),
            },
        },
        Err(error) => PullList {
            pulls: Vec::new(),
            error: Some(error),
        },
    }
}

async fn by_number(cwd: &Path, query: &str) -> PullList {
    let mut listed = recent(cwd).await;
    if listed.error.is_some() && listed.pulls.is_empty() {
        return listed;
    }
    listed
        .pulls
        .retain(|pull| pull.number.to_string().contains(query));
    if let Ok(exact) = view_number(cwd, query).await {
        if let Some(pos) = listed
            .pulls
            .iter()
            .position(|pull| pull.number == exact.number)
        {
            let found = listed.pulls.remove(pos);
            listed.pulls.insert(0, found);
        } else {
            listed.pulls.insert(0, exact);
        }
    }
    listed
}

async fn view_number(cwd: &Path, number: &str) -> Result<PullRef, String> {
    let stdout = gh(cwd, &["pr", "view", number, "--json", GH_FIELDS]).await?;
    let parsed: GhPull = serde_json::from_str(stdout.trim()).map_err(|error| error.to_string())?;
    Ok(to_ref(parsed))
}

async fn gh(cwd: &Path, args: &[&str]) -> Result<String, String> {
    // Colored JSON is not JSON. NO_COLOR loses to CLICOLOR_FORCE and FORCE_COLOR.
    let output = Command::new("gh")
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .env_remove("GH_FORCE_TTY")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(GH_TIMEOUT, output).await {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8(output.stdout).map_err(|error| error.to_string())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let line = stderr.lines().next().unwrap_or("").trim();
            if line.is_empty() {
                Err("gh could not list pull requests.".into())
            } else {
                Err(line.to_string())
            }
        }
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Err("GitHub CLI (gh) is not installed.".into())
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("gh timed out.".into()),
    }
}

fn parse_pulls(stdout: &str) -> Result<Vec<PullRef>, String> {
    let parsed: Vec<GhPull> =
        serde_json::from_str(stdout.trim()).map_err(|error| error.to_string())?;
    Ok(parsed.into_iter().map(to_ref).collect())
}

/// The pull request `gh` found for one branch, plus whether `gh` is signed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPullLookup {
    pub signed_in: bool,
    pub pull: Option<PullRef>,
    pub error: Option<String>,
}

/// The GitHub pull request for `branch` in the repo at `cwd`.
///
/// A signed-out `gh` returns `signed_in: false` and no error, so the panel
/// stays closed. A missing `gh`, a timeout, or a failed list sets `error`.
/// An open or draft request wins over a merged or closed one.
pub async fn pull_for_branch(cwd: &Path, branch: &str) -> BranchPullLookup {
    let branch = branch.trim();
    if branch.is_empty() {
        return BranchPullLookup {
            signed_in: false,
            pull: None,
            error: Some("This thread has no branch.".into()),
        };
    }
    match github_signed_in(cwd).await {
        Ok(false) => BranchPullLookup {
            signed_in: false,
            pull: None,
            error: None,
        },
        Err(error) => BranchPullLookup {
            signed_in: false,
            pull: None,
            error: Some(error),
        },
        Ok(true) => {
            match gh(
                cwd,
                &[
                    "pr", "list", "--head", branch, "--state", "all", "--json", GH_FIELDS,
                    "--limit", "20",
                ],
            )
            .await
            {
                Ok(stdout) => match parse_pulls(&stdout) {
                    Ok(pulls) => BranchPullLookup {
                        signed_in: true,
                        pull: prefer_open_pull(&pulls).cloned(),
                        error: None,
                    },
                    Err(error) => BranchPullLookup {
                        signed_in: true,
                        pull: None,
                        error: Some(error),
                    },
                },
                Err(error) => BranchPullLookup {
                    signed_in: true,
                    pull: None,
                    error: Some(error),
                },
            }
        }
    }
}

/// `Ok(true)` is signed in. `Ok(false)` is signed out. `Err` is a missing
/// `gh` or a timeout, which the panel can show.
async fn github_signed_in(cwd: &Path) -> Result<bool, String> {
    match gh(cwd, &["auth", "status"]).await {
        Ok(_) => Ok(true),
        Err(error) if error == "GitHub CLI (gh) is not installed." || error == "gh timed out." => {
            Err(error)
        }
        Err(_) => Ok(false),
    }
}

/// An open or draft pull request, otherwise the first listed.
fn prefer_open_pull(pulls: &[PullRef]) -> Option<&PullRef> {
    pulls
        .iter()
        .find(|pull| pull.state == "open" || pull.state == "draft")
        .or_else(|| pulls.first())
}

fn to_ref(pull: GhPull) -> PullRef {
    let state = if pull.is_draft {
        "draft".to_string()
    } else {
        pull.state.to_ascii_lowercase()
    };
    PullRef {
        number: pull.number,
        title: pull.title,
        url: pull.url,
        state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_draft_and_an_open_pull_request() {
        let raw = r#"[
            {"number":3,"title":"Gate","url":"https://example/3","state":"OPEN","isDraft":true},
            {"number":4,"title":"Road","url":"https://example/4","state":"MERGED","isDraft":false}
        ]"#;
        let pulls = parse_pulls(raw).unwrap();
        assert_eq!(pulls[0].state, "draft");
        assert_eq!(pulls[0].number, 3);
        assert_eq!(pulls[1].state, "merged");
        assert_eq!(pulls[1].title, "Road");
    }

    #[test]
    fn an_open_pull_request_wins_over_an_older_merged_one() {
        let pulls = parse_pulls(
            r#"[
            {"number":1,"title":"Old","url":"https://example/1","state":"MERGED","isDraft":false},
            {"number":2,"title":"Now","url":"https://example/2","state":"OPEN","isDraft":true}
        ]"#,
        )
        .unwrap();
        assert_eq!(prefer_open_pull(&pulls).unwrap().number, 2);
        assert_eq!(prefer_open_pull(&pulls).unwrap().state, "draft");
        assert!(prefer_open_pull(&[]).is_none());
    }

    #[tokio::test]
    async fn a_thread_without_a_branch_does_not_ask_gh() {
        let listed = pull_for_branch(Path::new("."), "  ").await;
        assert!(!listed.signed_in);
        assert!(listed.pull.is_none());
        assert_eq!(listed.error.as_deref(), Some("This thread has no branch."));
    }
}
