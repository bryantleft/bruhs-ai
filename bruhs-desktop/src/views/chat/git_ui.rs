//! The composer's branch chip menu: loading branches off the UI thread and switching.

use super::*;
use bruhs_models::logic::run_place::WorkspaceChoice;
pub(super) const GIT_BRANCH_EMPTY_LABEL: &str = "No branches";
pub(super) const GIT_BRANCH_CURRENT_MARK: &str = "●";
pub(super) const GIT_BRANCH_MENU_MIN_WIDTH_PX: f32 = 200.0;
pub(super) const GIT_BRANCH_MENU_MAX_HEIGHT_PX: f32 = 256.0;
pub(super) const GIT_BRANCH_EMPTY_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const GIT_BRANCH_CURRENT_MARK_WIDTH_PX: f32 = 12.0;
pub(super) const GIT_BRANCH_MENU_ROW_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const GIT_BRANCH_SWITCH_LABEL: &str = "Switch git branch";
pub(super) const GIT_BRANCH_MENU_RADIUS_PX: f32 = 8.0;

impl ChatView {
    pub(super) fn clear_git_info(&mut self) {
        self.git_info_stale = false;
        self.git_task = None;
        self.git_current_branch = None;
        self.git_branches.clear();
        self.branch_menu_open = false;
    }

    /// Re-read branches and status in the background; the chip and menu
    /// update when the answer arrives.
    pub(super) fn refresh_git_branches(&mut self, cx: &mut Context<'_, Self>) {
        if !self.project_scoped {
            self.clear_git_info();
            return;
        }
        self.git_info_stale = false;
        let load = spawn_tokio(
            cx,
            git_info::load_git_info(PathBuf::from(&self.project_path)),
        );
        self.git_task = Some(cx.spawn(async move |this, cx| {
            let info = load.await;
            // The chat may have closed meanwhile; nothing to show then.
            let _closed = this.update(cx, |this, cx| {
                this.apply_git_info(info);
                cx.notify();
            });
        }));
    }

    pub(super) fn apply_git_info(&mut self, info: git_info::GitInfo) {
        self.git_current_branch = info.current;
        self.git_branches = info.branches;
        self.git_dirty_count = info.dirty;
        if self.git_current_branch.is_none() {
            self.branch_menu_open = false;
        }
    }

    /// The branch the menu marks. A new worktree that has not started marks
    /// the chosen base branch instead of the checkout.
    pub(super) fn branch_menu_mark(&self) -> Option<String> {
        let started = self.thread_has_place();
        if !started
            && self.run_choice == WorkspaceChoice::NewWorktree
            && !self.base_branch.is_empty()
        {
            return Some(self.base_branch.clone());
        }
        self.git_current_branch.clone()
    }

    /// Switch to `branch` through bruhs-core (validated, `git switch --`, so
    /// a name can never act as a pathspec or option), off the UI thread.
    ///
    /// Before the thread has a place, New worktree stores a base branch and
    /// Previous worktree leaves the checkout alone.
    pub(super) fn checkout_branch(&mut self, branch: &str, cx: &mut Context<'_, Self>) {
        self.branch_menu_open = false;
        let started = self.thread_has_place();
        if !started && self.run_choice == WorkspaceChoice::NewWorktree {
            self.base_branch = branch.to_string();
            self.emit_run_prefs(cx);
            cx.notify();
            return;
        }
        if !started && self.run_choice == WorkspaceChoice::PreviousWorktree {
            cx.notify();
            return;
        }
        self.switch_checkout(branch, cx);
    }

    fn switch_checkout(&mut self, branch: &str, cx: &mut Context<'_, Self>) {
        let dir = PathBuf::from(&self.project_path);
        let name = branch.to_string();
        let switch = spawn_tokio(cx, async move {
            let result = bruhs_core::git::checkout(&dir, &name).await;
            (name, result)
        });
        self.checkout_task = Some(cx.spawn(async move |this, cx| {
            let (name, result) = switch.await;
            let _closed = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    tracing::warn!("switching to branch {name:?} failed: {error}");
                }
                this.refresh_git_branches(cx);
            });
        }));
        cx.notify();
    }

    pub(super) fn render_branch_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let marked = self.branch_menu_mark();
        let rows = if self.git_branches.is_empty() {
            vec![div()
                .px_3()
                .py_2()
                .text_size(px(GIT_BRANCH_EMPTY_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .child(GIT_BRANCH_EMPTY_LABEL)
                .into_any_element()]
        } else {
            self.git_branches
                .iter()
                .map(|branch| {
                    let branch_for_click = branch.clone();
                    let branch_for_key = branch.clone();
                    let debug_branch = branch.clone();
                    let is_current = marked.as_deref() == Some(branch.as_str());
                    div()
                        .id(("desktop-git-branch-row", element_key(branch)))
                        .debug_selector(move || {
                            format!("desktop-git-branch-row-{}", element_key(&debug_branch))
                        })
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .text_size(px(GIT_BRANCH_MENU_ROW_TEXT_SIZE_PX))
                        .text_color(if is_current {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_300()
                        })
                        .tab_index(0)
                        .role(Role::ListBoxOption)
                        .aria_label(branch.clone())
                        .aria_selected(is_current)
                        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::longan_800()))
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            cx.stop_propagation();
                            this.checkout_branch(&branch_for_click, cx);
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.checkout_branch(&branch_for_key, cx);
                            }
                        }))
                        .child(
                            div()
                                .w(px(GIT_BRANCH_CURRENT_MARK_WIDTH_PX))
                                .flex_shrink_0()
                                .text_color(theme::rambutan_500())
                                .child(if is_current {
                                    GIT_BRANCH_CURRENT_MARK
                                } else {
                                    ""
                                }),
                        )
                        .child(div().min_w_0().truncate().child(branch.clone()))
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        };

        div()
            .id("desktop-git-branch-menu")
            .debug_selector(|| "desktop-git-branch-menu".to_string())
            .role(Role::ListBox)
            .aria_label(GIT_BRANCH_SWITCH_LABEL)
            .w(px(GIT_BRANCH_MENU_MIN_WIDTH_PX))
            .max_h(px(GIT_BRANCH_MENU_MAX_HEIGHT_PX))
            .overflow_y_scrollbar()
            .rounded(px(GIT_BRANCH_MENU_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_700())
            .py_1()
            .children(rows)
            .into_any_element()
    }
}
