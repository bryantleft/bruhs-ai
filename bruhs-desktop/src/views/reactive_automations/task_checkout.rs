//! Checkout controls for a scheduled task. Branches come from the selected host.
use super::*;
use bruhs_ui::dropdown::{dropdown, DropdownGroup, DropdownOption, Listbox};

const OPTIONS: [(&str, &str); 3] = [
    ("worktree", "Create a new worktree"),
    ("project", "Use the project checkout"),
    ("existing", "Use a specific checkout"),
];

pub(super) struct CheckoutEditor {
    selected: &'static str,
    open: bool,
    base: Entity<InputState>,
    path: Entity<InputState>,
    remote: bool,
    search: Entity<InputState>,
    branches: Option<BranchMenu>,
    _search_changes: gpui::Subscription,
}

struct BranchMenu {
    project: String,
    request: uuid::Uuid,
    choices: Option<Result<Vec<String>, String>>,
}

impl CheckoutEditor {
    pub(super) fn new(
        saved: &TaskCheckout,
        window: &mut Window,
        cx: &mut Context<'_, ReactiveAutomations>,
    ) -> Self {
        let (selected, path, base, remote) = match saved {
            TaskCheckout::Project => ("project", "", "main", true),
            TaskCheckout::Existing { path } => ("existing", path.as_str(), "main", true),
            TaskCheckout::Worktree {
                base_branch,
                start_from_remote,
            } => ("worktree", "", base_branch.as_str(), *start_from_remote),
        };
        let search = edit_input("", window, cx);
        let changes = cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            selected,
            open: false,
            base: edit_input(base, window, cx),
            path: edit_input(path, window, cx),
            remote,
            search,
            branches: None,
            _search_changes: changes,
        }
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.branches = None;
    }

    pub(super) fn select(&mut self, selected: &'static str) {
        self.selected = selected;
        self.close();
    }

    pub(super) fn build(&self, cx: &App) -> Result<TaskCheckout, String> {
        let checkout = match self.selected {
            "existing" => TaskCheckout::Existing {
                path: value(&self.path, cx).trim().into(),
            },
            "worktree" => TaskCheckout::Worktree {
                base_branch: value(&self.base, cx).trim().into(),
                start_from_remote: self.remote,
            },
            _ => TaskCheckout::Project,
        };
        checkout.validate()?;
        Ok(checkout)
    }
}

impl ReactiveAutomations {
    fn open_task_branches(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Editor::Automation(editor)) = &mut self.editor else {
            return;
        };
        let project = editor.project_path(cx);
        if editor
            .checkout
            .branches
            .take()
            .is_some_and(|menu| menu.project == project)
        {
            cx.notify();
            return;
        }
        editor
            .checkout
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        let request = uuid::Uuid::new_v4();
        editor.checkout.branches = Some(BranchMenu {
            project: project.clone(),
            request,
            choices: None,
        });
        let client = self.client.clone();
        let result = spawn_tokio(cx, async move { client.task_base_branches(&project).await });
        cx.spawn(async move |this, cx| {
            let choices = result
                .await
                .map(|response| response.branches)
                .map_err(reactive_error);
            let _ = this.update(cx, |this, cx| {
                if let Some(Editor::Automation(editor)) = &mut this.editor {
                    if let Some(menu) = editor
                        .checkout
                        .branches
                        .as_mut()
                        .filter(|menu| menu.request == request)
                    {
                        menu.choices = Some(choices);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_task_checkout(
        &self,
        editor: &setup::AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let draft = &editor.checkout;
        let toggle = cx.weak_entity();
        let select = toggle.clone();
        let picker = dropdown(
            "task-checkout",
            "Task checkout",
            draft.selected,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options: OPTIONS
                    .iter()
                    .map(|(value, label)| DropdownOption {
                        value,
                        label: (*label).into(),
                        description: None,
                    })
                    .collect(),
            }],
            if draft.open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = toggle.update(cx, |this, cx| {
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.checkout.open = !editor.checkout.open;
                        editor.checkout.branches = None;
                        editor.security.encoding_open = false;
                        editor.task_thread_open = false;
                        editor.picker.open = None;
                        editor.open_filter = None;
                    }
                    cx.notify();
                });
            }),
            Rc::new(move |value, _, cx| {
                let _ = select.update(cx, |this, cx| {
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.checkout.select(value);
                    }
                    cx.notify();
                });
            }),
        );
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(muted("Checkout"))
            .child(picker);
        match draft.selected {
            "existing" => body = body.child(field("Checkout path", &draft.path, false, self.busy)),
            "worktree" => {
                let choose = ghost_control("task-choose-base", "Choose base branch", self.busy)
                    .aria_expanded(draft.branches.is_some())
                    .on_activate(cx, |this, window, cx| this.open_task_branches(window, cx));
                body = body.child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .child(field("Base branch", &draft.base, false, self.busy))
                        .child(choose),
                );
                if let Some(menu) = draft
                    .branches
                    .as_ref()
                    .filter(|menu| menu.project == editor.project_path(cx))
                {
                    body = body.child(field(
                        "Search base branches",
                        &draft.search,
                        false,
                        self.busy,
                    ));
                    let list = div()
                        .id("task-base-branches")
                        .max_h(px(160.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col();
                    let list = match &menu.choices {
                        None => list.child(muted("Loading branches…")),
                        Some(Err(error)) => list.child(muted(error.clone())),
                        Some(Ok(choices)) => {
                            let query = value(&draft.search, cx).to_lowercase();
                            let mut matches = choices
                                .iter()
                                .filter(|branch| branch.to_lowercase().contains(&query))
                                .peekable();
                            if matches.peek().is_none() {
                                list.child(muted("No matching branches"))
                            } else {
                                list.children(matches.map(|branch| {
                                    let name = branch.clone();
                                    ghost_control(
                                        SharedString::from(format!("task-base-{name}")),
                                        branch,
                                        self.busy,
                                    )
                                    .on_activate(
                                        cx,
                                        move |this, window, cx| {
                                            if let Some(Editor::Automation(editor)) =
                                                &mut this.editor
                                            {
                                                editor.checkout.base.update(cx, |input, cx| {
                                                    input.set_value(name.clone(), window, cx)
                                                });
                                                editor.checkout.branches = None;
                                            }
                                            cx.notify();
                                        },
                                    )
                                }))
                            }
                        }
                    };
                    body = body.child(list);
                }
                let remote = bruhs_ui::switch::switch(
                    "task-start-from-remote",
                    "Start worktree from remote",
                    draft.remote,
                    bruhs_ui::press::Interactivity::disabled_if(self.busy),
                )
                .on_activate(cx, |this, _, cx| {
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.checkout.remote = !editor.checkout.remote;
                    }
                    cx.notify();
                });
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(muted("Start from remote"))
                        .child(remote),
                );
            }
            _ => {}
        }
        body
    }
}
