//! Keep the sidebar in sync with pull requests, including for background threads.
use super::*;
use crate::app_state::spawn_tokio;
use bruhs_core::pulls::BranchPullLookup;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    merged: bool,
    keep: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Observation {
    pub project: String,
    pub branch: String,
    pub number: u64,
    state: String,
    /// An explicit restore or new activity must survive future checks of the same PR.
    handled: bool,
}

impl Workspace {
    pub(super) fn thread_settle_delay(&self, thread: &DesktopThread, cx: &App) -> Option<i64> {
        let path = thread.chat.read(cx).settings_project_path();
        let keep = self
            .project_defaults
            .get(path)
            .and_then(|p| p.keep_pull)
            .unwrap_or(self.pull_preferences.keep);
        if keep
            && self.known_thread_pulls.get(&thread.id).is_some_and(|pull| {
                pull.project == path
                    && thread.branch.as_ref() == Some(&pull.branch)
                    && matches!(pull.state.as_str(), "open" | "draft")
            })
        {
            return None;
        }
        self.idle_settle_after_ms(path)
    }

    pub(super) fn observe_thread_pull(
        &mut self,
        id: &str,
        path: &str,
        branch: &str,
        lookup: &BranchPullLookup,
        started: Instant,
        cx: &mut Context<'_, Self>,
    ) {
        if !lookup.signed_in
            || lookup.error.is_some()
            || self
                .pull_check_times
                .get(id)
                .is_some_and(|last| *last > started)
        {
            return;
        }
        let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if thread.branch.as_deref() != Some(branch)
            || thread.chat.read(cx).settings_project_path() != path
        {
            return;
        }
        self.pull_check_times.insert(id.to_owned(), started);
        let Some(pull) = lookup.pull.as_ref() else {
            if self.known_thread_pulls.remove(id).is_some() {
                self.persist_and_notify(cx)
            }
            return;
        };
        let previous = self.known_thread_pulls.get(id);
        let handled = previous.is_some_and(|p| {
            p.handled
                && p.project == path
                && p.branch == branch
                && p.number == pull.number
                && p.state == pull.state
        });
        let settle_merged = self
            .project_defaults
            .get(path)
            .and_then(|p| p.settle_merged)
            .unwrap_or(self.pull_preferences.merged);
        let terminal = pull.state == "closed" || (settle_merged && pull.state == "merged");
        let idle = !thread.pinned
            && thread.snoozed_until_ms.is_none()
            && !thread.chat.read(cx).is_streaming
            && !self.unread_thread_ids.contains(id)
            && self.active_thread_id.as_deref() != Some(id);
        let apply = !handled && terminal && (thread.settled || idle);
        let next = Observation {
            project: path.to_owned(),
            branch: branch.to_owned(),
            number: pull.number,
            state: pull.state.clone(),
            handled: handled || apply,
        };
        if previous == Some(&next) {
            return;
        }
        let newly_settled=apply && !thread.settled;
        if apply {
            thread.settled = true;
            thread.active_rank = None;
        }
        self.known_thread_pulls.insert(id.to_owned(), next);
        if newly_settled{self.launch_thread_hooks(id,true,cx);}
        self.persist_and_notify(cx);
    }

    pub(super) fn start_pull_lifecycle(&mut self, cx: &mut Context<'_, Self>) {
        if cfg!(test) {
            return;
        }
        self.new_idle_action_threads(cx);
        self.pull_lifecycle_task = Some(cx.spawn(async move |this, cx| {
            let mut last_key = None;
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
                let Ok(mut groups) = this.update(cx, |this, cx| {
                    for id in this.new_idle_action_threads(cx) {this.launch_thread_hooks(&id,true,cx);}
                    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
                    for t in &this.threads {
                        let path = labs::folder(t, cx);
                        let Some(branch) = t.branch.as_deref().filter(|branch| !branch.is_empty())
                        else {
                            continue;
                        };
                        if !path.is_empty() {
                            groups
                                .entry((path.to_owned(), branch.to_owned()))
                                .or_default()
                                .push(t.id.clone())
                        }
                    }
                    groups.into_iter().collect::<Vec<_>>()
                }) else {
                    return;
                };
                if let Some(key) = &last_key {
                    let at = groups.partition_point(|(k, _)| k <= key);
                    groups.rotate_left(at);
                }
                for ((path, branch), ids) in groups.into_iter().take(4) {
                    last_key = Some((path.clone(), branch.clone()));
                    let repo = std::path::PathBuf::from(&path);
                    let head = branch.clone();
                    let started = Instant::now();
                    let Ok(task) = this.update(cx, |_, cx| {
                        spawn_tokio(cx, async move {
                            if !repo.join(".git").exists() {
                                return None;
                            }
                            Some(bruhs_core::pulls::pull_for_branch(&repo, &head).await)
                        })
                    }) else {
                        return;
                    };
                    if let Some(found) = task.await {
                        if this
                            .update(cx, |this, cx| {
                                for id in &ids {
                                    this.observe_thread_pull(
                                        id, &path, &branch, &found, started, cx,
                                    )
                                }
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }));
    }

    pub(super) fn choose_pull_policy(&mut self, id: &str, value: &str, cx: &mut Context<'_, Self>) {
        let next = match value {
            "inherit" => None,
            "on" => Some(true),
            "off" => Some(false),
            _ => return,
        };
        let merged = id == "settings-settle-merged";
        let scope = &self.settings_project_scope;
        if scope.is_empty() {
            let Some(value) = next else { return };
            if merged {
                self.pull_preferences.merged = value
            } else {
                self.pull_preferences.keep = value
            }
        } else {
            let prefs = self.project_defaults.entry(scope.clone()).or_default();
            if merged {
                prefs.settle_merged = next
            } else {
                prefs.keep_pull = next
            }
            if *prefs == Default::default() {
                self.project_defaults.remove(scope);
            }
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn pull_settlement_rows(
        &self,
        inactivity: bool,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let scope = &self.settings_project_scope;
        let project = self.project_defaults.get(scope);
        let rows = [
            (
                "desktop-row-settle-merged",
                "settings-settle-merged",
                "Auto-settle merged threads",
                "Settle a thread when its pull request merges. Closed pull requests still settle automatically.",
                self.pull_preferences.merged,
                project.and_then(|p| p.settle_merged),
            ),
            (
                "desktop-row-keep-pull",
                "settings-keep-pull",
                "Keep threads with a PR",
                "Keep threads with an open pull request active during periods of inactivity.",
                self.pull_preferences.keep,
                project.and_then(|p| p.keep_pull),
            ),
        ];
        rows.into_iter()
            .filter(|(_, id, ..)| inactivity || *id == "settings-settle-merged")
            .map(|(row, id, label, description, default, over)| {
                let control = if scope.is_empty() {
                    settings_ui::switch(id, default, false, label, cx, move |this, _, cx| {
                        this.choose_pull_policy(id, if default { "off" } else { "on" }, cx)
                    })
                } else {
                    settings_ui::control_column(self.choice_control(
                        id,
                        label,
                        match over {
                            Some(true) => "on",
                            Some(false) => "off",
                            None => "inherit",
                        },
                        &[
                            ("inherit", "Use environment default"),
                            ("on", "On"),
                            ("off", "Off"),
                        ],
                        cx,
                    ))
                };
                self.settings_row(row, label, description, control)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{BranchPullLookup, Instant, Duration};

    #[gpui::test]
    fn observations_respect_project_overrides_and_manual_restore(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, general, code) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |w, _, cx| {
            let path = "/tmp/alpha";
            w.active_thread_id = Some(general);
            w.unread_thread_ids.remove(&code);
            let idx = w.threads.iter().position(|t| t.id == code).unwrap();
            w.threads[idx].branch = Some("feature".into());
            w.settings_project_scope = path.to_owned();
            w.idle_settle_days = 3;
            w.choose_pull_policy("settings-keep-pull", "on", cx);
            w.choose_pull_policy("settings-settle-merged", "on", cx);
            let result = |state: &str| BranchPullLookup {
                signed_in: true,
                error: None,
                pull: Some(bruhs_core::pulls::PullRef {
                    number: 42,
                    title: String::new(),
                    url: String::new(),
                    state: state.to_owned(),
                }),
            };
            let first = Instant::now();
            let later = first + Duration::from_secs(1);
            w.observe_thread_pull(&code, path, "feature", &result("open"), first, cx);
            assert_eq!(w.thread_settle_delay(&w.threads[idx], cx), None);
            w.observe_thread_pull(&code, path, "feature", &result("merged"), later, cx);
            assert!(w.threads[idx].settled);
            assert!(w.snapshot(cx).known_thread_pulls[&code].handled);
            w.toggle_settled_by_id(&code, cx);
            w.observe_thread_pull(&code, path, "feature", &result("open"), first, cx);
            w.observe_thread_pull(&code, path, "feature", &result("merged"), later, cx);
            assert!(!w.threads[idx].settled);
            assert!(w.thread_settle_delay(&w.threads[idx], cx).is_some());
            w.observe_thread_pull(&code, "/other", "feature", &result("closed"), later, cx);
            assert_eq!(w.known_thread_pulls[&code].state, "merged");
            w.choose_pull_policy("settings-settle-merged", "inherit", cx);
            w.observe_thread_pull(&code, path, "feature", &result("closed"), later, cx);
            assert!(w.threads[idx].settled);
        });
    }
}
