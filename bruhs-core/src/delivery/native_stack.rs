//! GitHub's native asynchronous merge has server-current-head authority, not a SHA lease.
//! Keep it behind a separate owner acknowledgement and never replay an uncertain submission.
use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::*;
use serde::Deserialize;
use std::{collections::HashSet, path::Path};
use tokio::sync::watch;
#[cfg(test)]
mod tests;

#[derive(Deserialize)]
struct NativeRef {
    #[serde(rename = "ref")]
    name: String,
    #[serde(default)]
    sha: String,
}
#[derive(Deserialize)]
struct NativePull {
    number: u64,
    state: String,
    draft: bool,
    merged_at: Option<String>,
    head: NativeRef,
}
#[derive(Deserialize)]
struct NativeStack {
    number: u64,
    open: bool,
    base: NativeRef,
    pull_requests: Vec<NativePull>,
}
#[derive(Deserialize)]
struct AsyncResult {
    status: String,
    details: AsyncDetails,
}
#[derive(Deserialize)]
struct AsyncDetails {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    expected_head_sha: String,
    #[serde(default)]
    sha: String,
}
fn invalid(message: impl Into<String>) -> DeliveryError {
    DeliveryError::Conflict(message.into())
}
fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl DeliveryService {
    async fn native_api(
        &self,
        path: &Path,
        repo: &ForgeRepository,
        suffix: &str,
    ) -> DeliveryResult<String> {
        self.github_read(
            path,
            &[
                "api",
                &format!("repos/{}{suffix}", repo.project),
                "--hostname",
                &repo.host,
            ],
        )
        .await
    }
    pub(super) async fn prepare_native_stack(
        &self,
        path: &Path,
        number: u64,
    ) -> DeliveryResult<NativeStackPlan> {
        let repo = self
            .forge_repository(path, Forge::GitHub)
            .await?
            .ok_or_else(|| invalid("A native stack requires a credential-free GitHub origin."))?;
        let text = self
            .native_api(path, &repo, &format!("/stacks?pull_request={number}"))
            .await?;
        let mut stacks: Vec<NativeStack> = serde_json::from_str(&text).map_err(|_| {
            invalid("GitHub returned an unreadable native-stack membership response.")
        })?;
        if stacks.len() != 1 {
            return Err(invalid(
                "Choose a pull request belonging to exactly one GitHub native stack.",
            ));
        }
        let stack = stacks.remove(0);
        if !stack.open
            || stack.number == 0
            || !bruhs_models::logic::git::is_valid_branch(&stack.base.name)
        {
            return Err(invalid(
                "The native stack is closed or has an invalid base.",
            ));
        }
        let text = self
            .native_api(
                path,
                &repo,
                &format!(
                    "/git/ref/heads/{}",
                    super::gitlab::segment(&stack.base.name)
                ),
            )
            .await?;
        let reference: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| invalid("GitHub returned an unreadable root branch."))?;
        let base_oid = reference["object"]["sha"]
            .as_str()
            .filter(|oid| valid_oid(oid))
            .ok_or_else(|| invalid("GitHub returned no root branch commit."))?
            .to_owned();
        let mut seen = HashSet::new();
        let mut layers = Vec::new();
        for member in stack.pull_requests {
            if member.number == 0 || !seen.insert(member.number) {
                return Err(invalid(
                    "The native stack contains duplicate or invalid PR numbers.",
                ));
            }
            if member
                .merged_at
                .as_ref()
                .is_some_and(|time| !time.is_empty())
            {
                continue;
            }
            if member.state != "open" || member.draft || !valid_oid(&member.head.sha) {
                return Err(invalid("Every unmerged native-stack member must be open, ready for review and have a full head commit."));
            }
            let pull = self.pull_for(path, member.number, Forge::GitHub).await?;
            if pull.state != "OPEN"
                || pull.draft
                || !pull.head_in_repository
                || pull.head != member.head.name
                || pull.head_oid != member.head.sha
                || pull.mergeable != "MERGEABLE"
                || pull.checks.iter().any(|check| {
                    check.pending()
                        || !matches!(
                            check.conclusion.as_deref().unwrap_or(&check.state),
                            "SUCCESS" | "NEUTRAL" | "SKIPPED"
                        )
                })
                || !matches!(pull.review_decision.as_deref(), None | Some("APPROVED"))
            {
                return Err(invalid(format!("Native stack PR #{} is changing, forked, blocked, draft, or awaiting checks/review. Refresh it before owner approval.",member.number)));
            }
            layers.push(StackLayer {
                number: member.number,
                head: pull.head,
                base: pull.base,
                head_oid: pull.head_oid,
                base_oid: pull.base_oid,
                local_oid: None,
                replay_after_oid: None,
            });
        }
        let merge_through = layers
            .last()
            .ok_or_else(|| invalid("The native stack has no open members to merge."))?
            .number;
        if !seen.contains(&number) {
            return Err(invalid("The selected PR is absent from this native stack."));
        }
        Ok(NativeStackPlan {
            repository: repo.project,
            stack_number: stack.number,
            base: stack.base.name,
            base_oid,
            merge_through,
            layers,
        })
    }

    pub(super) async fn run_native_stack(
        &self,
        prepared: &PreparedAction,
        preview: &NativeStackPlan,
        mut cancelled: watch::Receiver<bool>,
    ) -> DeliveryResult<()> {
        let execution = async {
            let DeliveryAction::MergeNativeStack { number, method } = prepared.action else {
                return Err(invalid(
                    "The native stack proposal has an incompatible action.",
                ));
            };
            let fresh = self
                .prepare_native_stack(Path::new(&prepared.path), number)
                .await?;
            if fresh.repository != preview.repository
                || fresh.stack_number != preview.stack_number
                || fresh.base != preview.base
                || fresh.layers.len() != preview.layers.len()
                || fresh
                    .layers
                    .iter()
                    .zip(&preview.layers)
                    .any(|(fresh, reviewed)| {
                        fresh.number != reviewed.number || fresh.head != reviewed.head
                    })
            {
                return Err(invalid("Native stack membership or destination changed. Prepare a new owner review; current-head acknowledgement does not authorize different PRs."));
            }
            if *cancelled.borrow() {
                return Err(invalid("Native stack submission cancelled before writing."));
            }
            let repo = prepared
                .repository
                .as_ref()
                .ok_or_else(|| invalid("The native stack has no frozen repository identity."))?;
            let operation=NativeMergeOperation{acknowledged_server_heads:true,acknowledged_at_ms:super::now(),preflight:fresh.clone(),operation_id:None,state:NativeMergeState::Submitting,server_head_oid:None,merge_commit_oid:None,message:"Submitting once. GitHub may use heads newer than this preflight; no expected-head request guard exists.".into()};
            self.mutate(|state| {
                let receipt = state
                    .actions
                    .iter_mut()
                    .find(|receipt| receipt.prepared.id == prepared.id)
                    .ok_or_else(|| invalid("The native receipt is unavailable."))?;
                if receipt.status != ActionStatus::Running {
                    return Err(invalid("Native stack execution stopped before submission."));
                }
                receipt.native_operation = Some(operation);
                Ok(())
            })
            .await?;
            let method = match method {
                MergeMethod::Merge => "merge",
                MergeMethod::Squash => "squash",
                MergeMethod::Rebase => "rebase",
            };
            let command = Command {
                program: "gh".into(),
                args: vec![
                    "api".into(),
                    format!(
                        "repos/{}/pulls/{}/merge-async",
                        repo.project, fresh.merge_through
                    ),
                    "--hostname".into(),
                    repo.host.clone(),
                    "--method".into(),
                    "PUT".into(),
                    "--input".into(),
                    "-".into(),
                ],
                stdin: Some(
                    serde_json::json!({"merge_method":method,"merge_action":"default"}).to_string(),
                ),
            };
            let output = match self
                .run_command(
                    Path::new(&prepared.path),
                    &command,
                    &mut cancelled,
                    Some(repo),
                )
                .await
            {
                Ok(output) => output,
                Err(error) => {
                    self.native_unknown(&prepared.id,"The submit response is unknown. GitHub may have accepted it; inspect the server before preparing another action.").await?;
                    return Err(error);
                }
            };
            self.record_native_result(&prepared.id, &output).await?;
            self.native_outcome(&prepared.id).await
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(prepared.timeout_seconds),
            execution,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                self.native_unknown(&prepared.id,"The reviewed execution budget ended. The server operation may continue; refresh its saved UUID or inspect GitHub. It will not be resubmitted.").await?;
                Err(invalid("The native stack exceeded its execution budget; the remote outcome may be unknown."))
            }
        }
    }
    async fn native_unknown(&self, id: &str, message: &str) -> DeliveryResult<()> {
        self.mutate(|state| {
            let receipt = state
                .actions
                .iter_mut()
                .find(|receipt| receipt.prepared.id == id)
                .ok_or_else(|| invalid("The native receipt is unavailable."))?;
            if let Some(operation) = &mut receipt.native_operation {
                operation.state = NativeMergeState::Unknown;
                operation.message = message.into();
            }
            Ok(())
        })
        .await
    }
    async fn record_native_result(&self, id: &str, output: &str) -> DeliveryResult<()> {
        let result: AsyncResult = match serde_json::from_str(output) {
            Ok(result) => result,
            Err(_) => {
                self.native_unknown(id,"GitHub returned an unreadable response. Inspect the server before another submission.").await?;
                return Err(invalid(
                    "GitHub returned an unreadable native merge result.",
                ));
            }
        };
        let state = match result.status.as_str() {
            "pending" => NativeMergeState::Pending,
            "merged" => NativeMergeState::Merged,
            "enqueued" => NativeMergeState::Enqueued,
            "failed" => NativeMergeState::Failed,
            _ => NativeMergeState::Unknown,
        };
        let uuid = if result.details.uuid.is_empty() {
            None
        } else {
            Some(uuid::Uuid::parse_str(&result.details.uuid).map_err(|_|invalid("GitHub returned an invalid native operation UUID; inspect the server before retrying."))?.to_string())
        };
        self.mutate(|journal| {
            let receipt = journal
                .actions
                .iter_mut()
                .find(|receipt| receipt.prepared.id == id)
                .ok_or_else(|| invalid("The native receipt is unavailable."))?;
            let operation = receipt
                .native_operation
                .as_mut()
                .ok_or_else(|| invalid("No native stack submission was recorded."))?;
            if let Some(uuid) = uuid {
                if operation
                    .operation_id
                    .as_ref()
                    .is_some_and(|saved| *saved != uuid)
                {
                    return Err(invalid("The native operation UUID changed unexpectedly."));
                }
                operation.operation_id = Some(uuid);
            }
            operation.state = state;
            operation.message = result.details.message.clone();
            operation.server_head_oid = (!result.details.expected_head_sha.is_empty())
                .then(|| result.details.expected_head_sha.clone());
            operation.merge_commit_oid =
                (!result.details.sha.is_empty()).then(|| result.details.sha.clone());
            receipt.updated_at_ms = super::now();
            Ok(())
        })
        .await
    }
    async fn native_outcome(&self, id: &str) -> DeliveryResult<()> {
        let receipt = self
            .state
            .lock()
            .await
            .actions
            .iter()
            .find(|receipt| receipt.prepared.id == id)
            .cloned()
            .ok_or_else(|| invalid("The native receipt is unavailable."))?;
        let operation = receipt
            .native_operation
            .as_ref()
            .ok_or_else(|| invalid("No native merge result was recorded."))?;
        // Verify actual per-PR completion even when GitHub reports a partially failed stack.
        let mut merged = 0;
        for layer in &operation.preflight.layers {
            let pull = self
                .pull_for(
                    Path::new(&receipt.prepared.path),
                    layer.number,
                    Forge::GitHub,
                )
                .await?;
            if pull.state == "MERGED" {
                merged += 1;
            }
        }
        self.mutate(|state| {
            let saved = state
                .actions
                .iter_mut()
                .find(|receipt| receipt.prepared.id == id)
                .ok_or_else(|| invalid("The native receipt is unavailable."))?;
            saved.completed_commands = merged;
            Ok(())
        })
        .await?;
        if operation.state == NativeMergeState::Merged && merged == operation.preflight.layers.len()
        {
            Ok(())
        } else {
            Err(invalid(match operation.state{NativeMergeState::Pending=>"GitHub is still processing the saved native operation. Refresh its status; do not resubmit.",NativeMergeState::Enqueued=>"GitHub enqueued this native stack. Queue acceptance does not confirm a merge. Refresh its status; do not resubmit.",NativeMergeState::Failed=>"GitHub reported native merge failure. Inspect the saved per-PR completed count and server state before preparing another action.",_=>"The native merge outcome is incomplete or unknown. Inspect GitHub and refresh the saved operation UUID; no automatic resubmission occurs."}))
        }
    }
    /// Read only: resumes observation of one saved operation UUID, never submits a merge.
    pub async fn refresh_native_stack(&self, id: &str) -> DeliveryResult<ActionReceipt> {
        let receipt = self
            .state
            .lock()
            .await
            .actions
            .iter()
            .find(|receipt| receipt.prepared.id == id)
            .cloned()
            .ok_or_else(|| invalid("The native receipt is unavailable."))?;
        if receipt.status == ActionStatus::Running {
            return Err(invalid(
                "Wait for the active native submission before refreshing it.",
            ));
        }
        let operation = receipt
            .native_operation
            .as_ref()
            .ok_or_else(|| invalid("This action has no native operation."))?;
        let uuid=operation.operation_id.as_deref().ok_or_else(||invalid("GitHub returned no operation UUID. Inspect the server; this submission will not be replayed."))?;
        let repo = receipt
            .prepared
            .repository
            .as_ref()
            .ok_or_else(|| invalid("The native repository identity is unavailable."))?;
        if self
            .forge_repository(Path::new(&receipt.prepared.path), Forge::GitHub)
            .await?
            .as_ref()
            != Some(repo)
        {
            return Err(invalid(
                "The repository origin changed. Restore it before refreshing this operation.",
            ));
        }
        let output = self
            .native_api(
                Path::new(&receipt.prepared.path),
                repo,
                &format!(
                    "/pulls/{}/merge-async/{uuid}",
                    operation.preflight.merge_through
                ),
            )
            .await?;
        self.record_native_result(id, &output).await?;
        let outcome = self.native_outcome(id).await;
        self.mutate(|state| {
            let saved = state
                .actions
                .iter_mut()
                .find(|receipt| receipt.prepared.id == id)
                .ok_or_else(|| invalid("The native receipt is unavailable."))?;
            saved.status = if outcome.is_ok() {
                ActionStatus::Completed
            } else if saved
                .native_operation
                .as_ref()
                .is_some_and(|operation| operation.state == NativeMergeState::Failed)
            {
                ActionStatus::Failed
            } else {
                ActionStatus::Interrupted
            };
            saved.error = outcome.err().map(|error| error.to_string());
            saved.updated_at_ms = super::now();
            Ok(saved.clone())
        })
        .await
    }
}
