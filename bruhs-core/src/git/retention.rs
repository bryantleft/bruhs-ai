//! Resolve the comparison branch for one automatic cleanup pass.
use super::{git, GitError};
use std::path::Path;

#[derive(Clone)]
pub(crate) struct DefaultBranch {
    pub name: String,
    pub commit: String,
    pub remote: bool,
}
async fn value(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = git(dir, args).await?;
    if !out.success() {
        return Err(out.failure());
    }
    Ok(out.stdout_lossy().trim().to_owned())
}
pub(crate) async fn head(dir: &Path) -> Result<String, GitError> {
    value(dir, &["rev-parse", "--verify", "HEAD^{commit}"]).await
}
pub(crate) async fn default_branch(project: &Path) -> Result<DefaultBranch, GitError> {
    let names = value(project, &["remote"]).await?;
    if names.is_empty() {
        return Ok(DefaultBranch {
            name: value(project, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?,
            commit: head(project).await?,
            remote: false,
        });
    }
    let remote = names
        .lines()
        .find(|n| *n == "origin")
        .or_else(|| names.lines().next())
        .ok_or_else(|| GitError::Failed("No git remote configured.".into()))?;
    let description = value(project, &["ls-remote", "--symref", "--", remote, "HEAD"]).await?;
    let name = description
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")?
                .strip_suffix("\tHEAD")
        })
        .ok_or_else(|| GitError::Failed("Remote default branch unavailable.".into()))?;
    let tracking = format!("refs/remotes/{remote}/{name}");
    let refspec = format!("+refs/heads/{name}:{tracking}");
    value(
        project,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--",
            remote,
            &refspec,
        ],
    )
    .await?;
    let commit = value(
        project,
        &["rev-parse", "--verify", &format!("{tracking}^{{commit}}")],
    )
    .await?;
    Ok(DefaultBranch {
        name: name.to_owned(),
        commit,
        remote: true,
    })
}
pub(crate) async fn integrated(
    project: &Path,
    head: &str,
    base: &DefaultBranch,
) -> Result<bool, GitError> {
    let result = git(
        project,
        &["merge-base", "--is-ancestor", head, &base.commit],
    )
    .await?;
    match result.status.and_then(|s| s.code()) {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(result.failure()),
    }
}
