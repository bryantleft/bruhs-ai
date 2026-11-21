use super::{command, now, DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::{delivery::*, logic::git::is_valid_branch};
use serde::Deserialize;
use std::{collections::HashSet, path::Path};
use tokio::sync::watch;

mod merge;
mod rebase;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GraphPull {
    number: u64,
    head: String,
    base: String,
    head_oid: String,
    base_oid: String,
    head_repository: String,
    base_repository: String,
}

fn conflict(message: impl Into<String>) -> DeliveryError {
    DeliveryError::Conflict(message.into())
}

fn valid_oid(oid: &str) -> DeliveryResult<()> {
    if matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(conflict(
            "The stack contains an invalid commit ID. Reload it before continuing.",
        ))
    }
}

fn chain<'a>(
    pulls: &'a [GraphPull],
    repository: &str,
    selected: u64,
) -> DeliveryResult<Vec<&'a GraphPull>> {
    let mut numbers = HashSet::new();
    if pulls.iter().any(|pull| !numbers.insert(pull.number)) {
        return Err(conflict(
            "GitHub returned duplicate pull request numbers; reload the stack.",
        ));
    }
    let mut current = pulls
        .iter()
        .find(|pull| pull.number == selected)
        .ok_or_else(|| conflict("The selected pull request is no longer open."))?;
    let mut seen = HashSet::new();
    loop {
        validate_node(current, repository, pulls)?;
        if !seen.insert(current.number) {
            return Err(conflict(
                "The branch relationships contain a cycle; resolve it before using a stack action.",
            ));
        }
        let mut parents = pulls.iter().filter(|parent| {
            parent.head == current.base
                && parent.head_repository.eq_ignore_ascii_case(repository)
                && parent.base_repository.eq_ignore_ascii_case(repository)
        });
        let Some(parent) = parents.next() else { break };
        if parents.next().is_some() {
            return Err(conflict(
                "More than one open PR owns a parent branch. Stack membership is ambiguous.",
            ));
        }
        current = parent;
    }
    let mut layers = Vec::new();
    seen.clear();
    loop {
        validate_node(current, repository, pulls)?;
        if !seen.insert(current.number) {
            return Err(conflict(
                "The branch relationships contain a cycle; resolve it before using a stack action.",
            ));
        }
        if let Some(previous) = layers.last().copied() {
            let previous: &GraphPull = previous;
            if current.base_oid != previous.head_oid {
                return Err(conflict(
                    "A stack branch changed while its graph was being read. Reload and review it again.",
                ));
            }
        }
        layers.push(current);
        let mut children = pulls.iter().filter(|child| {
            child.base == current.head && child.base_repository.eq_ignore_ascii_case(repository)
        });
        let Some(child) = children.next() else { break };
        if children.next().is_some() {
            return Err(conflict(
                "This stack branches into multiple child PRs. Choose and resolve one linear stack before continuing.",
            ));
        }
        current = child;
    }
    Ok(layers)
}

fn validate_node(pull: &GraphPull, repository: &str, pulls: &[GraphPull]) -> DeliveryResult<()> {
    if !pull.head_repository.eq_ignore_ascii_case(repository)
        || !pull.base_repository.eq_ignore_ascii_case(repository)
    {
        return Err(conflict(
            "Whole-stack actions require every head and base to belong to this repository. Fork branches are not combined by name.",
        ));
    }
    if !is_valid_branch(&pull.head) || !is_valid_branch(&pull.base) {
        return Err(conflict("The stack contains an invalid branch name."));
    }
    valid_oid(&pull.head_oid)?;
    valid_oid(&pull.base_oid)?;
    if pulls
        .iter()
        .filter(|other| {
            other.head == pull.head && other.head_repository.eq_ignore_ascii_case(repository)
        })
        .count()
        != 1
    {
        return Err(conflict(
            "Multiple open PRs share a head branch. Stack membership is ambiguous.",
        ));
    }
    Ok(())
}

fn component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

impl DeliveryService {
    async fn gitlab_stack_graph(&self, path: &Path) -> DeliveryResult<(String, Vec<GraphPull>)> {
        let repo = self
            .forge_repository(path, Forge::GitLab)
            .await?
            .ok_or_else(|| conflict("GitLab requires a repository origin."))?;
        let mut command = super::gitlab::api(
            &repo,
            "GET",
            "/merge_requests?state=opened&scope=all&per_page=100",
            None,
        );
        command.args.push("--paginate".into());
        let text = self
            .read(
                path,
                "glab",
                &command.args.iter().map(String::as_str).collect::<Vec<_>>(),
            )
            .await?;
        let list: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|_| {
            conflict("The complete GitLab MR graph was unreadable. Upgrade glab and reload it.")
        })?;
        let mut graph = Vec::with_capacity(list.len());
        for item in list {
            let number = item["iid"]
                .as_u64()
                .filter(|number| *number > 0)
                .ok_or_else(|| conflict("GitLab returned an invalid MR number."))?;
            let mr = self
                .gitlab_json(path, &repo, &format!("/merge_requests/{number}"))
                .await?;
            let source = mr["source_project_id"]
                .as_u64()
                .ok_or_else(|| conflict("GitLab returned no source project identity."))?;
            let target = mr["target_project_id"]
                .as_u64()
                .ok_or_else(|| conflict("GitLab returned no target project identity."))?;
            let head = mr["source_branch"]
                .as_str()
                .ok_or_else(|| conflict("GitLab returned no source branch."))?
                .to_owned();
            let base = mr["target_branch"]
                .as_str()
                .ok_or_else(|| conflict("GitLab returned no target branch."))?
                .to_owned();
            let head_oid = mr["sha"]
                .as_str()
                .ok_or_else(|| conflict("GitLab returned no MR commit."))?
                .to_owned();
            let base_oid = self
                .remote_base(path, &repo.project, &base, Forge::GitLab)
                .await?;
            graph.push(GraphPull {
                number,
                head,
                base,
                head_oid,
                base_oid,
                head_repository: if source == target {
                    repo.project.clone()
                } else {
                    format!("fork:{source}")
                },
                base_repository: repo.project.clone(),
            });
        }
        Ok((repo.project, graph))
    }
    async fn stack_graph(
        &self,
        path: &Path,
        forge: Forge,
    ) -> DeliveryResult<(String, Vec<GraphPull>)> {
        if forge == Forge::GitLab {
            return self.gitlab_stack_graph(path).await;
        }
        let repository = self
            .github_read(
                path,
                &[
                    "repo",
                    "view",
                    "--json",
                    "nameWithOwner",
                    "--jq",
                    ".nameWithOwner",
                ],
            )
            .await?
            .trim()
            .to_owned();
        if repository.split('/').count() != 2
            || repository.split('/').any(|part| {
                part.is_empty()
                    || !part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            })
        {
            return Err(conflict(
                "GitHub did not identify a valid owner/repository for this checkout.",
            ));
        }
        let endpoint = format!("repos/{repository}/pulls?state=open&per_page=100");
        // All pages are required: a missing 51st PR could hide an ambiguous branch owner.
        let json = self.github_read(path, &["api", &endpoint, "--paginate", "--slurp", "--jq",
            "map(map({number,head:.head.ref,base:.base.ref,head_oid:.head.sha,base_oid:.base.sha,head_repository:.head.repo.full_name,base_repository:.base.repo.full_name}))"]).await?;
        let pages: Vec<Vec<GraphPull>> = serde_json::from_str(&json).map_err(|error| {
            DeliveryError::Command(format!(
                "The complete GitHub stack graph was invalid: {error}"
            ))
        })?;
        Ok((repository, pages.into_iter().flatten().collect()))
    }

    async fn remote_base(
        &self,
        path: &Path,
        repository: &str,
        branch: &str,
        forge: Forge,
    ) -> DeliveryResult<String> {
        if forge == Forge::GitLab {
            let repo = self
                .forge_repository(path, forge)
                .await?
                .ok_or_else(|| conflict("GitLab requires a repository origin."))?;
            if repo.project != repository {
                return Err(conflict("The GitLab project changed."));
            }
            let value = self
                .gitlab_json(
                    path,
                    &repo,
                    &format!("/repository/branches/{}", component(branch)),
                )
                .await?;
            let oid = value["commit"]["id"]
                .as_str()
                .ok_or_else(|| conflict("GitLab returned no branch commit."))?
                .to_owned();
            valid_oid(&oid)?;
            return Ok(oid);
        }
        let endpoint = format!("repos/{repository}/git/ref/heads/{}", component(branch));
        let oid = self
            .github_read(path, &["api", &endpoint, "--jq", ".object.sha"])
            .await?
            .trim()
            .to_owned();
        valid_oid(&oid)?;
        Ok(oid)
    }

    async fn no_native_merge(
        &self,
        path: &Path,
        repository: &str,
        number: u64,
        forge: Forge,
    ) -> DeliveryResult<()> {
        if forge == Forge::GitLab {
            return Ok(());
        }
        let endpoint = format!("repos/{repository}/stacks?pull_request={number}");
        let json = match self.github_read(path, &["api", &endpoint]).await {
            Ok(json) => json,
            // This optional GitHub API returns 404 when native stacks are not enabled.
            Err(DeliveryError::Command(message)) if message.contains("(HTTP 404)") => return Ok(()),
            Err(error) => return Err(error),
        };
        let native: Vec<serde_json::Value> = serde_json::from_str(&json).map_err(|error| {
            DeliveryError::Command(format!(
                "GitHub native-stack membership was invalid: {error}"
            ))
        })?;
        if native.is_empty() {
            return Ok(());
        }
        Err(conflict(
            "This PR belongs to a GitHub native stack. The official gh-stack merge API does not accept reviewed expected-head leases. Use gh stack merge in a terminal; this reviewed action will not submit unpinned native-stack changes.",
        ))
    }

    pub(super) async fn prepare_stack(
        &self,
        path: &Path,
        selected: u64,
        rebase: bool,
        id: &str,
        forge: Forge,
    ) -> DeliveryResult<StackPlan> {
        let (repository, pulls) = self.stack_graph(path, forge).await?;
        let graph = chain(&pulls, &repository, selected)?;
        let first = graph
            .first()
            .ok_or_else(|| conflict("The stack is empty."))?;
        let base_oid = self
            .remote_base(path, &repository, &first.base, forge)
            .await?;
        if base_oid != first.base_oid {
            return Err(conflict(
                "The base branch changed while loading the stack. Prepare it again.",
            ));
        }
        let mut plan = StackPlan {
            repository,
            base: first.base.clone(),
            base_oid,
            local_base_oid: None,
            layers: graph
                .iter()
                .map(|pull| StackLayer {
                    number: pull.number,
                    head: pull.head.clone(),
                    base: pull.base.clone(),
                    head_oid: pull.head_oid.clone(),
                    base_oid: pull.base_oid.clone(),
                    local_oid: None,
                    replay_after_oid: None,
                })
                .collect(),
            temporary_worktree: None,
        };
        self.stack_checkout_safe(path, &plan).await?;
        if rebase {
            self.prepare_rebase(path, &mut plan, id).await?;
        } else {
            self.no_native_merge(path, &plan.repository, selected, forge)
                .await?;
            for layer in &plan.layers {
                validate_merge(
                    &self.pull_for(path, layer.number, forge).await?,
                    layer,
                    &layer.base,
                    &layer.base_oid,
                )?;
            }
        }
        Ok(plan)
    }

    async fn validate_stack_graph(
        &self,
        path: &Path,
        plan: &StackPlan,
        forge: Forge,
    ) -> DeliveryResult<()> {
        let (repository, pulls) = self.stack_graph(path, forge).await?;
        let selected = plan
            .layers
            .first()
            .ok_or_else(|| conflict("The frozen stack is empty."))?
            .number;
        let current = chain(&pulls, &repository, selected)?;
        if !repository.eq_ignore_ascii_case(&plan.repository)
            || current.len() != plan.layers.len()
            || current.iter().zip(&plan.layers).any(|(now, old)| {
                now.number != old.number
                    || now.head != old.head
                    || now.base != old.base
                    || now.head_oid != old.head_oid
                    || now.base_oid != old.base_oid
            })
            || self
                .remote_base(path, &repository, &plan.base, forge)
                .await?
                != plan.base_oid
        {
            return Err(conflict(
                "Stack membership, a head commit or a base changed after review. Prepare and review the complete stack again.",
            ));
        }
        Ok(())
    }

    async fn stack_checkout_safe(&self, path: &Path, plan: &StackPlan) -> DeliveryResult<()> {
        if !self
            .read(
                path,
                "git",
                &["status", "--porcelain=v1", "--untracked-files=normal"],
            )
            .await?
            .is_empty()
        {
            return Err(conflict(
                "The source checkout has staged, unstaged or untracked changes. Commit or move them before a stack action; no automatic stash is performed.",
            ));
        }
        for marker in [
            "rebase-merge",
            "rebase-apply",
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "BISECT_START",
        ] {
            let location = self
                .read(path, "git", &["rev-parse", "--git-path", marker])
                .await?;
            let location = Path::new(location.trim());
            if path.join(location).exists() {
                return Err(conflict(format!(
                    "Finish the active Git operation ({marker}) before a stack action."
                )));
            }
        }
        let worktrees = self
            .read(path, "git", &["worktree", "list", "--porcelain", "-z"])
            .await?;
        for layer in &plan.layers {
            let branch = format!("branch refs/heads/{}", layer.head);
            if worktrees.split('\0').any(|field| field == branch) {
                return Err(conflict(format!(
                    "Stack branch {} is checked out in a worktree. Switch that worktree to the root branch or detach it before continuing.",
                    layer.head
                )));
            }
        }
        Ok(())
    }

    async fn stack_running(
        &self,
        prepared: &PreparedAction,
        cancel: &watch::Receiver<bool>,
    ) -> DeliveryResult<()> {
        if *cancel.borrow()
            || self
                .state
                .lock()
                .await
                .actions
                .iter()
                .find(|receipt| receipt.prepared.id == prepared.id)
                .is_none_or(|receipt| receipt.status != ActionStatus::Running)
        {
            return Err(conflict(
                "Stack execution stopped. Inspect the receipt for steps already applied before preparing another action.",
            ));
        }
        Ok(())
    }

    async fn stack_log(
        &self,
        prepared: &PreparedAction,
        completed: Option<usize>,
        output: &str,
    ) -> DeliveryResult<()> {
        let full = self
            .mutate(|state| {
                let receipt = state
                    .actions
                    .iter_mut()
                    .find(|receipt| receipt.prepared.id == prepared.id)
                    .ok_or_else(|| conflict("The running stack receipt is unavailable."))?;
                if let Some(completed) = completed {
                    receipt.completed_commands = completed;
                }
                let full = receipt
                    .output
                    .len()
                    .saturating_add(output.len())
                    .saturating_add(1)
                    > command::OUTPUT_BYTES;
                if !full {
                    receipt.output.push_str(output);
                    receipt.output.push('\n');
                }
                receipt.updated_at_ms = now();
                Ok(full)
            })
            .await?;
        if full {
            Err(conflict(
                "The stack receipt reached its 1 MiB output budget. The completed count was saved; inspect applied steps before continuing.",
            ))
        } else {
            Ok(())
        }
    }

    async fn stack_exec(
        &self,
        prepared: &PreparedAction,
        cwd: &Path,
        specification: Command,
        cancel: &mut watch::Receiver<bool>,
    ) -> DeliveryResult<String> {
        self.stack_running(prepared, cancel).await?;
        self.stack_log(
            prepared,
            None,
            &format!(
                "$ {} {}",
                specification.program,
                specification
                    .args
                    .iter()
                    .map(|argument| format!("{argument:?}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        )
        .await?;
        let output = self
            .run_command(cwd, &specification, cancel, prepared.repository.as_ref())
            .await?;
        self.stack_log(prepared, None, &output).await?;
        Ok(output)
    }

    pub(super) async fn run_stack(
        &self,
        prepared: &PreparedAction,
        plan: &StackPlan,
        mut cancel: watch::Receiver<bool>,
    ) -> DeliveryResult<()> {
        let mut cleanup_owned = false;
        let execute = async {
            self.stack_running(prepared, &cancel).await?;
            self.validate_stack_graph(Path::new(&prepared.path), plan, prepared.forge)
                .await?;
            self.stack_checkout_safe(Path::new(&prepared.path), plan)
                .await?;
            match &prepared.action {
                DeliveryAction::RebaseStack { .. } => {
                    self.rebase_stack(prepared, plan, &mut cancel, &mut cleanup_owned)
                        .await
                }
                DeliveryAction::MergeStack { method, .. } => {
                    self.merge_stack(prepared, plan, *method, &mut cancel).await
                }
                _ => Err(conflict(
                    "A stack plan was attached to an incompatible action.",
                )),
            }
        };
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(prepared.timeout_seconds),
            execute,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                let inspect = if matches!(&prepared.action, DeliveryAction::RebaseStack { .. }) {
                    "Inspect local branch refs: their final atomic transaction may have completed. No remote push was submitted."
                } else {
                    "GitHub requests may already have applied; inspect the receipt and server state before continuing."
                };
                Err(conflict(format!("The stack exceeded its reviewed {} second budget. The local process was stopped. {inspect}", prepared.timeout_seconds)))
            }
        };
        // Cleanup is outside the user's execution budget and cancellation channel: the
        // detached scratch checkout is ours, even when a replay process is interrupted.
        if let Some(scratch) = plan.temporary_worktree.as_ref().filter(|_| cleanup_owned) {
            if Path::new(scratch).exists() {
                let hooks = Path::new(scratch)
                    .parent()
                    .ok_or_else(|| conflict("The stack scratch directory is invalid."))?
                    .join("hooks");
                let cleanup = self
                    .read(
                        Path::new(&prepared.path),
                        "git",
                        &[
                            "-c",
                            &format!("core.hooksPath={}", hooks.display()),
                            "worktree",
                            "remove",
                            "--force",
                            scratch,
                        ],
                    )
                    .await;
                if let Err(error) = cleanup {
                    return Err(conflict(format!(
                        "{} Scratch cleanup failed: {error}. Remove the owned detached worktree at {scratch} after inspecting it.",
                        result
                            .err()
                            .map_or_else(|| "The local branch transaction completed.".into(), |error| error.to_string())
                    )));
                }
            }
        }
        result
    }
}

fn validate_merge(
    pull: &Pull,
    layer: &StackLayer,
    base: &str,
    base_oid: &str,
) -> DeliveryResult<()> {
    if pull.state != "OPEN"
        || pull.draft
        || !pull.head_in_repository
        || pull.head != layer.head
        || pull.head_oid != layer.head_oid
        || pull.base != base
        || pull.base_oid != base_oid
    {
        return Err(conflict(format!(
            "PR #{} is draft/closed or its reviewed head/base changed. Review the stack again.",
            layer.number
        )));
    }
    if pull.mergeable != "MERGEABLE" {
        return Err(conflict(format!(
            "PR #{} is conflicting or GitHub has not finished computing mergeability. Resolve or retry after it settles.",
            layer.number
        )));
    }
    if !matches!(pull.review_decision.as_deref(), None | Some("APPROVED")) {
        return Err(conflict(format!(
            "PR #{} still requires an approving review. No later layer will be merged.",
            layer.number
        )));
    }
    if pull.checks.iter().any(|check| {
        !matches!(
            check.conclusion.as_deref().unwrap_or(&check.state),
            "SUCCESS" | "NEUTRAL" | "SKIPPED"
        ) || check.pending()
    }) {
        return Err(conflict(format!(
            "PR #{} has pending, failing or unknown checks. Wait for passing checks and review again.",
            layer.number
        )));
    }
    Ok(())
}
