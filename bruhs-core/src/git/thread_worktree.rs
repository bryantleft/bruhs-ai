//! A new conversation owns its checkout independently of the branch's display name.
use super::*;
use bruhs_models::logic::run_place::{
    WorktreePreferences,
    branch_naming::{Mode, Naming},
};
use std::collections::HashSet;

pub struct ThreadCheckout {
    pub path: PathBuf,
    pub branch: String,
    pub naming_notice: Option<String>,
}

pub async fn prepare_thread_worktree<'a>(
    repo: &Path,
    base: &str,
    identity: &str,
    prefs: WorktreePreferences,
    naming: &Naming,
    prompt: impl Into<crate::thread_title::FirstMessage<'a>>,
    model: &str,
) -> Result<ThreadCheckout, GitError> {
    let prompt = prompt.into();
    let dest = worktree_dest(repo, identity)?;
    let root = dest
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GitError::Failed("Worktree folder unavailable.".into()))?
        .to_owned();
    // Model calls may wait on the provider. Hold the shared Git lock only
    // while inspecting and preparing the checkout, then publish its final ref.
    let suggestion = if !dest.exists() && naming.mode != Mode::ThreadId {
        let writer = crate::delivery::DeliveryService::saved_writer_model(repo).await;
        use bruhs_models::delivery::WriterModel;
        let (model, options) = match &writer {
            Ok(WriterModel::Separate { model, options }) if crate::text_generators::split_selection(model).is_none() || crate::text_generators::active(model).await.map_err(GitError::Failed)? => (model.as_str(), Some(options)),
            _ => (model, None),
        };
        let suggestion = match writer.as_ref() {
            Err(error) => Err(error.to_string()),
            Ok(_) if model == "local" => {
                let mut text = prompt.text.to_owned();
                for file in prompt.files {
                    text.push_str("\nAttached: ");
                    text.push_str(&file.name);
                }
                local_suggestion(&text, naming.mode)
            }
            Ok(_) => crate::thread_title::branch_name_with_options(repo, model, options, prompt, naming)
                .await
                .map_err(|e| e.to_string()),
        };
        Some(suggestion.and_then(|raw| naming.branch(&raw)))
    } else {
        None
    };
    let _lock = crate::worktrees::lock(&root)
        .await
        .map_err(GitError::Failed)?;
    if dest.exists() {
        let branch = owned_branch(repo, &dest, identity).await?;
        initialize_worktree_submodules(repo, &dest, prefs.submodules).await?;
        return Ok(ThreadCheckout {
            path: dest,
            branch,
            naming_notice: None,
        });
    }
    let short = identity.rsplit('/').next().unwrap_or(identity);
    let preferred = if naming.mode == Mode::Static {
        naming.branch(short).map_err(GitError::Failed)?
    } else {
        identity.to_owned()
    };
    let current = available_name(repo, &preferred).await?;
    add_worktree_inner(repo, base, &dest, &current, prefs, Some(identity)).await?;
    let Some(suggestion) = suggestion else {
        return Ok(ThreadCheckout {
            path: dest,
            branch: current,
            naming_notice: None,
        });
    };
    let renamed = match suggestion {
        Ok(name) => rename_generated(&dest, &current, &name, naming.mode)
            .await
            .map_err(|e| e.to_string()),
        Err(reason) => Err(reason),
    };
    match renamed {
        Ok(branch) => Ok(ThreadCheckout {
            path: dest,
            branch,
            naming_notice: None,
        }),
        Err(reason) => Ok(ThreadCheckout {
            path: dest,
            naming_notice: Some(format!(
                "Using branch {current}. Branch naming failed: {reason}"
            )),
            branch: current,
        }),
    }
}

fn local_suggestion(prompt: &str, mode: Mode) -> Result<String, String> {
    let text = prompt
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    match mode {
        Mode::Static => Ok(text),
        Mode::Semantic => {
            let intent = text
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let prefix = match intent.as_str() {
                "fix" | "repair" | "resolve" => "fix",
                "document" | "docs" => "docs",
                "refactor" => "refactor",
                "add" | "implement" | "build" => "feat",
                _ => "chore",
            };
            Ok(format!("{prefix}/{text}"))
        }
        Mode::Custom | Mode::ThreadId => {
            Err("Select a text generation model to use custom branch naming instructions.".into())
        }
    }
}

pub(super) async fn write_owner(dest: &Path, branch: &str, identity: &str) -> Result<(), GitError> {
    let key = format!("branch.{branch}.bruhs-thread");
    let out = git(dest, &["config", "--local", &key, identity]).await?;
    if !out.success() {
        return Err(out.failure());
    }
    Ok(())
}

async fn owned_branch(repo: &Path, dest: &Path, identity: &str) -> Result<String, GitError> {
    let in_use = || GitError::Failed("Worktree directory is already in use.".into());
    let branch = worktree_branch(repo, dest)
        .await?
        .flatten()
        .ok_or_else(in_use)?;
    if !dest.join(".git").is_file()
        || resolve(repo) == resolve(dest)
        || resolve(&common_git_dir(repo).await?) != resolve(&common_git_dir(dest).await?)
        || !current_branch_matches(dest, &branch).await?
    {
        return Err(in_use());
    }
    let key = format!("branch.{branch}.bruhs-thread");
    let owner = git(dest, &["config", "--local", "--get", &key]).await?;
    if owner.success() {
        return (owner.stdout_lossy().trim_end() == identity)
            .then_some(branch)
            .ok_or_else(in_use);
    }
    // Before explicit ownership, bruhs used the thread ID in the branch.
    if owner.status.and_then(|s| s.code()) == Some(1)
        && (branch == identity
            || branch.ends_with(&format!(
                "-{}",
                identity.rsplit('/').next().unwrap_or(identity)
            )))
    {
        return Ok(branch);
    }
    Err(in_use())
}

async fn available_name(repo: &Path, requested: &str) -> Result<String, GitError> {
    let out = git(
        repo,
        &["for-each-ref", "--format=%(refname:strip=2)", "refs/heads/"],
    )
    .await?;
    if !out.success() {
        return Err(out.failure());
    }
    let refs: HashSet<_> = out.stdout_lossy().lines().map(str::to_lowercase).collect();
    let has_parent = requested
        .match_indices('/')
        .any(|(i, _)| refs.contains(&requested[..i].to_lowercase()));
    let base = if has_parent {
        requested.replace('/', "-")
    } else {
        requested.to_owned()
    };
    let mut occupied = refs.clone();
    for name in &refs {
        occupied.extend(name.match_indices('/').map(|(i, _)| name[..i].to_owned()));
    }
    (0..=100)
        .map(|i| {
            if i == 0 {
                base.clone()
            } else {
                format!("{base}-{i}")
            }
        })
        .find(|candidate| !occupied.contains(&candidate.to_lowercase()))
        .ok_or_else(|| GitError::Failed(format!("No available branch name for {base}.")))
}

async fn rename_generated(
    dest: &Path,
    current: &str,
    suggested: &str,
    mode: Mode,
) -> Result<String, GitError> {
    if suggested == current {
        return Ok(current.to_owned());
    }
    let name = if mode == Mode::Custom {
        suggested.to_owned()
    } else {
        available_name(dest, suggested).await?
    };
    // Git moves the owner setting with the ref, and refuses to replace an existing branch.
    let out = git(dest, &["branch", "-m", "--", current, &name]).await?;
    if !out.success() {
        return Err(out.failure());
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::Repo;

    #[test]
    fn local_text_mode_names_branches_without_a_provider() {
        assert_eq!(
            local_suggestion("Fix search", Mode::Static).unwrap(),
            "Fix search"
        );
        assert_eq!(
            local_suggestion("Fix search", Mode::Semantic).unwrap(),
            "fix/Fix search"
        );
        assert!(local_suggestion("Fix search", Mode::Custom).is_err());
    }

    #[tokio::test]
    async fn generated_branches_disambiguate_without_replacing_refs_and_keep_their_owner() {
        let repo = Repo::new();
        let area = tempfile::tempdir().unwrap();
        let path = area.path().join("bruhs-abc");
        add_worktree_inner(
            repo.path(),
            "main",
            &path,
            "bruhs/abc",
            Default::default(),
            Some("bruhs/abc"),
        )
        .await
        .unwrap();
        repo.git(&["branch", "team/Add-Search"]);
        let branch = rename_generated(&path, "bruhs/abc", "team/add-search", Mode::Static)
            .await
            .unwrap();
        assert_eq!(branch, "team/add-search-1");
        assert_eq!(
            owned_branch(repo.path(), &path, "bruhs/abc").await.unwrap(),
            branch
        );
        assert!(owned_branch(repo.path(), &path, "bruhs/def").await.is_err());
        repo.git(&["branch", "fix"]);
        assert_eq!(
            available_name(&path, "fix/broken-tab").await.unwrap(),
            "fix-broken-tab"
        );
        assert_eq!(available_name(&path, "team").await.unwrap(), "team-1");
        assert!(
            rename_generated(&path, &branch, "main", Mode::Custom)
                .await
                .is_err()
        );
        let exact = rename_generated(&path, &branch, "Issues/BR-42/修正", Mode::Custom)
            .await
            .unwrap();
        assert_eq!(exact, "Issues/BR-42/修正");
        assert_eq!(
            owned_branch(repo.path(), &path, "bruhs/abc").await.unwrap(),
            exact
        );
        assert!(branch_exists(repo.path(), "team/Add-Search").await.unwrap());
        let legacy = area.path().join("legacy");
        add_worktree(repo.path(), "main", &legacy, "team/old-abc")
            .await
            .unwrap();
        assert!(
            owned_branch(repo.path(), &legacy, "bruhs/abc")
                .await
                .is_ok()
        );
        assert!(
            owned_branch(repo.path(), &legacy, "bruhs/def")
                .await
                .is_err()
        );
    }
}
