use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::*;
use std::path::Path;

pub(super) fn repository(remote: &str, requested: Forge) -> DeliveryResult<ForgeRepository> {
    let invalid = || {
        DeliveryError::Invalid("Use a credential-free HTTPS or SSH GitHub/GitLab origin. Choose GitLab explicitly for a self-hosted instance.".into())
    };
    let (host, project) = if remote.contains("://") {
        let url = url::Url::parse(remote).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "https" | "ssh")
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port().is_some()
            || url.scheme() == "https" && !url.username().is_empty()
        {
            return Err(invalid());
        }
        (
            url.host_str().ok_or_else(invalid)?.to_owned(),
            url.path().trim_matches('/').to_owned(),
        )
    } else {
        let (authority, project) = remote.split_once(':').ok_or_else(invalid)?;
        let host = authority.strip_prefix("git@").unwrap_or(authority);
        (host.to_owned(), project.trim_matches('/').to_owned())
    };
    let project = project.strip_suffix(".git").unwrap_or(&project).to_owned();
    if host.is_empty()
        || host.starts_with('-')
        || host
            .split('.')
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
        || project.split('/').count() < 2
        || project.split('/').any(|p| {
            p.is_empty()
                || p == ".."
                || p == "."
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        })
    {
        return Err(invalid());
    }
    let forge = match requested {
        Forge::Auto if host.eq_ignore_ascii_case("gitlab.com") => Forge::GitLab,
        Forge::Auto if host.eq_ignore_ascii_case("github.com") => Forge::GitHub,
        Forge::Auto => return Err(invalid()),
        value => value,
    };
    Ok(ForgeRepository {
        forge,
        host,
        project,
        remote_url: remote.into(),
    })
}

impl DeliveryService {
    pub(super) async fn forge_repository(
        &self,
        path: &Path,
        requested: Forge,
    ) -> DeliveryResult<Option<ForgeRepository>> {
        let remote = self
            .read(path, "git", &["config", "--get", "remote.origin.url"])
            .await;
        match remote {
            Ok(remote)=>repository(remote.trim(),requested).map(Some),
            Err(_) if requested!=Forge::GitLab=>Ok(None),
            Err(_)=>Err(DeliveryError::Invalid("GitLab delivery requires remote.origin.url. Add its HTTPS/SSH repository URL and sign in with glab auth login.".into())),
        }
    }
    pub async fn pulls_for(&self, path: &Path, forge: Forge) -> DeliveryResult<Vec<Pull>> {
        match self.forge_repository(path, forge).await? {
            Some(repo) if repo.forge == Forge::GitLab => self.gitlab_pulls(path, &repo).await,
            _ => self.github_pulls(path).await,
        }
    }
    pub async fn pull_for(&self, path: &Path, number: u64, forge: Forge) -> DeliveryResult<Pull> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive review number.".into(),
            ));
        }
        match self.forge_repository(path, forge).await? {
            Some(repo) if repo.forge == Forge::GitLab => {
                self.gitlab_pull(path, &repo, number).await
            }
            _ => self.github_pull(path, number).await,
        }
    }
    pub async fn diff_for(&self, path: &Path, number: u64, forge: Forge) -> DeliveryResult<String> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive review number.".into(),
            ));
        }
        match self.forge_repository(path, forge).await? {
            Some(repo) if repo.forge == Forge::GitLab => {
                self.gitlab_diff(path, &repo, number).await
            }
            _ => self.github_diff(path, number).await,
        }
    }
    pub async fn comments_for(
        &self,
        path: &Path,
        number: u64,
        forge: Forge,
    ) -> DeliveryResult<Vec<PullComment>> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive review number.".into(),
            ));
        }
        match self.forge_repository(path, forge).await? {
            Some(repo) if repo.forge == Forge::GitLab => {
                self.gitlab_comments(path, &repo, number).await
            }
            _ => self.github_line_comments(path, number).await,
        }
    }
    pub async fn issue_for(&self, path: &Path, number: u64, forge: Forge) -> DeliveryResult<Issue> {
        if number == 0 {
            return Err(DeliveryError::Invalid(
                "Choose a positive issue number.".into(),
            ));
        }
        match self.forge_repository(path, forge).await? {
            Some(repo) if repo.forge == Forge::GitLab => {
                self.gitlab_issue(path, &repo, number).await
            }
            _ => self.github_issue(path, number).await,
        }
    }
    pub async fn pulls(&self, path: &Path) -> DeliveryResult<Vec<Pull>> {
        self.pulls_for(path, Forge::Auto).await
    }
    pub async fn pull(&self, path: &Path, number: u64) -> DeliveryResult<Pull> {
        self.pull_for(path, number, Forge::Auto).await
    }
    pub async fn diff(&self, path: &Path, number: u64) -> DeliveryResult<String> {
        self.diff_for(path, number, Forge::Auto).await
    }
    pub async fn line_comments(
        &self,
        path: &Path,
        number: u64,
    ) -> DeliveryResult<Vec<PullComment>> {
        self.comments_for(path, number, Forge::Auto).await
    }
    pub async fn issue(&self, path: &Path, number: u64) -> DeliveryResult<Issue> {
        self.issue_for(path, number, Forge::Auto).await
    }
}
