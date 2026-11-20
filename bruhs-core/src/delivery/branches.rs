//! Read-only branch lookups for the task board: the branch a working
//! directory is on, and the pull or merge requests opened from a branch.
use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::{
    delivery::{Forge, Pull},
    logic::git::BranchName,
};
use std::path::Path;

/// The newest few requests from one branch are enough to place a card;
/// older ones were superseded.
const PER_BRANCH: &str = "5";
/// What a board card needs: no body or file list, which can be large enough
/// to exceed the delivery output budget for five requests.
const BOARD_FIELDS: &str = "number,title,url,state,isDraft,isCrossRepository,baseRefName,baseRefOid,headRefName,headRefOid,updatedAt,mergeable,reviewDecision,statusCheckRollup,comments,reviews";

impl DeliveryService {
    /// The checked-out branch, or `None` for a detached HEAD.
    pub async fn current_branch(&self, path: &Path) -> DeliveryResult<Option<String>> {
        let output = self
            .read(path, "git", &["rev-parse", "--abbrev-ref", "HEAD"])
            .await?;
        let branch = output.trim();
        Ok((!branch.is_empty() && branch != "HEAD").then(|| branch.to_owned()))
    }

    /// Open, merged and closed requests whose head is `branch`, newest first.
    pub async fn pulls_from_branch(&self, path: &Path, branch: &str) -> DeliveryResult<Vec<Pull>> {
        let branch = BranchName::try_from(branch)
            .map_err(|_| DeliveryError::Invalid("The task branch name is not valid.".into()))?;
        match self.forge_repository(path, Forge::Auto).await? {
            Some(repo) if repo.forge == Forge::GitLab => {
                self.gitlab_pulls_from_branch(path, &repo, branch.as_str())
                    .await
            }
            _ => {
                // `--head=` keeps a branch that starts like a flag a value.
                let head = format!("--head={}", branch.as_str());
                let output = self
                    .github_read(
                        path,
                        &[
                            "pr",
                            "list",
                            &head,
                            "--state",
                            "all",
                            "--limit",
                            PER_BRANCH,
                            "--json",
                            BOARD_FIELDS,
                        ],
                    )
                    .await?;
                super::github::parse_pulls(&output)
            }
        }
    }
}
