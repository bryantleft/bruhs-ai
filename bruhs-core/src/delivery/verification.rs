//! Owner-reviewed deterministic checks of HEAD plus the selected delivery payload.

use super::{plan, DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::{
    ActionStatus, Command, DeliveryAction, PreparedAction, SourceVerification,
};
use std::path::{Path, PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn same_files(left: &[String], right: &[String]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

impl DeliveryService {
    pub(super) async fn verification_path(&self, id: &str) -> DeliveryResult<PathBuf> {
        uuid::Uuid::parse_str(id).map_err(|_| {
            DeliveryError::Invalid(
                "The check receipt has an invalid identity. Prepare checks again.".into(),
            )
        })?;
        let root = tokio::fs::canonicalize(self.root.as_path())
            .await
            .map_err(|error| {
                DeliveryError::Storage(format!("Cannot locate private check storage: {error}"))
            })?;
        Ok(root.join("verification").join(id))
    }

    pub(super) async fn verification_for(
        &self,
        path: &Path,
        files: &[String],
        head: &str,
        id: &str,
    ) -> DeliveryResult<SourceVerification> {
        let receipt = self
            .state
            .lock()
            .await
            .actions
            .iter()
            .find(|row| row.prepared.id == id)
            .cloned()
            .ok_or_else(|| {
                DeliveryError::NotFound(
                    "The independent check receipt is unavailable. Prepare and run checks again."
                        .into(),
                )
            })?;
        let DeliveryAction::Verify {
            files: checked_files,
        } = &receipt.prepared.action
        else {
            return Err(DeliveryError::Invalid("Choose an independent check receipt, not an agent report or another delivery action.".into()));
        };
        if receipt.status != ActionStatus::Completed
            || receipt.error.is_some()
            || receipt.prepared.commands.is_empty()
            || receipt.completed_commands != receipt.prepared.commands.len()
        {
            return Err(DeliveryError::Conflict("Independent checks did not all finish successfully. Inspect the receipt and run a new reviewed check before delivery.".into()));
        }
        if receipt.prepared.path != path.to_string_lossy()
            || receipt.prepared.expected_head != head
            || !same_files(files, checked_files)
        {
            return Err(DeliveryError::Conflict("These checks belong to different source files, workspace or HEAD. Prepare and run checks for this delivery.".into()));
        }
        let fingerprint = self.payload_fingerprint(path, files).await?;
        if receipt.prepared.worktree_fingerprint.as_ref() != Some(&fingerprint) {
            return Err(DeliveryError::Conflict("Source changed after the independent checks. Prepare and run checks again before delivery.".into()));
        }
        let checked_tree=receipt.checked_tree.filter(|tree| valid_oid(tree)).ok_or_else(|| DeliveryError::Conflict(
            "This check receipt has no immutable tree evidence. Prepare and run checks again.".into()))?;
        Ok(SourceVerification {
            receipt_id: id.to_owned(),
            source_head: head.to_owned(),
            source_fingerprint: fingerprint,
            checked_tree,
            files: checked_files.clone(),
            commands: receipt.prepared.commands.clone(),
            completed_at_ms: receipt.updated_at_ms,
        })
    }

    pub(super) async fn validate_delivery_verification(
        &self,
        prepared: &PreparedAction,
    ) -> DeliveryResult<()> {
        let DeliveryAction::CommitPushPull {
            files,
            verification_id,
            ..
        } = &prepared.action
        else {
            return Ok(());
        };
        let proof = prepared.verification.as_ref().ok_or_else(|| DeliveryError::Conflict(
            "This delivery has no independent check evidence. Prepare and run checks, then prepare delivery with their receipt.".into()))?;
        if verification_id.as_ref() != Some(&proof.receipt_id)
            || proof.source_head != prepared.expected_head
            || !same_files(files, &proof.files)
            || proof.commands.is_empty()
            || !valid_oid(&proof.checked_tree)
        {
            return Err(DeliveryError::Conflict("The saved check evidence does not match this delivery. Prepare and review it again.".into()));
        }
        if self
            .payload_fingerprint(Path::new(&prepared.path), files)
            .await?
            != proof.source_fingerprint
        {
            return Err(DeliveryError::Conflict("Source changed after the independent checks. Run checks and review delivery again.".into()));
        }
        Ok(())
    }

    /// Resolve only the reviewed commit placeholder. Publish the checked object
    /// to the frozen remote URL, independent of later branch or origin changes.
    pub(super) async fn resolve_checked_command(
        &self,
        prepared: &PreparedAction,
        command: &Command,
    ) -> DeliveryResult<Option<Command>> {
        let DeliveryAction::CommitPushPull { head, message, .. } = &prepared.action else {
            return Ok(None);
        };
        if command.program != "git" {
            return Ok(None);
        }
        if command.args.first().map(String::as_str) == Some("update-ref") {
            let expected = vec![
                "update-ref".to_owned(),
                format!("refs/remotes/origin/{head}"),
                "<checked-commit>".to_owned(),
            ];
            if command.args != expected {
                return Err(DeliveryError::Conflict("The reviewed tracking update is not bound to a checked commit. Prepare delivery again.".into()));
            }
            let commit=self.state.lock().await.actions.iter().find(|row|row.prepared.id==prepared.id).and_then(|row|row.checked_commit.clone())
                .filter(|commit|valid_oid(commit)).ok_or_else(||DeliveryError::Conflict("No checked commit was retained before the tracking update. Inspect the receipt; do not replay the push.".into()))?;
            return Ok(Some(Command {
                program: "git".into(),
                args: vec![
                    "update-ref".into(),
                    format!("refs/remotes/origin/{head}"),
                    commit,
                ],
                stdin: None,
            }));
        }
        if command.args.first().map(String::as_str) != Some("push") {
            return Ok(None);
        }
        let repository = prepared.repository.as_ref().ok_or_else(|| {
            DeliveryError::Conflict(
                "The push has no reviewed remote. Prepare delivery again.".into(),
            )
        })?;
        let expected = vec![
            "push".to_owned(),
            repository.remote_url.clone(),
            format!("<checked-commit>:refs/heads/{head}"),
        ];
        if command.args != expected {
            return Err(DeliveryError::Conflict(
                "The reviewed push has an older or unexpected source form. Prepare delivery again."
                    .into(),
            ));
        }
        let proof = prepared.verification.as_ref().ok_or_else(|| {
            DeliveryError::Conflict("Independent check evidence is missing.".into())
        })?;
        let path = Path::new(&prepared.path);
        let commit = self
            .read(path, "git", &["rev-parse", "--verify", "HEAD"])
            .await?
            .trim()
            .to_owned();
        if !valid_oid(&commit) {
            return Err(DeliveryError::Command(
                "Git returned an invalid commit identity before push.".into(),
            ));
        }
        let tree = self
            .read(path, "git", &["rev-parse", &format!("{commit}^{{tree}}")])
            .await?;
        let parent = self
            .read(path, "git", &["rev-parse", &format!("{commit}^")])
            .await?;
        let committed_message = self
            .read(path, "git", &["show", "-s", "--format=%B", &commit])
            .await?;
        if tree.trim() != proof.checked_tree
            || parent.trim() != prepared.expected_head
            || committed_message.trim_end() != message.trim_end()
        {
            return Err(DeliveryError::Conflict("The created commit differs from the checked tree, reviewed parent or message. Nothing was pushed. Inspect the local commit and run checks again.".into()));
        }
        self.mutate(|state| {
            let receipt = state
                .actions
                .iter_mut()
                .find(|row| row.prepared.id == prepared.id)
                .ok_or_else(|| {
                    DeliveryError::NotFound(
                        "The running delivery receipt is unavailable before push.".into(),
                    )
                })?;
            if receipt.status != ActionStatus::Running {
                return Err(DeliveryError::Conflict(
                    "Delivery stopped before push. No checked commit was submitted.".into(),
                ));
            }
            receipt.checked_commit = Some(commit.clone());
            Ok(())
        })
        .await?;
        Ok(Some(Command {
            program: "git".into(),
            args: vec![
                "push".into(),
                repository.remote_url.clone(),
                format!("{commit}:refs/heads/{head}"),
            ],
            stdin: None,
        }))
    }

    pub(super) async fn run_verification(
        &self,
        prepared: &PreparedAction,
        mut cancel: watch::Receiver<bool>,
    ) -> DeliveryResult<()> {
        let DeliveryAction::Verify { files } = &prepared.action else {
            return Err(DeliveryError::Invalid(
                "This action is not an independent check.".into(),
            ));
        };
        let checkout = self.verification_path(&prepared.id).await?;
        if prepared.workspace_path.as_deref() != checkout.to_str() {
            return Err(DeliveryError::Conflict(
                "The private check workspace differs from its review. Prepare checks again.".into(),
            ));
        }
        let parent = checkout
            .parent()
            .ok_or_else(|| DeliveryError::Storage("Private check storage has no parent.".into()))?;
        match tokio::fs::create_dir(parent).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = tokio::fs::symlink_metadata(parent)
                    .await
                    .map_err(storage_error)?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(DeliveryError::Storage(
                        "Private check storage was replaced. Restore it before running checks."
                            .into(),
                    ));
                }
            }
            Err(error) => return Err(storage_error(error)),
        }
        // Only a directory created here is ever removed. A collision preserves
        // the existing path and fails instead of adopting or erasing it.
        tokio::fs::create_dir(&checkout).await.map_err(|error| {
            DeliveryError::Storage(format!(
                "Cannot create private check workspace {}: {error}",
                checkout.display()
            ))
        })?;
        let work = async {
            let source = Path::new(&prepared.path);
            if self.payload_fingerprint(source, files).await?.as_str()
                != prepared.worktree_fingerprint.as_deref().unwrap_or("")
            {
                return Err(DeliveryError::Conflict(
                    "Source changed after check review. Prepare and review the checks again."
                        .into(),
                ));
            }
            let clone = Command {
                program: "git".into(),
                args: [
                    "-c",
                    "core.hooksPath=/dev/null",
                    "clone",
                    "--shared",
                    "--no-checkout",
                    "--local",
                    "--no-tags",
                    "--",
                    &prepared.path,
                    &checkout.to_string_lossy(),
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                stdin: None,
            };
            self.run_command(source, &clone, &mut cancel, None).await?;
            let checkout_head = Command {
                program: "git".into(),
                args: vec![
                    "-c".into(),
                    "core.hooksPath=/dev/null".into(),
                    "checkout".into(),
                    "--detach".into(),
                    prepared.expected_head.clone(),
                ],
                stdin: None,
            };
            self.run_command(&checkout, &checkout_head, &mut cancel, None)
                .await?;
            // The clone's implicit origin must never offer a route back into
            // the owner's source repository from a check command.
            self.run_command(
                &checkout,
                &Command {
                    program: "git".into(),
                    args: vec!["remote".into(), "remove".into(), "origin".into()],
                    stdin: None,
                },
                &mut cancel,
                None,
            )
            .await?;
            copy_payload(source, &checkout, files).await?;
            if self.payload_fingerprint(&checkout, files).await?.as_str()
                != prepared.worktree_fingerprint.as_deref().unwrap_or("")
            {
                return Err(DeliveryError::Conflict("The private check snapshot differs from the reviewed payload. Prepare checks again.".into()));
            }
            let mut stage = Command {
                program: "git".into(),
                args: vec![
                    "-c".into(),
                    "core.hooksPath=/dev/null".into(),
                    "add".into(),
                    "--all".into(),
                    "--".into(),
                ],
                stdin: None,
            };
            stage.args.extend(files.iter().cloned());
            self.run_command(&checkout, &stage, &mut cancel, None)
                .await?;
            let tree = self
                .read(&checkout, "git", &["write-tree"])
                .await?
                .trim()
                .to_owned();
            if !valid_oid(&tree) {
                return Err(DeliveryError::Command(
                    "Git returned an invalid check tree identity.".into(),
                ));
            }
            self.mutate(|state| {
                let receipt = state
                    .actions
                    .iter_mut()
                    .find(|row| row.prepared.id == prepared.id)
                    .ok_or_else(|| {
                        DeliveryError::NotFound(
                            "The check receipt disappeared while capturing its tree.".into(),
                        )
                    })?;
                receipt.checked_tree = Some(tree.clone());
                Ok(())
            })
            .await?;
            let snapshot = self.fingerprint(&checkout, files).await?;
            self.run_commands(prepared, &mut cancel, Some(&checkout))
                .await?;
            let head = self.read(&checkout, "git", &["rev-parse", "HEAD"]).await?;
            if head.trim() != prepared.expected_head
                || self.fingerprint(&checkout, files).await? != snapshot
                || self.read(&checkout, "git", &["write-tree"]).await?.trim() != tree
            {
                return Err(DeliveryError::Conflict("Check commands changed the reviewed source snapshot. Their exit status is not accepted as a source check; inspect the receipt and prepare checks again.".into()));
            }
            if self.payload_fingerprint(source, files).await?.as_str()
                != prepared.worktree_fingerprint.as_deref().unwrap_or("")
            {
                return Err(DeliveryError::Conflict("Source changed while checks ran. The result is outdated; prepare and run checks again.".into()));
            }
            Ok(())
        };
        let result = match tokio::time::timeout(std::time::Duration::from_secs(prepared.timeout_seconds), work).await {
            Ok(result) => result,
            Err(_) => Err(DeliveryError::Command(format!("Independent checks exceeded the reviewed {} second budget. The active process was stopped. Inspect this receipt and prepare a new check; nothing was published.", prepared.timeout_seconds))),
        };
        if let Err(error) = tokio::fs::remove_dir_all(&checkout).await {
            return Err(DeliveryError::Storage(format!("Independent check result: {}. Private workspace cleanup failed at {}: {error}. Inspect and remove that owned workspace before retrying.", result.as_ref().err().map_or("commands finished".to_owned(), ToString::to_string), checkout.display())));
        }
        result
    }
}

fn storage_error(error: std::io::Error) -> DeliveryError {
    DeliveryError::Storage(format!("Private check workspace: {error}"))
}

async fn copy_payload(source: &Path, checkout: &Path, files: &[String]) -> DeliveryResult<()> {
    for relative in files {
        plan::relative_file(relative)?;
        let destination = checkout.join(relative);
        let mut parent = checkout.to_owned();
        let parts = Path::new(relative).components().collect::<Vec<_>>();
        for part in &parts[..parts.len().saturating_sub(1)] {
            parent.push(part.as_os_str());
            match tokio::fs::symlink_metadata(&parent).await {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {},
                Ok(_) => return Err(DeliveryError::Conflict("A selected path has a non-directory parent in HEAD. Select its structural change too or use a dedicated workspace.".into())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => tokio::fs::create_dir(&parent).await.map_err(storage_error)?,
                Err(error) => return Err(storage_error(error)),
            }
        }
        match tokio::fs::remove_file(&destination).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
        let path = source.join(relative);
        let metadata = match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
            Ok(_) => return Err(DeliveryError::Conflict("A selected source file changed type during snapshot creation. Prepare checks again.".into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(storage_error(error)),
        };
        let resolved = tokio::fs::canonicalize(&path)
            .await
            .map_err(storage_error)?;
        if !resolved.starts_with(source) {
            return Err(DeliveryError::Conflict(
                "A selected source file moved outside the workspace. Prepare checks again.".into(),
            ));
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let mut input = options.open(&path).await.map_err(storage_error)?;
        if !input.metadata().await.map_err(storage_error)?.is_file() {
            return Err(DeliveryError::Conflict(
                "Only regular files can enter the check snapshot.".into(),
            ));
        }
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .await
            .map_err(storage_error)?;
        let mut buffer = [0; 8192];
        loop {
            let length = input.read(&mut buffer).await.map_err(storage_error)?;
            if length == 0 {
                break;
            }
            output
                .write_all(&buffer[..length])
                .await
                .map_err(storage_error)?;
        }
        output.flush().await.map_err(storage_error)?;
        tokio::fs::set_permissions(&destination, metadata.permissions())
            .await
            .map_err(storage_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
