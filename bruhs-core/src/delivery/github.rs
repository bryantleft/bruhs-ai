use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::{Check, Issue, Pull, PullComment};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

const FIELDS: &str = "number,title,body,url,state,isDraft,isCrossRepository,baseRefName,baseRefOid,headRefName,headRefOid,mergeCommit,updatedAt,mergeable,reviewDecision,statusCheckRollup,comments,reviews,files";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPull {
    number: u64,
    title: String,
    #[serde(default)]
    body: String,
    url: String,
    state: String,
    is_draft: bool,
    is_cross_repository: bool,
    base_ref_name: String,
    base_ref_oid: String,
    head_ref_name: String,
    head_ref_oid: String,
    merge_commit: Option<Value>,
    updated_at: String,
    #[serde(default)]
    mergeable: String,
    review_decision: Option<String>,
    #[serde(default)]
    status_check_rollup: Vec<Value>,
    #[serde(default)]
    comments: Vec<Value>,
    #[serde(default)]
    reviews: Vec<Value>,
    #[serde(default)]
    files: Vec<Value>,
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn comment(value: &Value) -> PullComment {
    PullComment {
        id: value["id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| value["id"].as_u64().map(|id| id.to_string()))
            .unwrap_or_default(),
        author: value["author"]["login"]
            .as_str()
            .or_else(|| value["user"]["login"].as_str())
            .unwrap_or_default()
            .to_owned(),
        body: text(value, "body"),
        url: value["url"]
            .as_str()
            .or_else(|| value["html_url"].as_str())
            .map(str::to_owned),
        state: value["state"].as_str().map(str::to_owned),
        path: value["path"].as_str().map(str::to_owned),
        line: value["line"]
            .as_u64()
            .and_then(|line| u32::try_from(line).ok()),
    }
}

impl From<GitHubPull> for Pull {
    fn from(pull: GitHubPull) -> Self {
        Self {
            number: pull.number,
            title: pull.title,
            body: pull.body,
            url: pull.url,
            state: pull.state,
            draft: pull.is_draft,
            base: pull.base_ref_name,
            head: pull.head_ref_name,
            head_oid: pull.head_ref_oid,
            head_in_repository: !pull.is_cross_repository,
            base_oid: pull.base_ref_oid,
            merge_commit_oid: pull
                .merge_commit
                .as_ref()
                .and_then(|commit| commit["oid"].as_str().map(str::to_owned)),
            updated_at: pull.updated_at,
            mergeable: pull.mergeable,
            review_decision: pull.review_decision,
            checks: pull
                .status_check_rollup
                .iter()
                .map(|check| Check {
                    name: check["name"]
                        .as_str()
                        .or_else(|| check["context"].as_str())
                        .unwrap_or("Check")
                        .to_owned(),
                    state: check["status"]
                        .as_str()
                        .or_else(|| check["state"].as_str())
                        .unwrap_or("UNKNOWN")
                        .to_owned(),
                    conclusion: check["conclusion"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                    url: check["detailsUrl"]
                        .as_str()
                        .or_else(|| check["targetUrl"].as_str())
                        .map(str::to_owned),
                })
                .collect(),
            comments: pull.comments.iter().map(comment).collect(),
            reviews: pull.reviews.iter().map(comment).collect(),
            files: pull
                .files
                .iter()
                .filter_map(|file| file["path"].as_str().map(str::to_owned))
                .collect(),
        }
    }
}

pub(super) fn parse_pulls(output: &str) -> DeliveryResult<Vec<Pull>> {
    let pulls: Vec<GitHubPull> = serde_json::from_str(output).map_err(|error| {
        DeliveryError::Command(format!(
            "GitHub returned invalid pull request data: {error}"
        ))
    })?;
    Ok(pulls.into_iter().map(Pull::from).collect())
}

impl DeliveryService {
    pub(super) async fn github_pulls(&self, path: &Path) -> DeliveryResult<Vec<Pull>> {
        // A page has the same 50-record bound as reactive history; all larger result sets
        // are explicit via the host's own CLI rather than silently unbounded allocations.
        let output = self
            .read(
                path,
                "gh",
                &[
                    "pr", "list", "--state", "open", "--limit", "50", "--json", FIELDS,
                ],
            )
            .await?;
        parse_pulls(&output)
    }

    pub(super) async fn github_pull(&self, path: &Path, number: u64) -> DeliveryResult<Pull> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive pull request number.".into(),
            ));
        }
        let output = self
            .read(
                path,
                "gh",
                &["pr", "view", &number.to_string(), "--json", FIELDS],
            )
            .await?;
        serde_json::from_str::<GitHubPull>(&output)
            .map(Pull::from)
            .map_err(|error| {
                DeliveryError::Command(format!(
                    "GitHub returned invalid pull request data: {error}"
                ))
            })
    }

    pub(super) async fn github_diff(&self, path: &Path, number: u64) -> DeliveryResult<String> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive pull request number.".into(),
            ));
        }
        self.read(path, "gh", &["pr", "diff", &number.to_string(), "--patch"])
            .await
    }

    pub(super) async fn github_line_comments(
        &self,
        path: &Path,
        number: u64,
    ) -> DeliveryResult<Vec<PullComment>> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive pull request number.".into(),
            ));
        }
        let output = self
            .read(
                path,
                "gh",
                &[
                    "api",
                    &format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments?per_page=100"),
                    "--paginate",
                    "--slurp",
                ],
            )
            .await?;
        let pages: Vec<Vec<Value>> = serde_json::from_str(&output).map_err(|error| {
            DeliveryError::Command(format!("GitHub review comments were invalid: {error}"))
        })?;
        Ok(pages.iter().flatten().map(comment).collect())
    }

    pub(super) async fn github_issue(&self, path: &Path, number: u64) -> DeliveryResult<Issue> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive issue number.".into(),
            ));
        }
        let output = self
            .read(
                path,
                "gh",
                &[
                    "issue",
                    "view",
                    &number.to_string(),
                    "--json",
                    "number,title,body,url",
                ],
            )
            .await?;
        serde_json::from_str(&output).map_err(|error| {
            DeliveryError::Command(format!("GitHub returned invalid issue data: {error}"))
        })
    }
}
