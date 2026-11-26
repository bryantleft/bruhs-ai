//! Clone a repository into a folder explicitly selected in the project picker.
use super::*;
use bruhs_models::logic::app_link::PromptRepository;
use std::path::Component;

const PROMPT_CLONE_RECEIPT: &str = ".git/bruhs-prompt-clone";

fn folder_names(remote: &str, group_by_host: bool) -> Result<Vec<String>, GitError> {
    let invalid = || {
        GitError::Failed(
            "Enter an HTTPS or SSH repository URL, such as https://github.com/owner/repo.git."
                .into(),
        )
    };
    let normalized;
    let address = if let Some(rest) = remote.strip_prefix("git@").filter(|s| !s.contains("://")) {
        let (host, path) = rest.split_once(':').ok_or_else(invalid)?;
        normalized = format!("ssh://git@{host}/{path}");
        &normalized
    } else {
        remote
    };
    let url = url::Url::parse(address).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "ssh")
        || url.query().is_some()
        || url.fragment().is_some()
        || url.password().is_some()
        || (url.scheme() == "https" && !url.username().is_empty())
        || remote.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid());
    }
    let mut parts: Vec<_> = url
        .path_segments()
        .ok_or_else(invalid)?
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    let name = parts.last_mut().ok_or_else(invalid)?;
    if let Some(trimmed) = name.strip_suffix(".git") {
        *name = trimmed.to_owned();
    }
    let host = url.host_str().ok_or_else(invalid)?;
    if host.is_empty()
        || host.starts_with('-')
        || url.username().starts_with('-')
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'[' | b']' | b':'))
    {
        return Err(invalid());
    }
    let mut folders = Vec::new();
    if group_by_host {
        let host = host.trim_matches(['[', ']']).replace(':', "-");
        folders.push(
            url.port()
                .map_or_else(|| host.clone(), |p| format!("{host}-{p}")),
        );
        folders.extend(parts);
    } else {
        folders.push(name.clone());
    }
    if folders.iter().any(|part| {
        part.starts_with('.')
            || part.ends_with('.')
            || part.contains(['%', '\\', ':'])
            || !matches!(
                Path::new(part).components().collect::<Vec<_>>().as_slice(),
                [Component::Normal(_)]
            )
    }) {
        return Err(invalid());
    }
    Ok(folders)
}

pub async fn clone_project(
    root: &Path,
    remote: &str,
    group_by_host: bool,
) -> Result<PathBuf, GitError> {
    let remote = remote.trim();
    let folders = folder_names(remote, group_by_host)?;
    clone_to(root, remote, &folders).await
}

/// Clone the repository selected from a prompt link. Replaying the same
/// confirmed operation can recover a completed checkout after a lost reply.
pub async fn clone_prompt_repository(
    root: &Path,
    repository: &PromptRepository,
    clone_id: uuid::Uuid,
) -> Result<PathBuf, GitError> {
    let root = tokio::fs::canonicalize(root).await?;
    let name = repository.name();
    let dest = root.join(name);
    if completed_prompt_clone(&dest, repository, clone_id).await {
        return Ok(dest);
    }
    let dest = clone_to(&root, &repository.clone_url(), &[name.to_owned()]).await?;
    // A receipt is useful for recovering a network response. The checkout is
    // still usable if its Git metadata cannot store the receipt.
    if let Err(error) =
        tokio::fs::write(dest.join(PROMPT_CLONE_RECEIPT), clone_id.to_string()).await
    {
        tracing::warn!(%error, "Could not record completed prompt clone");
    }
    Ok(dest)
}

async fn completed_prompt_clone(
    dest: &Path,
    repository: &PromptRepository,
    clone_id: uuid::Uuid,
) -> bool {
    let receipt = dest.join(PROMPT_CLONE_RECEIPT);
    let expected = clone_id.to_string();
    if !tokio::fs::symlink_metadata(&receipt)
        .await
        .is_ok_and(|meta| meta.is_file() && usize::try_from(meta.len()) == Ok(expected.len()))
        || !tokio::fs::canonicalize(&receipt)
            .await
            .is_ok_and(|actual| actual == receipt)
        || !tokio::fs::read_to_string(receipt)
            .await
            .is_ok_and(|actual| actual == expected)
    {
        return false;
    }
    project_identity(dest).await.is_some_and(|identity| {
        identity.relative_path.is_empty() && repository.matches_remote(&identity.repository)
    })
}

async fn clone_to(root: &Path, remote: &str, folders: &[String]) -> Result<PathBuf, GitError> {
    let Some((name, parents)) = folders.split_last() else {
        return Err(GitError::Failed(
            "The repository has no folder name.".into(),
        ));
    };
    let root = tokio::fs::canonicalize(root).await?;
    let mut parent = root.clone();
    for part in parents {
        let candidate = parent.join(part);
        match tokio::fs::create_dir(&candidate).await {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            result => result?,
        }
        parent = tokio::fs::canonicalize(candidate).await?;
        if !parent.starts_with(&root) {
            return Err(GitError::Failed(
                "The host folder points outside the selected directory.".into(),
            ));
        }
    }
    let dest = parent.join(name);
    // Reserve the exact destination atomically. A concurrent clone or existing folder is never overwritten.
    tokio::fs::create_dir(&dest).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            GitError::Failed(format!(
                "{} already exists. Select it or choose another folder.",
                dest.display()
            ))
        } else {
            error.into()
        }
    })?;
    let result = GitCommand::new(&dest, &["clone", "--quiet", "--", remote, "."])
        .allow_network()
        .env("GIT_TERMINAL_PROMPT", "0")
        .run_with_timeout(Duration::from_secs(900))
        .await
        .and_then(|out| {
            if out.success() {
                Ok(())
            } else {
                Err(out.failure())
            }
        });
    if let Err(error) = result {
        // Only remove an empty reservation. Preserve anything else written while Git ran.
        let _ = tokio::fs::remove_dir(&dest).await;
        return Err(error);
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_folder_layout_handles_nested_groups_and_ssh() {
        assert_eq!(
            folder_names("git@example.org:design/mobile/client.git", true).unwrap(),
            ["example.org", "design", "mobile", "client"]
        );
        assert_eq!(
            folder_names("https://example.org/team/repo.git", false).unwrap(),
            ["repo"]
        );
        assert!(folder_names("https://example.org/team/%2Fbad", true).is_err());
        assert!(folder_names("https://user:token@example.org/repo", true).is_err());
        assert!(folder_names("ssh://-oProxyCommand=id/repo.git", false).is_err());
        assert!(folder_names("git@-oProxyCommand=id:team/repo.git", false).is_err());
        assert!(folder_names("ssh://-oBad@example.org/team/repo.git", false).is_err());
    }
    #[tokio::test]
    async fn clone_preserves_an_existing_checkout_and_returns_the_actual_path() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        tokio::fs::create_dir(&src).await.unwrap();
        assert!(GitCommand::new(&src, &["init", "-q"])
            .run()
            .await
            .unwrap()
            .success());
        let folders = folder_names("https://example.org/team/repo", true).unwrap();
        let checkout = clone_to(dir.path(), src.to_str().unwrap(), &folders)
            .await
            .unwrap();
        assert!(checkout.join(".git").is_dir());
        assert!(clone_to(dir.path(), src.to_str().unwrap(), &folders)
            .await
            .is_err());
        assert!(checkout.join(".git").is_dir());
    }

    #[tokio::test]
    async fn prompt_clone_retry_recovers_only_its_completed_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        tokio::fs::create_dir(&src).await.unwrap();
        assert!(git(&src, &["init", "-q"]).await.unwrap().success());
        let root = tokio::fs::canonicalize(dir.path()).await.unwrap();
        let dest = clone_to(&root, src.to_str().unwrap(), &["docs".into()])
            .await
            .unwrap();
        let repository = PromptRepository::parse("example/docs").unwrap();
        assert!(git(
            &dest,
            &["remote", "set-url", "origin", &repository.clone_url()]
        )
        .await
        .unwrap()
        .success());
        let id = uuid::Uuid::new_v4();
        assert!(!completed_prompt_clone(&dest, &repository, id).await);
        tokio::fs::write(dest.join(PROMPT_CLONE_RECEIPT), id.to_string())
            .await
            .unwrap();
        assert_eq!(
            clone_prompt_repository(&root, &repository, id)
                .await
                .unwrap(),
            dest
        );
        assert!(
            clone_prompt_repository(&root, &repository, uuid::Uuid::new_v4())
                .await
                .is_err()
        );
        assert!(git(
            &dest,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/other/docs.git"
            ]
        )
        .await
        .unwrap()
        .success());
        assert!(clone_prompt_repository(&root, &repository, id)
            .await
            .is_err());
    }
}
