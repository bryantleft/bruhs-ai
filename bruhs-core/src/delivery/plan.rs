use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::{
    delivery::*,
    logic::{git::is_valid_branch, run_place::WORKTREE_BRANCH_PREFIX},
};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

fn command(program: &str, args: &[&str], stdin: Option<String>) -> Command {
    Command {
        program: program.into(),
        args: args.iter().map(|argument| (*argument).to_owned()).collect(),
        stdin,
    }
}

fn required(value: &str, name: &str) -> DeliveryResult<()> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(DeliveryError::Invalid(format!(
            "{name} must contain text without NUL characters."
        )));
    }
    Ok(())
}

fn branch(value: &str) -> DeliveryResult<()> {
    if !is_valid_branch(value) {
        return Err(DeliveryError::Invalid(format!(
            "Invalid branch name: {value}"
        )));
    }
    Ok(())
}

pub(super) fn relative_file(value: &str) -> DeliveryResult<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\0')
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(DeliveryError::Invalid("Choose a file relative to the repository, without parent or current-directory components.".into()));
    }
    if path.components().any(|part| part.as_os_str() == ".git") {
        return Err(DeliveryError::Invalid(
            "Repository internals cannot be selected for delivery.".into(),
        ));
    }
    Ok(())
}

fn oid(value: &str) -> DeliveryResult<()> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DeliveryError::Invalid(
            "Reload the pull request to obtain its full head commit ID.".into(),
        ));
    }
    Ok(())
}

pub(super) async fn prepare(
    service: &DeliveryService,
    request: PrepareRequest,
) -> DeliveryResult<PreparedAction> {
    if request.timeout_seconds == 0 {
        return Err(DeliveryError::Invalid(
            "Choose a positive execution timeout in seconds.".into(),
        ));
    }
    if std::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(request.timeout_seconds))
        .is_none()
    {
        return Err(DeliveryError::Invalid(
            "The execution timeout exceeds this host's timer range. Choose a smaller budget."
                .into(),
        ));
    }
    let path = tokio::fs::canonicalize(&request.path)
        .await
        .map_err(|error| {
            DeliveryError::Invalid(format!("The repository path is unavailable: {error}"))
        })?;
    let (head, current_branch) = service.repository_state(&path).await?;
    let repository = if matches!(
        &request.action,
        DeliveryAction::Lifecycle { .. } | DeliveryAction::StartTrackerIssue { .. }
    ) {
        None
    } else {
        service.forge_repository(&path, request.forge).await?
    };
    let forge = repository
        .as_ref()
        .map_or(Forge::GitHub, |repository| repository.forge);
    let id = uuid::Uuid::new_v4().to_string();
    let mut workspace_path = None;
    let mut issue = None;
    let mut fingerprint = None;
    let mut stack = None;
    let mut native_stack = None;
    let commands = match &request.action {
        DeliveryAction::CreatePull {
            title,
            body,
            base,
            head,
            draft,
        } => create_pull(title, body, base, head, *draft)?,
        DeliveryAction::CommitPushPull {
            files,
            message,
            title,
            body,
            base,
            head,
            draft,
        } => {
            if files.is_empty() {
                return Err(DeliveryError::Invalid(
                    "Select the exact files to commit.".into(),
                ));
            }
            if *head != current_branch {
                return Err(DeliveryError::Invalid(
                    "Commit/push must use this workspace's current branch as its head.".into(),
                ));
            }
            required(message, "Commit message")?;
            for file in files {
                relative_file(file)?;
                let metadata = match tokio::fs::symlink_metadata(path.join(file)).await {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        service.read(&path, "git", &["ls-files", "--error-unmatch", "--", file]).await
                            .map_err(|_| DeliveryError::Invalid(format!("Selected file {file} does not exist and is not a tracked deletion.")))?;
                        continue;
                    }
                    Err(error) => {
                        return Err(DeliveryError::Invalid(format!(
                            "Selected file {file} is unavailable: {error}"
                        )));
                    }
                };
                if !metadata.file_type().is_file() {
                    return Err(DeliveryError::Invalid(format!(
                        "Select individual regular files; {file} is a directory or symlink."
                    )));
                }
                let resolved = tokio::fs::canonicalize(path.join(file))
                    .await
                    .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
                if !resolved.starts_with(&path) {
                    return Err(DeliveryError::Invalid(format!(
                        "Selected file {file} resolves outside the repository."
                    )));
                }
            }
            fingerprint = Some(service.fingerprint(&path, files).await?);
            let mut add = command("git", &["add", "--"], None);
            add.args.extend(files.iter().cloned());
            let mut commit = command(
                "git",
                &["commit", "--only", "--message", message, "--"],
                None,
            );
            commit.args.extend(files.iter().cloned());
            let mut commands = vec![
                add,
                commit,
                command("git", &["push", "--set-upstream", "origin", head], None),
            ];
            commands.extend(create_pull(title, body, base, head, *draft)?);
            commands
        }
        DeliveryAction::Review {
            number,
            head_oid,
            event,
            body,
            comments,
        } => {
            positive(*number)?;
            oid(head_oid)?;
            if *event == ReviewEvent::RequestChanges {
                required(body, "Change request")?;
            }
            for comment in comments {
                relative_file(&comment.path)?;
                required(&comment.body, "Line comment")?;
                if comment.line == 0 {
                    return Err(DeliveryError::Invalid(
                        "Line comments use positive line numbers.".into(),
                    ));
                }
            }
            let event = match event {
                ReviewEvent::Comment => "COMMENT",
                ReviewEvent::Approve => "APPROVE",
                ReviewEvent::RequestChanges => "REQUEST_CHANGES",
            };
            let body = serde_json::json!({"commit_id":head_oid,"event":event,"body":body,"comments":comments});
            vec![command(
                "gh",
                &[
                    "api",
                    "--method",
                    "POST",
                    &format!("repos/{{owner}}/{{repo}}/pulls/{number}/reviews"),
                    "--input",
                    "-",
                ],
                Some(body.to_string()),
            )]
        }
        DeliveryAction::Comment { number, body } => {
            positive(*number)?;
            required(body, "Comment")?;
            vec![command(
                "gh",
                &["pr", "comment", &number.to_string(), "--body-file", "-"],
                Some(body.clone()),
            )]
        }
        DeliveryAction::RequestReviewers { number, reviewers } => {
            positive(*number)?;
            if reviewers.is_empty() {
                return Err(DeliveryError::Invalid(
                    "Enter at least one reviewer.".into(),
                ));
            }
            for reviewer in reviewers {
                required(reviewer, "Reviewer")?;
                if reviewer.starts_with('-') || reviewer.contains(',') {
                    return Err(DeliveryError::Invalid(
                        "Enter each reviewer or team as one name.".into(),
                    ));
                }
            }
            vec![command(
                "gh",
                &[
                    "pr",
                    "edit",
                    &number.to_string(),
                    "--add-reviewer",
                    &reviewers.join(","),
                ],
                None,
            )]
        }
        DeliveryAction::Merge {
            number,
            head_oid,
            method,
            auto,
        } => {
            positive(*number)?;
            oid(head_oid)?;
            let method = match method {
                MergeMethod::Merge => "--merge",
                MergeMethod::Squash => "--squash",
                MergeMethod::Rebase => "--rebase",
            };
            let mut specification = command(
                "gh",
                &[
                    "pr",
                    "merge",
                    &number.to_string(),
                    method,
                    "--match-head-commit",
                    head_oid,
                ],
                None,
            );
            if *auto {
                specification.args.push("--auto".into());
            }
            vec![specification]
        }
        DeliveryAction::RebaseStack { number } | DeliveryAction::MergeStack { number, .. } => {
            positive(*number)?;
            stack = Some(
                service
                    .prepare_stack(
                        &path,
                        *number,
                        matches!(&request.action, DeliveryAction::RebaseStack { .. }),
                        &id,
                        forge,
                    )
                    .await?,
            );
            Vec::new()
        }
        DeliveryAction::MergeNativeStack { number, .. } => {
            positive(*number)?;
            if forge != Forge::GitHub {
                return Err(DeliveryError::Invalid(
                    "Native stacks require a GitHub repository.".into(),
                ));
            }
            native_stack = Some(service.prepare_native_stack(&path, *number).await?);
            Vec::new()
        }
        DeliveryAction::Lifecycle { phase } => {
            let scripts = service.scripts(&path).await?;
            match phase {
                LifecyclePhase::Setup => scripts.setup,
                LifecyclePhase::Run => scripts.run,
                LifecyclePhase::Teardown => scripts.teardown,
            }
        }
        DeliveryAction::StartIssue {
            base,
            branch: new_branch,
            run_setup,
            ..
        }
        | DeliveryAction::StartTrackerIssue {
            base,
            branch: new_branch,
            run_setup,
            ..
        } => {
            branch(base)?;
            branch(new_branch)?;
            if !new_branch.starts_with(WORKTREE_BRANCH_PREFIX) {
                return Err(DeliveryError::Invalid(
                    "Issue workspaces use a branch beginning with bruhs/.".into(),
                ));
            }
            issue = Some(match &request.action {
                DeliveryAction::StartIssue { number, .. } => {
                    positive(*number)?;
                    service.issue_for(&path, *number, forge).await?
                }
                DeliveryAction::StartTrackerIssue {
                    provider,
                    reference,
                    ..
                } => service.tracker_issue(&path, *provider, reference).await?,
                _ => unreachable!("issue workspace arm matches only issue actions"),
            });
            let destination = crate::git::worktree_dest(&path, new_branch)
                .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
            if destination.exists() {
                return Err(DeliveryError::Conflict(
                    "This issue workspace already exists. Open it instead of creating another."
                        .into(),
                ));
            }
            workspace_path = Some(destination.to_string_lossy().into_owned());
            let mut commands = vec![command(
                "git",
                &[
                    "worktree",
                    "add",
                    "-b",
                    new_branch,
                    &destination.to_string_lossy(),
                    base,
                ],
                None,
            )];
            if *run_setup {
                commands.extend(service.scripts(&path).await?.setup);
            }
            commands
        }
    };
    let commands = if let Some(repository) = &repository {
        if forge == Forge::GitLab {
            service
                .gitlab_plan(&path, repository, &request.action, commands)
                .await?
        } else {
            commands
        }
    } else {
        commands
    };
    if commands.is_empty() && stack.is_none() && native_stack.is_none() {
        return Err(DeliveryError::Invalid(
            "No commands are configured for this lifecycle phase. Add them to .bruhs/config.json."
                .into(),
        ));
    }
    for specification in &commands {
        validate_command(specification)?;
    }
    Ok(PreparedAction {
        id,
        path: path.to_string_lossy().into_owned(),
        forge,
        repository,
        action: request.action,
        commands,
        expected_head: head,
        expected_branch: current_branch,
        worktree_fingerprint: fingerprint,
        timeout_seconds: request.timeout_seconds,
        created_at_ms: super::now(),
        workspace_path,
        issue,
        stack,
        native_stack,
    })
}

fn positive(number: u64) -> DeliveryResult<()> {
    if number == 0 {
        Err(DeliveryError::Invalid(
            "Choose a positive issue or pull request number.".into(),
        ))
    } else {
        Ok(())
    }
}

fn create_pull(
    title: &str,
    body: &str,
    base: &str,
    head: &str,
    draft: bool,
) -> DeliveryResult<Vec<Command>> {
    required(title, "Pull request title")?;
    branch(base)?;
    branch(head)?;
    if base == head {
        return Err(DeliveryError::Invalid(
            "The pull request head and base must be different branches.".into(),
        ));
    }
    let mut specification = command(
        "gh",
        &[
            "pr",
            "create",
            "--title",
            title,
            "--body-file",
            "-",
            "--base",
            base,
            "--head",
            head,
        ],
        Some(body.to_owned()),
    );
    if draft {
        specification.args.push("--draft".into());
    }
    Ok(vec![specification])
}

fn validate_command(command: &Command) -> DeliveryResult<()> {
    required(&command.program, "Command executable")?;
    if command.program.starts_with('-')
        || command.args.iter().any(|arg| arg.contains('\0'))
        || command
            .stdin
            .as_ref()
            .is_some_and(|stdin| stdin.contains('\0'))
    {
        return Err(DeliveryError::Invalid(
            "The command contains an invalid executable, argument, or input.".into(),
        ));
    }
    Ok(())
}

impl DeliveryService {
    pub async fn scripts(&self, path: &Path) -> DeliveryResult<ProjectScripts> {
        let file = path.join(".bruhs/config.json");
        match tokio::fs::read(&file).await {
            Ok(bytes) => {
                if bytes.len() > bruhs_models::reactive::MAX_PAYLOAD_BYTES {
                    return Err(DeliveryError::Invalid(
                        "Project scripts exceed the 64 KiB automation configuration budget.".into(),
                    ));
                }
                let scripts: ProjectScripts = serde_json::from_slice(&bytes).map_err(|error| {
                    DeliveryError::Invalid(format!("{} is invalid: {error}", file.display()))
                })?;
                for command in scripts
                    .setup
                    .iter()
                    .chain(&scripts.run)
                    .chain(&scripts.teardown)
                {
                    validate_command(command)?;
                }
                Ok(scripts)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ProjectScripts::default())
            }
            Err(error) => Err(DeliveryError::Invalid(format!(
                "Could not read {}: {error}",
                file.display()
            ))),
        }
    }

    pub(super) async fn fingerprint(
        &self,
        path: &Path,
        files: &[String],
    ) -> DeliveryResult<String> {
        let patch = self
            .read(path, "git", &["diff", "--binary", "HEAD"])
            .await?;
        let root = path.to_owned();
        let files = files.to_owned();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut hash = Sha256::new();
            hash.update(patch.as_bytes());
            for path in files {
                hash.update(path.as_bytes());
                let mut file = match std::fs::File::open(root.join(&path)) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        hash.update(b"\0tracked-deletion\0");
                        continue;
                    }
                    Err(error) => {
                        return Err(DeliveryError::Invalid(format!(
                            "Cannot fingerprint {path}: {error}"
                        )));
                    }
                };
                let mut buffer = [0; 8192];
                loop {
                    let count = file
                        .read(&mut buffer)
                        .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
                    if count == 0 {
                        break;
                    }
                    hash.update(&buffer[..count]);
                }
            }
            Ok(format!("{:x}", hash.finalize()))
        })
        .await
        .map_err(|error| DeliveryError::Storage(error.to_string()))?
    }
}
