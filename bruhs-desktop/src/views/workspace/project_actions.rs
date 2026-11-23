//! Commands explicitly saved in Settings and run in a visible project terminal.
use super::*;
use crate::views::terminal_panel::ProjectCommand;
use bruhs_models::logic::keybindings::{chord_claimed, parse_chord, Chord};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Lifecycle {
    run_on_worktree_create: bool,
    wait_for_setup: bool,
    run_on_settle: bool,
    stop_on_settle: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProjectAction {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview_url: Option<String>,
    #[serde(default)]
    auto_open_preview: bool,
    #[serde(flatten)]
    lifecycle: Lifecycle,
}

pub(super) struct ActionEditor {
    pub open: bool,
    pub(super) art_input: Entity<InputState>,
    pub(super) art_open: bool,
    pub(super) art_error: Option<String>,
    id: Option<String>,
    name: Entity<InputState>,
    command: Entity<InputState>,
    shortcut: Entity<InputState>,
    preview_url: Entity<InputState>,
    auto_open_preview: bool,
    lifecycle: Lifecycle,
    error: Option<String>,
}

impl ActionEditor {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            open: false,
            shortcut: cx.new(|cx| InputState::new(window, cx).placeholder("e.g. mod+alt+r")),
            preview_url: cx
                .new(|cx| InputState::new(window, cx).placeholder("http://localhost:3000")),
            auto_open_preview: false,
            lifecycle: Default::default(),
            art_input: cx.new(|cx| InputState::new(window, cx).placeholder("Emoji or symbol")),
            art_open: false,
            art_error: None,
            id: None,
            error: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Action name")),
            command: cx.new(|cx| InputState::new(window, cx).placeholder("Shell command")),
        }
    }
}

impl Workspace {
    fn edit_project_action(
        &mut self,
        action: Option<ProjectAction>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.action_editor.id = action.as_ref().map(|a| a.id.clone());
        self.action_editor.name.update(cx, |field, cx| {
            field.set_value(action.as_ref().map_or("", |a| a.name.as_str()), window, cx);
            field.focus(window, cx);
        });
        self.action_editor.command.update(cx, |field, cx| {
            field.set_value(
                action.as_ref().map_or("", |a| a.command.as_str()),
                window,
                cx,
            )
        });
        self.action_editor.shortcut.update(cx, |input, cx| {
            input.set_value(
                action
                    .as_ref()
                    .and_then(|a| a.shortcut.as_deref())
                    .unwrap_or(""),
                window,
                cx,
            )
        });
        self.action_editor.preview_url.update(cx, |input, cx| {
            input.set_value(
                action
                    .as_ref()
                    .and_then(|a| a.preview_url.as_deref())
                    .unwrap_or(""),
                window,
                cx,
            )
        });
        self.action_editor.auto_open_preview = action.as_ref().is_some_and(|a| a.auto_open_preview);
        self.action_editor.lifecycle = action.as_ref().map(|a| a.lifecycle).unwrap_or_default();
        self.settings_scroll.set_offset(Default::default());
        self.action_editor.open = true;
        self.action_editor.error = None;
        cx.notify();
    }

    fn save_project_action(&mut self, cx: &mut Context<'_, Self>) {
        let name = self.action_editor.name.read(cx).value().trim().to_owned();
        let command = self
            .action_editor
            .command
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if name.is_empty() || command.is_empty() || command.chars().any(char::is_control) {
            self.action_editor.error = Some("Enter a name and a single-line shell command.".into());
            cx.notify();
            return;
        }
        let shortcut = self
            .action_editor
            .shortcut
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let shortcut = (!shortcut.is_empty()).then_some(shortcut);
        if let Some(raw) = shortcut.as_deref() {
            let mac = cfg!(target_os = "macos");
            let chord = parse_chord(raw, mac);
            let valid = chord
                .as_ref()
                .is_some_and(|c| c.command || c.ctrl || c.alt || is_function_key(&c.key));
            if !valid {
                self.action_editor.error =
                    Some("Use a function key or a shortcut with Command, Control or Alt.".into());
                cx.notify();
                return;
            }
            if let Some(c) = chord {
                let conflict = chord_claimed(&self.keybindings, &c, mac)
                    || self
                        .project_actions
                        .iter()
                        .filter(|(scope, _)| {
                            self.settings_project_scope.is_empty()
                                || scope.is_empty()
                                || **scope == self.settings_project_scope
                        })
                        .flat_map(|(_, actions)| actions)
                        .any(|a| {
                            Some(&a.id) != self.action_editor.id.as_ref()
                                && a.shortcut
                                    .as_deref()
                                    .and_then(|s| parse_chord(s, mac))
                                    .as_ref()
                                    == Some(&c)
                        });
                if conflict {
                    self.action_editor.error =
                        Some("That shortcut is already assigned here.".into());
                    cx.notify();
                    return;
                }
            }
        }
        let raw = self
            .action_editor
            .preview_url
            .read(cx)
            .value()
            .trim()
            .to_string();
        let preview_url = (!raw.is_empty()).then_some(raw);
        if preview_url
            .as_deref()
            .is_some_and(|raw| !valid_preview_url(raw))
        {
            self.action_editor.error = Some("Enter a full HTTP or HTTPS preview URL.".into());
            cx.notify();
            return;
        }
        let action = ProjectAction {
            id: self
                .action_editor
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            command,
            shortcut,
            lifecycle: self.action_editor.lifecycle,
            auto_open_preview: preview_url.is_some() && self.action_editor.auto_open_preview,
            preview_url,
        };
        let actions = self
            .project_actions
            .entry(self.settings_project_scope.clone())
            .or_default();
        if let Some(index) = actions.iter().position(|a| a.id == action.id) {
            actions[index] = action;
        } else {
            actions.push(action);
        }
        self.action_editor.open = false;
        self.idle_action_runs = None;
        self.new_idle_action_threads(cx);
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    fn action_checkout(&self, cx: &App) -> Option<String> {
        self.checkout_for_action_scope(&self.settings_project_scope, cx)
    }
    fn checkout_for_action_scope(&self, scope: &str, cx: &App) -> Option<String> {
        if let Some(thread) = self.active_thread() {
            let chat = thread.chat.read(cx);
            if chat.project_scoped
                && (scope.is_empty()
                    || scope == chat.settings_project_path()
                    || scope == chat.project_path)
            {
                return Some(
                    chat.worktree_path
                        .as_ref()
                        .unwrap_or(&thread.project_path)
                        .clone(),
                );
            }
        }
        if !scope.is_empty() {
            return Some(scope.to_owned());
        }
        self.active_project.as_ref().map(|p| p.path.clone())
    }

    fn run_project_action(
        &mut self,
        action: &ProjectAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(cwd) = self.action_checkout(cx) else {
            return;
        };
        self.run_action_in(action, cwd, window, cx);
    }

    pub(super) fn shortcut_project_action(
        &self,
        chord: &Chord,
        cx: &App,
    ) -> Option<(ProjectAction, String)> {
        if self.project_actions.is_empty() {
            return None;
        }
        let scope = self.active_settings_project(cx)?;
        let cwd = self.checkout_for_action_scope(&scope, cx)?;
        self.project_actions
            .get(&scope)
            .into_iter()
            .flatten()
            .chain(self.project_actions.get("").into_iter().flatten())
            .find(|a| {
                a.shortcut
                    .as_deref()
                    .and_then(|s| parse_chord(s, cfg!(target_os = "macos")))
                    .as_ref()
                    == Some(chord)
            })
            .map(|a| (a.clone(), cwd))
    }

    pub(super) fn show_action_preview(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !valid_preview_url(url) {
            return;
        }
        self.close_transient_surfaces();
        if crate::views::preview_panel::WebViewHost::available() {
            self.open_chat_view(&OpenChatView, window, cx);
            self.set_preview_panel_visible(true, cx);
            self.preview_panel
                .update(cx, |panel, cx| panel.go_to(url, window, cx));
        } else {
            cx.open_url(url);
        }
    }

    pub(super) fn run_action_in(
        &mut self,
        action: &ProjectAction,
        cwd: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let project = Path::new(&cwd)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.close_transient_surfaces();
        self.active_view = AppView::Terminal;
        self.sync_terminal_context(cx);
        let owner = self
            .active_thread()
            .filter(|t| {
                let chat = t.chat.read(cx);
                chat.project_scoped
                    && (chat.project_path == cwd
                        || chat.worktree_path.as_deref() == Some(cwd.as_str()))
            })
            .map(|t| t.id.clone());
        self.terminal_panel.update(cx, |panel, cx| {
            panel.open_with_command(
                Some(cwd),
                project,
                Some(ProjectCommand {
                    name: action.name.clone(),
                    command: action.command.clone(),
                    preview: owner
                        .clone()
                        .zip(action.preview_url.clone())
                        .filter(|(_, url)| valid_preview_url(url)),
                    stop_on_settle_for: owner.filter(|_| action.lifecycle.stop_on_settle),
                }),
                !action.auto_open_preview,
                window,
                cx,
            )
        });
        if action.auto_open_preview {
            if let Some(url) = action.preview_url.as_deref() {
                self.show_action_preview(url, window, cx);
            }
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn actions_for_project(&self, scope: &str) -> impl Iterator<Item = &ProjectAction> {
        self.project_actions.get("").into_iter().flatten().chain(
            (!scope.is_empty())
                .then(|| self.project_actions.get(scope))
                .flatten()
                .into_iter()
                .flatten(),
        )
    }

    pub(super) fn setup_steps(&self, scope: &str) -> Vec<bruhs_core::git::SetupAction> {
        self.actions_for_project(scope)
            .filter(|a| a.lifecycle.run_on_worktree_create && a.lifecycle.wait_for_setup)
            .map(|a| bruhs_core::git::SetupAction {
                id: a.id.clone(),
                name: a.name.clone(),
                command: a.command.clone(),
            })
            .collect()
    }

    /// Observe the same transition the sidebar displays. Opening the app or merely viewing
    /// an already-settled conversation must not launch commands again.
    pub(super) fn new_idle_action_threads(&mut self, cx: &App) -> Vec<String> {
        if !self
            .project_actions
            .values()
            .flatten()
            .any(|a| a.lifecycle.run_on_settle || a.lifecycle.stop_on_settle)
        {
            self.idle_action_runs = None;
            return Vec::new();
        }
        let prior = self.idle_action_runs.take();
        let mut next = BTreeMap::new();
        let mut newly = Vec::new();
        for thread in &self.threads {
            let chat = thread.chat.read(cx);
            if !chat.project_scoped
                || !self
                    .actions_for_project(chat.settings_project_path())
                    .any(|a| a.lifecycle.run_on_settle || a.lifecycle.stop_on_settle)
            {
                continue;
            }
            let handled =
                prior.as_ref().and_then(|r| r.get(&thread.id)) == Some(&thread.updated_at_ms);
            if handled || self.section_for_thread(thread, cx) == SidebarSection::Settled {
                next.insert(thread.id.clone(), thread.updated_at_ms);
                if prior.is_some() && !handled && !thread.settled {
                    newly.push(thread.id.clone());
                }
            }
        }
        self.idle_action_runs = Some(next);
        newly
    }

    pub(super) fn launch_thread_hooks(&self, id: &str, settle: bool, cx: &mut Context<'_, Self>) {
        if settle {
            self.terminal_panel
                .update(cx, |panel, cx| panel.stop_thread_actions(id, cx));
        }
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let chat = thread.chat.read(cx);
        if !chat.project_scoped {
            return;
        }
        let cwd = chat
            .worktree_path
            .as_ref()
            .unwrap_or(&chat.project_path)
            .clone();
        let commands: Vec<_> = self
            .actions_for_project(chat.settings_project_path())
            .filter(|a| {
                if settle {
                    a.lifecycle.run_on_settle
                } else {
                    a.lifecycle.run_on_worktree_create && !a.lifecycle.wait_for_setup
                }
            })
            .map(|a| ProjectCommand {
                name: a.name.clone(),
                command: a.command.clone(),
                preview: a
                    .preview_url
                    .clone()
                    .filter(|url| valid_preview_url(url))
                    .map(|url| (id.to_owned(), url)),
                stop_on_settle_for: (!settle && a.lifecycle.stop_on_settle).then(|| id.to_owned()),
            })
            .collect();
        if commands.is_empty() {
            return;
        }
        let panel = self.terminal_panel.clone();
        let window = self.workflow_window;
        let project = chat.project_name.clone();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                panel.update(cx, |panel, cx| {
                    for command in commands {
                        panel.open_with_command(
                            Some(cwd.clone()),
                            Some(project.clone()),
                            Some(command),
                            false,
                            window,
                            cx,
                        );
                    }
                });
            });
        });
    }

    fn render_project_action_editor(&self, cx: &Context<'_, Self>) -> AnyElement {
        let editor = &self.action_editor;
        let field = |label: &'static str, aria: &'static str, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(label),
                )
                .child(Input::new(input).aria_label(aria))
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap_2()
            .child(self.settings_heading(
                if editor.id.is_some() {
                    "Edit action"
                } else {
                    "Add action"
                },
                "",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(on_press_view(
                        web_button(
                            "settings-project-action-cancel",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("Cancel project action changes")
                        .child("Cancel"),
                        cx,
                        |this, _, cx| {
                            this.action_editor.open = false;
                            cx.notify();
                        },
                    ))
                    .child(on_press_view(
                        web_button(
                            "settings-project-action-save",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("Save project action")
                        .child("Save"),
                        cx,
                        |this, _, cx| this.save_project_action(cx),
                    )),
            );
        let mut form = div()
            .w_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(field("Name", "Action name", &editor.name))
            .child(field("Shell command", "Shell command", &editor.command))
            .child(field(
                "Keyboard shortcut",
                "Action shortcut",
                &editor.shortcut,
            ))
            .child(field("Preview URL", "Preview URL", &editor.preview_url));
        let life = editor.lifecycle;
        type Change = fn(&mut ActionEditor);
        for (id, title, state, disabled, toggle) in [
            (
                "action-auto-preview",
                "Open preview when run",
                editor.auto_open_preview,
                false,
                (|e: &mut ActionEditor| e.auto_open_preview = !e.auto_open_preview) as Change,
            ),
            (
                "action-create",
                "Run on worktree creation",
                life.run_on_worktree_create,
                false,
                |e: &mut ActionEditor| {
                    e.lifecycle.run_on_worktree_create = !e.lifecycle.run_on_worktree_create;
                    if !e.lifecycle.run_on_worktree_create {
                        e.lifecycle.wait_for_setup = false;
                    }
                },
            ),
            (
                "action-wait",
                "Wait before starting the agent",
                life.wait_for_setup,
                !life.run_on_worktree_create,
                |e: &mut ActionEditor| e.lifecycle.wait_for_setup = !e.lifecycle.wait_for_setup,
            ),
            (
                "action-settle",
                "Run when thread settles",
                life.run_on_settle,
                false,
                |e: &mut ActionEditor| e.lifecycle.run_on_settle = !e.lifecycle.run_on_settle,
            ),
            (
                "action-stop",
                "Stop when thread settles",
                life.stop_on_settle,
                false,
                |e: &mut ActionEditor| e.lifecycle.stop_on_settle = !e.lifecycle.stop_on_settle,
            ),
        ] {
            form = form.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(title)
                    .child(settings_ui::switch(
                        id,
                        state,
                        disabled,
                        title,
                        cx,
                        move |this, _, cx| {
                            toggle(&mut this.action_editor);
                            cx.notify();
                        },
                    )),
            );
        }
        div()
            .id("settings-project-action-editor")
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .when_some(editor.error.clone(), |el, e| {
                el.child(div().id("project-action-error").role(Role::Alert).child(e))
            })
            .child(
                form.rounded(px(14.))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(theme::longan_800()),
            )
            .into_any_element()
    }

    pub(super) fn render_project_actions(&self, cx: &Context<'_, Self>) -> AnyElement {
        if let Some(view) = self.project_inventory_view(cx) {
            return view;
        }
        let scope = &self.settings_project_scope;
        let target = self.action_checkout(cx);
        let button = |id: SharedString, label: &'static str, disabled: bool| {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(disabled),
            )
            .aria_label(label)
            .child(label)
        };
        if self.action_editor.open {
            return self.render_project_action_editor(cx);
        }
        let mut rows = Vec::new();
        let inherited = (!scope.is_empty())
            .then(|| self.project_actions.get(""))
            .flatten();
        for (action, local) in self
            .project_actions
            .get(scope)
            .into_iter()
            .flatten()
            .map(|a| (a, true))
            .chain(inherited.into_iter().flatten().map(|a| (a, false)))
        {
            let action_to_run = action.clone();
            let mut actions = div().flex().flex_wrap().gap_2().child(on_press_view(
                button(format!("run-{}", action.id).into(), "Run", target.is_none())
                    .aria_label(format!("Run {}", action.name)),
                cx,
                move |this, window, cx| this.run_project_action(&action_to_run, window, cx),
            ));
            if let Some(url) = action
                .preview_url
                .as_ref()
                .filter(|url| valid_preview_url(url))
            {
                let url = url.clone();
                actions = actions.child(on_press_view(
                    button(format!("preview-{}", action.id).into(), "Preview", false)
                        .aria_label(format!("Preview {}", action.name)),
                    cx,
                    move |this, window, cx| this.show_action_preview(&url, window, cx),
                ));
            }
            if let Some(shortcut) = &action.shortcut {
                actions = actions.child(
                    div()
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(shortcut.clone()),
                );
            }
            if local {
                let edit = action.clone();
                let id = action.id.clone();
                actions = actions
                    .child(on_press_view(
                        button(format!("edit-{}", id).into(), "Edit", false)
                            .aria_label(format!("Edit {}", action.name)),
                        cx,
                        move |this, window, cx| {
                            this.edit_project_action(Some(edit.clone()), window, cx)
                        },
                    ))
                    .child(on_press_view(
                        button(format!("remove-{}", id).into(), "Remove", false)
                            .aria_label(format!("Remove {}", action.name)),
                        cx,
                        move |this, _, cx| {
                            if let Some(actions) =
                                this.project_actions.get_mut(&this.settings_project_scope)
                            {
                                actions.retain(|a| a.id != id);
                                if actions.is_empty() {
                                    this.project_actions.remove(&this.settings_project_scope);
                                }
                            }
                            this.stamp_run_prefs_on_threads(cx);
                            this.persist_and_notify(cx);
                        },
                    ));
            }
            rows.push(self.settings_row(
                role_settings_ui::project_value(&format!("settings-project-action-{}", action.id)),
                action.name.clone(),
                if local {
                    action.command.clone()
                } else {
                    format!("Environment action · {}", action.command)
                },
                actions.into_any_element(),
            ));
        }
        rows.push(
            self.settings_row(
                "settings-project-action-add",
                "Add action",
                "Save a command to run in a project checkout or worktree.",
                on_press_view(
                    button(
                        "settings-project-action-add-button".into(),
                        "Add action",
                        false,
                    ),
                    cx,
                    |this, window, cx| this.edit_project_action(None, window, cx),
                )
                .into_any_element(),
            ),
        );
        let mut overview = vec![self.projects_scope_row(cx)];
        if !scope.is_empty() {
            overview.push(self.checkout_name_row(cx));
            overview.push(self.project_art_row(cx));
            overview.push(self.project_grouping_override_row(cx));
        }
        if let Some(path) = target {
            overview.push(
                self.settings_row(
                    "desktop-settings-project-folder",
                    "Checkout folder",
                    path.clone(),
                    on_press_view(
                        button("settings-project-terminal".into(), "Open terminal", false),
                        cx,
                        move |this, window, cx| {
                            this.close_transient_surfaces();
                            this.active_view = AppView::Terminal;
                            this.terminal_panel.update(cx, |panel, cx| {
                                panel.open(Some(path.clone()), None, window, cx)
                            });
                            this.persist_and_notify(cx);
                        },
                    )
                    .into_any_element(),
                ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(self.settings_card("Project overview", "", overview))
            .children(self.linked_checkout_cards(cx))
            .child(self.settings_card(
                "New threads",
                "",
                vec![
                    self.new_thread_model_row(cx),
                    self.new_thread_placement_row(cx),
                ],
            ))
            .child(self.settings_card("Actions", "", rows))
            .when(
                !scope.is_empty() && self.listed_projects().iter().any(|p| &p.path == scope),
                |view| {
                    let remove = self.remove_checkout_button(
                        "settings-remove-checkout".into(),
                        project_removal::Placement::Section,
                        project_removal::Target::Checkout(scope.to_owned()),
                        true,
                        cx,
                    );
                    view.child(self.settings_card(
                        "Danger",
                        "",
                        vec![self.settings_row(
                            "settings-remove-checkout-row",
                            "Remove checkout",
                            "Remove this checkout and its conversations. Files stay on disk.",
                            remove,
                        )],
                    ))
                },
            )
            .into_any_element()
    }
}

fn valid_preview_url(raw: &str) -> bool {
    bruhs_models::local_tools::validate_browser_url(raw).is_ok()
        && reqwest::Url::parse(raw)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}
fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|s| s.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

#[cfg(test)]
mod tests {
    use super::{OpenSettings, SettingsSection};
    #[gpui::test]
    fn idle_actions_follow_sidebar_transitions_once_per_activity(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, general, code) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |workspace, _, cx| {
            let action = serde_json::from_value(
                serde_json::json!({"id":"tidy","name":"Tidy","command":"pwd","run_on_settle":true}),
            )
            .unwrap();
            workspace
                .project_actions
                .insert("/tmp/alpha".into(), vec![action]);
            workspace.idle_settle_days = 1;
            workspace.threads[1].updated_at_ms = bruhs_models::EpochMs::UNIX_EPOCH;
            // Arrival in a running workspace never triggers an old command.
            assert!(workspace.new_idle_action_threads(cx).is_empty());
            workspace.threads[1].updated_at_ms = bruhs_models::EpochMs::now();
            assert!(workspace.new_idle_action_threads(cx).is_empty());
            workspace.threads[1].updated_at_ms = bruhs_models::EpochMs::new(1234).unwrap();
            workspace.unread_thread_ids.insert(code.clone());
            assert!(workspace.new_idle_action_threads(cx).is_empty());
            workspace.unread_thread_ids.remove(&code);
            assert_eq!(workspace.new_idle_action_threads(cx), vec![code.clone()]);
            workspace.active_thread_id = Some(code);
            assert!(workspace.new_idle_action_threads(cx).is_empty());
            workspace.active_thread_id = Some(general);
            assert!(workspace.new_idle_action_threads(cx).is_empty());
        });
    }

    #[gpui::test]
    fn project_actions_save_and_edit_in_the_selected_checkout(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        let project = tempfile::tempdir().unwrap();
        let path = project.path().to_string_lossy().to_string();
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_settings(&OpenSettings, window, cx);
            workspace.set_settings_section(SettingsSection::Projects, cx);
            workspace.settings_project_scope = path.clone();
            workspace.edit_project_action(None, window, cx);
            workspace
                .action_editor
                .name
                .update(cx, |f, cx| f.set_value("Verify checkout", window, cx));
            workspace
                .action_editor
                .command
                .update(cx, |f, cx| f.set_value("pwd > checkout.txt", window, cx));
            workspace
                .action_editor
                .shortcut
                .update(cx, |field, cx| field.set_value("mod+,", window, cx));
            workspace.save_project_action(cx);
            assert!(workspace
                .action_editor
                .error
                .as_deref()
                .unwrap()
                .contains("already assigned"));
            workspace
                .action_editor
                .shortcut
                .update(cx, |field, cx| field.set_value("ctrl+shift+k", window, cx));
            workspace
                .action_editor
                .preview_url
                .update(cx, |field, cx| field.set_value("file:///tmp", window, cx));
            workspace.save_project_action(cx);
            assert!(workspace
                .action_editor
                .error
                .as_deref()
                .unwrap()
                .contains("preview URL"));
            workspace.action_editor.preview_url.update(cx, |field, cx| {
                field.set_value("http://localhost:4173/preview", window, cx)
            });
            workspace.action_editor.auto_open_preview = true;
            workspace.save_project_action(cx);
            let saved = workspace.snapshot(cx);
            assert_eq!(saved.project_actions[&path].len(), 1);
            let action = saved.project_actions[&path][0].clone();
            assert_eq!(action.shortcut.as_deref(), Some("ctrl+shift+k"));
            assert_eq!(
                action.preview_url.as_deref(),
                Some("http://localhost:4173/preview")
            );
            assert!(action.auto_open_preview);
            assert_eq!(
                workspace.action_checkout(cx).as_deref(),
                Some(path.as_str())
            );
            workspace.edit_project_action(Some(action), window, cx);
            workspace
                .action_editor
                .command
                .update(cx, |f, cx| f.set_value("pwd", window, cx));
            workspace.save_project_action(cx);
            assert_eq!(
                workspace.snapshot(cx).project_actions[&path][0].command,
                "pwd"
            );
        });
    }
}
