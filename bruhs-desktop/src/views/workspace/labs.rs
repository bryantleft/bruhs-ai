//! Project folder layout for the native thread sidebar.
use super::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Folders {
    pub enabled: bool,
    pub two_lines: bool,
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub collapsed: std::collections::BTreeSet<String>,
}

pub(super) fn folder<'a>(thread: &'a DesktopThread, cx: &'a App) -> &'a str {
    let chat = thread.chat.read(cx);
    if let Some(root) = chat.scratch_folder().and_then(std::path::Path::parent).and_then(std::path::Path::to_str) {
        root
    } else if chat.project_scoped || !thread.project_name.is_empty() {
        chat.settings_project_path()
    } else {
        ""
    }
}

#[cfg(target_os = "macos")]
pub(super) struct FastCheckout {
    enabled: bool,
    saving: bool,
    error: Option<String>,
}

#[cfg(target_os = "macos")]
impl FastCheckout {
    pub(super) fn load() -> Self {
        Self { enabled: bruhs_core::git::checkout_cache::enabled(), saving: false, error: None }
    }
}

impl Workspace {
    #[cfg(target_os = "macos")]
    fn toggle_fast_checkout(&mut self, cx: &mut Context<'_, Self>) {
        if self.fast_checkout.saving { return; }
        let enabled = !self.fast_checkout.enabled;
        self.fast_checkout.saving = true;
        self.fast_checkout.error = None;
        let save = cx.background_spawn(async move {
            bruhs_core::git::checkout_cache::set_enabled(enabled)
        });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                this.fast_checkout.saving = false;
                match result {
                    Ok(()) => this.fast_checkout.enabled = enabled,
                    Err(error) => {
                        this.fast_checkout.enabled = bruhs_core::git::checkout_cache::enabled();
                        this.fast_checkout.error = Some(format!("Could not change fast checkout. {error}"));
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn labs_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let mut rows = vec![self.settings_row(
            "labs-sidebar-folders",
            "Boring sidebar (group by project)",
            "Group active threads into project folders. Pinned threads stay above the folders.",
            settings_ui::switch(
                "labs-enable-folders",
                self.labs_sidebar.enabled,
                false,
                "Group sidebar by project",
                cx,
                |this, _, cx| {
                    this.labs_sidebar.enabled = !this.labs_sidebar.enabled;
                    this.persist_and_notify(cx);
                },
            ),
        )];
        if self.labs_sidebar.enabled {
            rows.push(self.settings_row(
                "labs-sidebar-two-lines",
                "Two-line folder rows",
                "Show branch, pull request and agent below the thread title.",
                settings_ui::switch(
                    "labs-enable-two-lines",
                    self.labs_sidebar.two_lines,
                    false,
                    "Two-line folder rows",
                    cx,
                    |this, _, cx| {
                        this.labs_sidebar.two_lines = !this.labs_sidebar.two_lines;
                        this.persist_and_notify(cx);
                    },
                ),
            ));
        }
        rows.push(self.settings_row(
            "labs-suggest-next",
            "Suggest next prompt",
            bruhs_models::logic::prompt_suggestion::SUGGESTION_HELP,
            settings_ui::switch(
                "labs-enable-suggestions",
                self.task_models.prompt_suggestions,
                false,
                "Suggest next prompt",
                cx,
                |this, _, cx| {
                    this.set_task_models(cx, |models| {
                        models.prompt_suggestions = !models.prompt_suggestions
                    });
                },
            ),
        ));
        #[cfg(target_os = "macos")]
        rows.push(self.settings_row(
            "labs-worktree-fast-checkout",
            "Fast worktree checkout",
            self.fast_checkout.error.clone().unwrap_or_else(||
                "Clone new worktrees from a cached checkout on this Mac. Git checks out files normally when cloning is unavailable. Turning this off deletes the cache.".to_owned()
            ),
            settings_ui::switch(
                "labs-toggle-fast-checkout",
                self.fast_checkout.enabled,
                self.fast_checkout.saving,
                "Fast worktree checkout",
                cx,
                |this, _, cx| this.toggle_fast_checkout(cx),
            ),
        ));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::Workspace;
    #[gpui::test]
    fn collapsing_a_checkout_keeps_other_folders_visible_and_search_reveals_matches(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |w, window, cx| {
            w.labs_sidebar.enabled = true;
            w.labs_sidebar.two_lines = true;
            let t = Workspace::create_thread_with_title(
                "Alpha".into(),
                "/other/alpha".into(),
                w.current_model.clone(),
                w.permission_mode,
                "Second alpha",
                window,
                cx,
            );
            w.threads.push(t);
            cx.notify();
        });
        cx.run_until_parked();
        for selector in ["desktop-sidebar-folder-/tmp/alpha", "desktop-sidebar-folder-/other/alpha"] {
            assert!(cx.debug_bounds(selector).is_some());
        }
        let button = cx
            .debug_bounds("desktop-sidebar-folder-/tmp/alpha")
            .unwrap();
        cx.simulate_click(button.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        workspace.update_in(cx, |w, window, cx| {
            assert!(w.snapshot(cx).labs_sidebar.collapsed.contains("/tmp/alpha"));
            assert!(!w.labs_sidebar.collapsed.contains("/other/alpha"));
            w.sidebar_search
                .update(cx, |input, cx| input.set_value("Alpha session", window, cx));
            cx.notify();
        });
        cx.run_until_parked();
        workspace.update_in(cx, |w, _, cx| {
            let (_, id) = w
                .threads
                .iter()
                .enumerate()
                .find(|(_, t)| t.project_path == "/tmp/alpha")
                .unwrap();
            assert!(w
                .sidebar_viewport
                .borrow()
                .row_targets(id.chat.entity_id())
                .is_some());
            assert!(w.labs_sidebar.collapsed.contains("/tmp/alpha"));
            assert!(w.sidebar_search.read(cx).value().contains("Alpha"));
        });
    }
}
