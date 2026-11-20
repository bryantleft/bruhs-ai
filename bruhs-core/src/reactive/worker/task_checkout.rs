//! Resolve the checkout before starting a scheduled provider turn.
use crate::git;
use bruhs_models::{
    logic::run_place::WorktreePreferences,
    reactive::{ReactiveRun, TaskCheckout},
};
use std::path::Path;

pub(super) async fn resolve(project: &Path, run: &ReactiveRun) -> Result<String, String> {
    let checkout = match &run.task_checkout {
        TaskCheckout::Project => project.to_owned(),
        TaskCheckout::Existing { path } => {
            let path = std::fs::canonicalize(path)
                .map_err(|error| format!("Cannot open the selected checkout at {path}. {error}"))?;
            if !path.is_dir()
                || git::worktree_branch(project, &path)
                    .await
                    .map_err(|e| e.to_string())?
                    .is_none()
            {
                return Err(format!(
                    "{} is not a registered checkout of {}.",
                    path.display(),
                    project.display()
                ));
            }
            path
        }
        TaskCheckout::Worktree {
            base_branch,
            start_from_remote,
        } => {
            let branch = format!("bruhs/task-{}", run.thread_id.as_str());
            let registered = git::registered_worktrees(project)
                .await
                .map_err(|e| e.to_string())?;
            if let Some((checkout, _)) = registered
                .into_iter()
                .find(|(_, name)| name.as_deref() == Some(branch.as_str()))
            {
                if !checkout.is_dir() {
                    return Err(format!(
                        "The task worktree is missing at {}.",
                        checkout.display()
                    ));
                }
                checkout
            } else {
                let dest = git::worktree_dest(project, &branch).map_err(|e| e.to_string())?;
                let prefs = WorktreePreferences {
                    start_from_remote: *start_from_remote,
                    ..Default::default()
                };
                git::add_worktree_with_preferences(project, base_branch, &dest, &branch, prefs)
                    .await
                    .map_err(|e| {
                        format!("Could not create the task worktree from {base_branch}. {e}")
                    })?;
                dest
            }
        }
    };
    if !run.task_checkout.is_project() {
        // Git can still list a removed worktree after an unrelated checkout has taken its place.
        let project_store = git::common_git_dir(project)
            .await
            .map_err(|e| e.to_string())?;
        let actual_store = git::common_git_dir(&checkout)
            .await
            .map_err(|e| e.to_string())?;
        if !checkout.join(".git").exists()
            || std::fs::canonicalize(project_store).map_err(|e| e.to_string())?
                != std::fs::canonicalize(actual_store).map_err(|e| e.to_string())?
        {
            return Err(format!(
                "{} no longer belongs to the task project.",
                checkout.display()
            ));
        }
    }
    checkout
        .into_os_string()
        .into_string()
        .map_err(|_| "The checkout path cannot be represented as text.".into())
}
