//! Local repository metadata used to group checkouts without contacting a remote.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectIdentity {
    pub repository: String,
    pub relative_path: String,
    pub label: String,
}

/// Normalize equivalent SSH and HTTPS fetch URLs. User information and URL
/// query parameters never become part of a displayed or persisted group key.
fn network_origin(raw: &str) -> Option<String> {
    let scp;
    let address = if raw.contains("://") {
        raw
    } else {
        let (host, path) = raw.split_once(':')?;
        if host.contains(['/', '\\']) || host.len() <= 1 {
            return None;
        }
        scp = format!("ssh://{host}/{}", path.trim_start_matches('/'));
        &scp
    };
    let url = url::Url::parse(address).ok()?;
    if !matches!(url.scheme(), "https" | "http" | "ssh" | "git") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let path = url.path().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    let port = url
        .port()
        .filter(|port| !matches!((url.scheme(), *port), ("ssh", 22) | ("git", 9418)));
    Some(match port {
        Some(port) => format!("{host}:{port}/{path}"),
        None => format!("{host}/{path}"),
    })
}

pub async fn project_identity(checkout: &Path) -> Option<ProjectIdentity> {
    let checkout = tokio::fs::canonicalize(checkout).await.ok()?;
    let out = git(&checkout, &["rev-parse", "--show-toplevel"])
        .await
        .ok()?;
    if !out.success() {
        return None;
    }
    let root = tokio::fs::canonicalize(out.stdout_lossy().trim_end_matches(['\r', '\n']))
        .await
        .ok()?;
    let relative_path = checkout
        .strip_prefix(&root)
        .ok()?
        .iter()
        .map(|part| part.to_str())
        .collect::<Option<Vec<_>>>()?
        .join("/");
    let origin = git(&root, &["remote", "get-url", "origin"])
        .await
        .ok()
        .filter(|out| out.success())
        .map(|out| out.stdout_lossy().trim_end_matches(['\r', '\n']).to_owned());
    if let Some(label) = origin.as_deref().and_then(network_origin) {
        return Some(ProjectIdentity {
            repository: format!("remote:{label}"),
            label,
            relative_path,
        });
    }
    let mut common = common_git_dir(&root).await.ok()?;
    if let Some(remote) = origin {
        let local = if remote.starts_with("file://") {
            url::Url::parse(&remote)
                .ok()
                .and_then(|url| url.to_file_path().ok())
        } else if !remote.contains("://") && !remote.contains("::") {
            Some(root.join(remote))
        } else {
            None
        };
        if let Some(local) = local {
            if let Ok(dir) = common_git_dir(&local).await {
                common = dir;
            }
        }
    }
    let common = tokio::fs::canonicalize(common).await.ok()?;
    let repo = if common.file_name()? == ".git" {
        common.parent()?
    } else {
        &common
    };
    Some(ProjectIdentity {
        repository: format!("local:{}", common.to_str()?),
        label: repo.file_name()?.to_str()?.to_owned(),
        relative_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn project_identity_matches_worktrees_and_local_clones_without_an_origin_on_the_source() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let worktree = temp.path().join("topic");
        let clone = temp.path().join("copy");
        tokio::fs::create_dir(&main).await.unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Tester",
                "-c",
                "user.email=test@example.org",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-qm",
                "Initial",
            ],
            vec!["worktree", "add", "--detach", worktree.to_str().unwrap()],
            vec!["clone", "--local", ".", clone.to_str().unwrap()],
        ] {
            assert!(git(&main, &args).await.unwrap().success());
        }
        let source = project_identity(&main).await.unwrap();
        for checkout in [worktree, clone] {
            tokio::fs::create_dir(checkout.join("docs")).await.unwrap();
            let copy = project_identity(&checkout.join("docs")).await.unwrap();
            assert_eq!(copy.repository, source.repository);
            assert_eq!(copy.relative_path, "docs");
        }
        assert!(project_identity(temp.path()).await.is_none());
    }

    #[test]
    fn origin_keys_handle_credentials_default_ports_and_distinct_servers() {
        let https =
            network_origin("https://name:secret@EXAMPLE.org/group/app.git?q=secret").unwrap();
        assert_eq!(https, "example.org/group/app");
        assert_eq!(
            network_origin("ssh://user@example.org:22/group/app/").unwrap(),
            https
        );
        assert_eq!(
            network_origin("git@example.org:group/app.git").unwrap(),
            https
        );
        assert_ne!(
            network_origin("ssh://git@example.org:2222/group/app").unwrap(),
            https
        );
        assert!(network_origin("/srv/host/repo.git").is_none());
    }
}
