use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::{
    delivery::*,
    logic::{git::is_valid_branch, run_place::WORKTREE_BRANCH_PREFIX},
};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

pub(super) fn requires_repository(action: &DeliveryAction) -> bool {
    !matches!(
        action,
        DeliveryAction::Lifecycle { .. }
            | DeliveryAction::StartTrackerIssue { .. }
            | DeliveryAction::Verify { .. }
    )
}

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
    if path
        .components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case(".git"))
    {
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
    let (head_oid, current_branch) = service.repository_state(&path).await?;
    let mut repository = if !requires_repository(&request.action) {
        None
    } else {
        Some(service.forge_repository(&path, request.forge).await?.ok_or_else(|| {
            DeliveryError::Invalid("Delivery requires remote.origin.url. Add the intended forge origin before preparing an action.".into())
        })?)
    };
    let forge = repository
        .as_ref()
        .map_or(Forge::GitHub, |repository| repository.forge);
    let id = uuid::Uuid::new_v4().to_string();
    let mut workspace_path = None;
    let mut issue = None;
    let mut fingerprint = None;
    let mut verification = None;
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
            verification_id,
            message,
            title,
            body,
            base,
            head,
            draft,
        } => {
            if *head != current_branch {
                return Err(DeliveryError::Invalid(
                    "Commit/push must use this workspace's current branch as its head.".into(),
                ));
            }
            required(message, "Commit message")?;
            validate_source_files(service, &path, files).await?;
            fingerprint = Some(service.fingerprint(&path, files).await?);
            if let Some(id) = verification_id {
                verification = Some(
                    service
                        .verification_for(&path, files, &head_oid, id)
                        .await?,
                );
            }
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
                command(
                    "git",
                    &[
                        "push",
                        &repository
                            .as_ref()
                            .ok_or_else(|| {
                                DeliveryError::Invalid(
                                    "A reviewed origin is required before push.".into(),
                                )
                            })?
                            .remote_url,
                        &format!("<checked-commit>:refs/heads/{head}"),
                    ],
                    None,
                ),
                command(
                    "git",
                    &[
                        "update-ref",
                        &format!("refs/remotes/origin/{head}"),
                        "<checked-commit>",
                    ],
                    None,
                ),
                command(
                    "git",
                    &["branch", &format!("--set-upstream-to=origin/{head}"), head],
                    None,
                ),
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
        DeliveryAction::Verify { files } => {
            validate_source_files(service, &path, files).await?;
            fingerprint = Some(service.payload_fingerprint(&path, files).await?);
            workspace_path = Some(
                service
                    .verification_path(&id)
                    .await?
                    .to_string_lossy()
                    .into_owned(),
            );
            let commands = service.scripts(&path).await?.check;
            if commands.is_empty() {
                return Err(DeliveryError::Invalid(
                    "No independent checks are configured. Add check commands to .bruhs/config.json, then prepare and review them.".into(),
                ));
            }
            commands
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
    if repository.is_none() && commands.iter().any(|command| command.program == "gh") {
        repository = Some(service.forge_repository(&path, Forge::GitHub).await?.ok_or_else(|| {
            DeliveryError::Invalid("A GitHub project command requires remote.origin.url. Add the intended origin and prepare the action again.".into())
        })?);
    }
    let mut commands = if let Some(repository) = &repository {
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
    if let (
        DeliveryAction::Merge {
            number,
            head_oid,
            method,
            ..
        },
        Some(repo),
        [command],
    ) = (&request.action, &repository, commands.as_mut_slice())
    {
        if *method != MergeMethod::Rebase
            && service.preferences(Some(&path)).await.remove_agent_credits
        {
            service
                .clean_merge_message(&path, repo, *number, head_oid, *method, command)
                .await?;
        }
    }
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
        source_task: None,
        verification,
        action: request.action,
        commands,
        expected_head: head_oid,
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

pub(super) async fn validate_source_files(
    service: &DeliveryService,
    path: &Path,
    files: &[String],
) -> DeliveryResult<()> {
    if files.is_empty() {
        return Err(DeliveryError::Invalid(
            "Select the exact files to check or commit.".into(),
        ));
    }
    let mut unique = std::collections::HashSet::new();
    for file in files {
        if !unique.insert(file) {
            return Err(DeliveryError::Invalid(
                "Select each source file only once.".into(),
            ));
        }
    }
    for file in files {
        relative_file(file)?;
        let metadata = match tokio::fs::symlink_metadata(path.join(file)).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                service
                    .read(path, "git", &["ls-files", "--error-unmatch", "--", file])
                    .await
                    .map_err(|_| {
                        DeliveryError::Invalid(format!(
                            "Selected file {file} does not exist and is not a tracked deletion."
                        ))
                    })?;
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
        if !resolved.starts_with(path) {
            return Err(DeliveryError::Invalid(format!(
                "Selected file {file} resolves outside the repository."
            )));
        }
    }
    Ok(())
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
        let bruhs_dir = path.join(".bruhs");
        let file = bruhs_dir.join("config.json");
        let file_for_read = file.clone();
        let limit = u64::try_from(bruhs_models::reactive::MAX_PAYLOAD_BYTES).unwrap_or(64 * 1024);
        let read = tokio::task::spawn_blocking(move || {
            match std::fs::symlink_metadata(&bruhs_dir) {
                Ok(meta) if meta.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(DeliveryError::Invalid(
                        "Project scripts path contains a link or non-directory.".into(),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(DeliveryError::Invalid(format!(
                        "Could not read {}: {error}",
                        file_for_read.display()
                    )));
                }
            }
            match std::fs::symlink_metadata(&file_for_read) {
                Ok(meta) if !meta.file_type().is_file() => {
                    return Err(DeliveryError::Invalid(
                        "Project scripts file must be a regular file, not a link or special file."
                            .into(),
                    ));
                }
                Ok(meta) if meta.len() > limit => {
                    return Err(DeliveryError::Invalid(
                        "Project scripts exceed the 64 KiB automation configuration budget.".into(),
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(DeliveryError::Invalid(format!(
                        "Could not read {}: {error}",
                        file_for_read.display()
                    )));
                }
            }
            match crate::workspace_trust::read_regular_file_bounded(&file_for_read, limit) {
                Ok(Some(bytes)) => Ok(Some(bytes)),
                Ok(None) => Err(DeliveryError::Invalid(
                    "Project scripts exceed the 64 KiB automation configuration budget or changed during read.".into(),
                )),
                Err(error) => Err(DeliveryError::Invalid(format!(
                    "Could not read {}: {error}",
                    file_for_read.display()
                ))),
            }
        })
        .await
        .map_err(|error| DeliveryError::Invalid(error.to_string()))??;
        let Some(bytes) = read else {
            return Ok(ProjectScripts::default());
        };
        let scripts: ProjectScripts = serde_json::from_slice(&bytes).map_err(|error| {
            DeliveryError::Invalid(format!("{} is invalid: {error}", file.display()))
        })?;
        for command in scripts
            .setup
            .iter()
            .chain(&scripts.run)
            .chain(&scripts.check)
            .chain(&scripts.teardown)
        {
            validate_command(command)?;
        }
        Ok(scripts)
    }

    pub(super) async fn fingerprint(
        &self,
        path: &Path,
        files: &[String],
    ) -> DeliveryResult<String> {
        self.fingerprint_scoped(path, files, false).await
    }

    pub(super) async fn payload_fingerprint(
        &self,
        path: &Path,
        files: &[String],
    ) -> DeliveryResult<String> {
        Ok(format!(
            "payload-v2:{}",
            self.fingerprint_scoped(path, files, true).await?
        ))
    }

    async fn fingerprint_scoped(
        &self,
        path: &Path,
        files: &[String],
        selected_only: bool,
    ) -> DeliveryResult<String> {
        let mut sorted = files.to_vec();
        if selected_only {
            sorted.sort();
        }
        // Payload identity is independent of staging. HEAD is a separate lease;
        // selected paths, contents, kinds and modes define its pending changes.
        let patch = if selected_only {
            String::new()
        } else {
            self.read(
                path,
                "git",
                &[
                    "-c",
                    "core.quotePath=true",
                    "diff",
                    "--binary",
                    "--full-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD",
                    "--",
                ],
            )
            .await?
        };
        let files = if selected_only {
            sorted.as_slice()
        } else {
            files
        };
        let root = path.to_owned();
        let files = files.to_owned();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;

            fn frame(hash: &mut Sha256, bytes: &[u8]) -> DeliveryResult<()> {
                let length = u64::try_from(bytes.len()).map_err(|_| {
                    DeliveryError::Invalid("Source review field exceeds its encoding range.".into())
                })?;
                hash.update(length.to_be_bytes());
                hash.update(bytes);
                Ok(())
            }

            let root = std::fs::canonicalize(root)
                .map_err(|error| DeliveryError::Invalid(format!("Cannot read the reviewed repository: {error}")))?;
            let mut hash = Sha256::new();
            hash.update(b"bruhs.delivery.source.v2\0");
            frame(&mut hash, patch.as_bytes())?;
            for relative in files {
                frame(&mut hash, relative.as_bytes())?;
                let path = root.join(&relative);
                let entry = match std::fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        hash.update([0_u8]);
                        continue;
                    }
                    Err(error) => return Err(DeliveryError::Invalid(format!("Cannot fingerprint {relative}: {error}"))),
                };
                if !entry.file_type().is_file() {
                    return Err(DeliveryError::Conflict(format!("Selected file {relative} is no longer a regular file. Prepare and review the action again.")));
                }
                let resolved = std::fs::canonicalize(&path)
                    .map_err(|error| DeliveryError::Invalid(format!("Cannot fingerprint {relative}: {error}")))?;
                if !resolved.starts_with(&root) {
                    return Err(DeliveryError::Conflict(format!("Selected file {relative} resolves outside the reviewed repository. Prepare the action again.")));
                }
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    // A replaced symlink or FIFO must not redirect or block a review.
                    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                }
                let mut file = options.open(&path)
                    .map_err(|error| DeliveryError::Invalid(format!("Cannot fingerprint {relative}: {error}")))?;
                let before = file.metadata()
                    .map_err(|error| DeliveryError::Invalid(format!("Cannot inspect {relative}: {error}")))?;
                if !before.is_file() {
                    return Err(DeliveryError::Conflict(format!("Selected file {relative} changed type during review. Prepare the action again.")));
                }
                hash.update([1_u8]);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    hash.update(before.permissions().mode().to_be_bytes());
                }
                #[cfg(not(unix))]
                hash.update([u8::from(before.permissions().readonly())]);
                let expected = before.len();
                hash.update(expected.to_be_bytes());
                let mut observed = 0_u64;
                let mut buffer = [0; 8192];
                loop {
                    let count = file.read(&mut buffer)
                        .map_err(|error| DeliveryError::Invalid(format!("Cannot read {relative}: {error}")))?;
                    if count == 0 { break; }
                    observed = observed.checked_add(u64::try_from(count).map_err(|_| {
                        DeliveryError::Invalid("Source review file exceeds its encoding range.".into())
                    })?).ok_or_else(|| DeliveryError::Invalid("Source review file exceeds its encoding range.".into()))?;
                    if observed > expected {
                        return Err(DeliveryError::Conflict(format!("Selected file {relative} changed while being read. Prepare the action again.")));
                    }
                    hash.update(&buffer[..count]);
                }
                if observed != expected {
                    return Err(DeliveryError::Conflict(format!("Selected file {relative} changed while being read. Prepare the action again.")));
                }
            }
            Ok(format!("v2:{:x}", hash.finalize()))
        })
        .await
        .map_err(|error| DeliveryError::Storage(error.to_string()))?
    }
}
