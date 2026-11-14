use super::*;

fn gh(args: &[&str]) -> Command {
    Command {
        program: "gh".into(),
        args: args.iter().map(|argument| (*argument).to_owned()).collect(),
        stdin: None,
    }
}

impl DeliveryService {
    pub(super) async fn merge_stack(
        &self,
        prepared: &PreparedAction,
        plan: &StackPlan,
        method: MergeMethod,
        cancel: &mut watch::Receiver<bool>,
    ) -> DeliveryResult<()> {
        let path = Path::new(&prepared.path);
        let merge_method = method;
        let gitlab = prepared.repository.as_ref().filter(|repository| {
            prepared.forge == Forge::GitLab && repository.forge == Forge::GitLab
        });
        // Check every layer before any write. Each later merge is checked again
        // after retargeting because new CI and reviews may become required then.
        for layer in &plan.layers {
            self.no_native_merge(path, &plan.repository, layer.number, prepared.forge)
                .await?;
            validate_merge(
                &self.pull_for(path, layer.number, prepared.forge).await?,
                layer,
                &layer.base,
                &layer.base_oid,
            )?;
        }
        let method = match method {
            MergeMethod::Merge => "--merge",
            MergeMethod::Squash => "--squash",
            MergeMethod::Rebase => "--rebase",
        };
        let mut root = plan.base_oid.clone();
        for (index, layer) in plan.layers.iter().enumerate() {
            self.stack_running(prepared, cancel).await?;
            if self
                .remote_base(path, &plan.repository, &plan.base, prepared.forge)
                .await?
                != root
            {
                return Err(conflict(
                    "The root branch changed outside this stack operation. Earlier completed merges remain applied; later layers were stopped.",
                ));
            }
            // Reject changed future heads/bases before retargeting this layer.
            let mut retargeted = index == 0;
            for (offset, future) in plan.layers[index..].iter().enumerate() {
                let current = self.pull_for(path, future.number, prepared.forge).await?;
                if index > 0 && offset == 0 && current.base == plan.base && current.base_oid == root
                {
                    // GitHub may retarget a dependent PR automatically when the
                    // just-merged parent branch is deleted by repository policy.
                    validate_merge(&current, future, &plan.base, &root)?;
                    retargeted = true;
                } else {
                    validate_merge(&current, future, &future.base, &future.base_oid)?;
                }
            }
            self.no_native_merge(path, &plan.repository, layer.number, prepared.forge)
                .await?;
            if !retargeted {
                let retarget = if let Some(repository) = gitlab {
                    super::super::gitlab::api(
                        repository,
                        "PUT",
                        &format!("/merge_requests/{}", layer.number),
                        Some(serde_json::json!({"target_branch":plan.base})),
                    )
                } else {
                    gh(&[
                        "pr",
                        "edit",
                        &layer.number.to_string(),
                        "--base",
                        &plan.base,
                    ])
                };
                self.stack_exec(prepared, path, retarget, cancel).await?;
                self.stack_log(prepared, None, &format!("Retargeted PR #{} from {} to {}. This server change remains applied if its refreshed checks or reviews stop the next merge.", layer.number, layer.base, plan.base)).await?;
            }
            let current = self.pull_for(path, layer.number, prepared.forge).await?;
            validate_merge(&current, layer, &plan.base, &root)?;
            // GitHub enforces current branch protection and reviews; expected-head
            // guards the reviewed commit against changes between read and merge.
            let merge = if let Some(repository) = gitlab {
                self.gitlab_plan(
                    path,
                    repository,
                    &DeliveryAction::Merge {
                        number: layer.number,
                        head_oid: layer.head_oid.clone(),
                        method: merge_method,
                        auto: false,
                    },
                    Vec::new(),
                )
                .await?
                .pop()
                .ok_or_else(|| conflict("GitLab produced no reviewed merge request."))?
            } else {
                gh(&[
                    "pr",
                    "merge",
                    &layer.number.to_string(),
                    method,
                    "--match-head-commit",
                    &layer.head_oid,
                ])
            };
            self.stack_exec(prepared, path, merge, cancel).await?;
            let merged = self.pull_for(path, layer.number, prepared.forge).await?;
            if merged.state != "MERGED" {
                return Err(conflict(format!(
                    "GitHub accepted the merge request for PR #{} but has not confirmed it merged (it may be queued). Later layers were stopped. Inspect GitHub before preparing another stack action.",
                    layer.number
                )));
            }
            if merged.head_oid != layer.head_oid || merged.base != plan.base {
                return Err(conflict(format!(
                    "PR #{} merged with an unexpected head or destination. Stop and inspect GitHub; no later layer was attempted.",
                    layer.number
                )));
            }
            self.stack_log(
                prepared,
                Some(index + 1),
                &format!(
                    "Merged PR #{} at reviewed head {}. {} of {} PRs completed.",
                    layer.number,
                    layer.head_oid,
                    index + 1,
                    plan.layers.len()
                ),
            )
            .await?;
            root = merged.merge_commit_oid.ok_or_else(|| conflict(format!("PR #{} merged, but GitHub did not return its merge commit. The completed count is saved; no later layer was attempted.", layer.number)))?;
            valid_oid(&root)?;
            if self
                .remote_base(path, &plan.repository, &plan.base, prepared.forge)
                .await?
                != root
            {
                return Err(conflict(
                    "The root changed after a completed merge. Its completed count is saved; no later layer was attempted.",
                ));
            }
        }
        Ok(())
    }
}
