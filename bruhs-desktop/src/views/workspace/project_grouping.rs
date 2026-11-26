//! Repository grouping is a display preference. Commands keep their physical checkout.
use super::*;
use bruhs_core::git::ProjectIdentity;
use futures_util::{stream, StreamExt};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    RepositoryPath,
    Separate,
    #[default]
    #[serde(other)]
    Repository,
}

impl Mode {
    fn choice(self) -> (&'static str, &'static str) {
        match self {
            Self::Repository => ("repository", "Group by repository"),
            Self::RepositoryPath => ("repository_path", "Group by repository path"),
            Self::Separate => ("separate", "Keep separate"),
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub mode: Mode,
    remembered: Mode,
    overrides: BTreeMap<String, Mode>,
}

impl Preferences {
    pub(super) fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub(super) fn forget_checkout(&mut self, path: &str) {
        self.overrides.remove(path);
    }
    pub(super) fn forget_host(&mut self, host: uuid::Uuid) {
        let prefix = format!("linked:{host}:");
        self.overrides
            .retain(|scope, _| !scope.starts_with(&prefix));
    }

    fn effective(&self, path: &str) -> Mode {
        self.overrides.get(path).copied().unwrap_or(self.mode)
    }

    fn toggle(&mut self) {
        if self.mode == Mode::Separate {
            self.mode = if self.remembered == Mode::RepositoryPath {
                Mode::RepositoryPath
            } else {
                Mode::Repository
            };
        } else {
            self.remembered = self.mode;
            self.mode = Mode::Separate;
        }
    }
}

#[derive(Default)]
pub(super) struct Identities {
    values: BTreeMap<String, ProjectIdentity>,
    checked: BTreeMap<String, Instant>,
}

#[derive(Clone, Debug)]
pub(super) struct Member {
    pub host: Option<uuid::Uuid>,
    pub environment: String,
    pub path: String,
    pub name: String,
    pub has_custom_name: bool,
}

impl Member {
    pub(super) fn scope(&self) -> String {
        match self.host {
            Some(id) => format!("linked:{id}:{}", self.path),
            None => self.path.clone(),
        }
    }
    pub(super) fn row_id(&self) -> String {
        format!("linked-checkout-{}", element_key(&self.scope()))
    }
}

pub(super) struct Group {
    pub key: String,
    pub members: Vec<Member>,
    pub label: String,
}

impl Workspace {
    pub(super) fn project_prompt_repository(
        &self,
        path: &str,
    ) -> Option<bruhs_models::logic::app_link::PromptRepository> {
        let identity = self.project_identities.values.get(path)?;
        bruhs_models::logic::app_link::PromptRepository::from_remote_identity(&identity.repository)
    }

    pub(super) fn refresh_project_identities(&mut self, cx: &mut Context<'_, Self>) {
        let now = Instant::now();
        let mut paths: BTreeSet<_> = self.listed_projects().into_iter().map(|p| p.path).collect();
        paths.extend(
            self.threads
                .iter()
                .map(|t| labs::folder(t, cx))
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
        );
        paths.retain(|path| {
            self.project_identities
                .checked
                .get(path)
                .is_none_or(|last| now.duration_since(*last) >= Duration::from_secs(30))
        });
        if paths.is_empty() {
            return;
        }
        for path in &paths {
            self.project_identities.checked.insert(path.clone(), now);
        }
        let read = crate::app_state::spawn_tokio(cx, async move {
            stream::iter(paths)
                .map(|path| async move {
                    let identity = bruhs_core::git::project_identity(Path::new(&path)).await;
                    (path, identity)
                })
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
        });
        cx.spawn(async move |this, cx| {
            let results = read.await;
            let _ = this.update(cx, |this, cx| {
                for (path, identity) in results {
                    if this.project_identities.checked.get(&path) != Some(&now) {
                        continue;
                    }
                    if let Some(identity) = identity {
                        this.project_identities.values.insert(path, identity);
                    } else {
                        this.project_identities.values.remove(&path);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn project_group_key(&self, path: &str) -> String {
        self.checkout_group_key(path, None, self.project_identities.values.get(path))
    }

    fn checkout_group_key(
        &self,
        scope: &str,
        host: Option<uuid::Uuid>,
        identity: Option<&ProjectIdentity>,
    ) -> String {
        let mode = self.project_list.grouping.effective(scope);
        if let Some(repo) = identity.filter(|_| mode != Mode::Separate) {
            let relative = if mode == Mode::RepositoryPath {
                repo.relative_path.as_str()
            } else {
                ""
            };
            // Include the length so repository URLs with punctuation cannot collide with subpaths.
            let namespace = host
                .filter(|_| !repo.repository.starts_with("remote:"))
                .map(|id| format!("linked:{id}:"))
                .unwrap_or_default();
            return format!(
                "{namespace}repo:{}:{}{}",
                repo.repository.len(),
                repo.repository,
                relative
            );
        }
        scope.to_owned()
    }

    pub(super) fn group_label(&self, path: &str) -> Option<String> {
        self.project_identities.values.get(path).map(|repo| {
            if self.project_list.grouping.effective(path) == Mode::RepositoryPath
                && !repo.relative_path.is_empty()
            {
                format!("{}/{}", repo.label, repo.relative_path)
            } else {
                repo.label.clone()
            }
        })
    }

    pub(super) fn grouped_projects(&self) -> Vec<Group> {
        self.checkout_groups(true)
    }

    pub(super) fn checkout_groups(&self, include_linked: bool) -> Vec<Group> {
        let mut indices = BTreeMap::<String, usize>::new();
        let mut groups = Vec::<Group>::new();
        let mut add = |member: Member, identity: Option<&ProjectIdentity>| {
            let scope = member.scope();
            let key = self.checkout_group_key(&scope, member.host, identity);
            if let Some(index) = indices.get(&key) {
                groups[*index].members.push(member);
                return;
            }
            let label = match identity {
                Some(repo)
                    if self.project_list.grouping.effective(&scope) == Mode::RepositoryPath
                        && !repo.relative_path.is_empty() =>
                {
                    format!("{}/{}", repo.label, repo.relative_path)
                }
                Some(repo) => repo.label.clone(),
                None => member.name.clone(),
            };
            indices.insert(key.clone(), groups.len());
            groups.push(Group {
                key,
                label,
                members: vec![member],
            });
        };
        for project in self.listed_projects() {
            let identity = self.project_identities.values.get(&project.path);
            let title = self.project_list.title(&project.path);
            add(
                Member {
                    host: None,
                    environment: "This machine".into(),
                    path: project.path,
                    name: project.name,
                    has_custom_name: title.is_some(),
                },
                identity,
            );
        }
        if include_linked {
            for (id, environment, checkout) in self.linked_projects.checkouts() {
                add(
                    Member {
                        host: Some(id),
                        environment,
                        path: checkout.path.clone(),
                        name: checkout.name.clone(),
                        has_custom_name: checkout.title.is_some(),
                    },
                    checkout.identity.as_ref(),
                );
            }
        }
        for group in &mut groups {
            if let Some(named) = group.members.iter().find(|p| p.has_custom_name) {
                group.label.clone_from(&named.name);
            }
        }
        groups
    }

    pub(super) fn project_grouping_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mode = self.project_list.grouping.mode;
        let label = self.ui_language().text("Project grouping");
        let toggle = settings_ui::switch(
            "settings-project-grouping",
            mode != Mode::Separate,
            false,
            label,
            cx,
            |this, _, cx| {
                this.project_list.grouping.toggle();
                this.persist_and_notify(cx);
            },
        );
        let reset = (mode != Mode::default()).then(|| {
            let label = self.ui_language().text("Reset project grouping");
            let button = web_button(
                "reset-project-grouping",
                WebButtonVariant::Ghost,
                WebButtonSize::IconSmall,
                ButtonState::Enabled,
            )
            .debug_selector(|| "reset-project-grouping".into())
            .aria_label(label)
            .tooltip(move |w, cx| Tooltip::new(label).build(w, cx))
            .child(
                svg()
                    .path(assets::ROTATE_CCW_ICON_PATH)
                    .size(bruhs_ui::zoom::px(14.))
                    .text_color(theme::rambutan_400()),
            );
            on_press_view(button, cx, |this, _, cx| {
                this.project_list.grouping.mode = Mode::default();
                this.persist_and_notify(cx);
            })
            .into_any_element()
        });
        self.settings_row_with_reset(
            "desktop-settings-row-project-grouping",
            "Project grouping",
            "Combine checkouts of the same repository.",
            None,
            false,
            Some(toggle),
            reset,
        )
    }

    pub(super) fn project_grouping_override_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let id = "settings-project-grouping-override";
        let scope = &self.settings_project_scope;
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let control = self.checkout_rule_control(
            id,
            scope,
            &format!("Grouping rule for {scope}"),
            self.settings_listbox(id),
            toggle,
            select,
        );
        self.settings_row(
            "settings-project-grouping-override-row",
            "Project grouping",
            "How this checkout groups with other copies.",
            control,
        )
    }

    pub(super) fn checkout_rule_control(
        &self,
        id: &'static str,
        scope: &str,
        label: &str,
        listbox: Listbox,
        toggle: ToggleHandler,
        select: SelectHandler,
    ) -> AnyElement {
        let prefs = &self.project_list.grouping;
        let saved = prefs.overrides.get(scope);
        let value = saved.map_or("inherit", |mode| mode.choice().0);
        let options = [
            ("inherit", "Use global default"),
            Mode::Repository.choice(),
            Mode::RepositoryPath.choice(),
            Mode::Separate.choice(),
        ]
        .into_iter()
        .map(|(value, label)| DropdownOption {
            value,
            label: label.into(),
            description: None,
        })
        .collect();
        let mut control = bruhs_ui::dropdown::Dropdown::new(
            id,
            label,
            value,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            listbox,
            toggle,
            select,
        );
        if saved.is_none() {
            control = control.selection_label(format!("Default ({})", prefs.mode.choice().1), None);
        }
        div()
            .w(bruhs_ui::zoom::px(320.))
            .max_w_full()
            .min_w_0()
            .child(control)
            .into_any_element()
    }

    pub(super) fn set_grouping_override(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        self.set_checkout_grouping(self.settings_project_scope.clone(), value, cx);
    }
    pub(super) fn set_checkout_grouping(
        &mut self,
        scope: String,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if scope.is_empty() {
            return;
        }
        let mode = match value {
            "inherit" => None,
            "repository" => Some(Mode::Repository),
            "repository_path" => Some(Mode::RepositoryPath),
            "separate" => Some(Mode::Separate),
            _ => return,
        };
        if let Some(mode) = mode {
            self.project_list.grouping.overrides.insert(scope, mode);
        } else {
            self.project_list.grouping.overrides.remove(&scope);
        }
        self.persist_and_notify(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    #[gpui::test]
    async fn project_grouping_controls_match_real_origins_and_preserve_checkout_targets(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ws, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        let temp = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for (clone, remote) in [
            (
                "one",
                "https://someone:private@example.org/team/editor.git?token=private",
            ),
            ("two", "git@example.org:team/editor.git"),
            ("three", "ssh://git@example.org/other/editor.git"),
        ] {
            let dir = temp.path().join(clone);
            std::fs::create_dir_all(dir.join("src")).unwrap();
            for args in [vec!["init", "-q"], vec!["remote", "add", "origin", remote]] {
                assert!(std::process::Command::new("git")
                    .args(args)
                    .current_dir(&dir)
                    .status()
                    .unwrap()
                    .success());
            }
            paths.push(dir.join("src").to_str().unwrap().to_owned());
        }
        ws.update_in(cx, |ws, window, cx| {
            for path in &paths {
                ws.create_project_thread(
                    DesktopProjectLocation {
                        path: path.clone(),
                        name: "src".into(),
                    },
                    window,
                    cx,
                );
            }
            ws.open_settings(&OpenSettings, window, cx);
            ws.reveal_settings_row("desktop-settings-row-project-grouping".into(), cx);
        });
        cx.condition(&ws, |ws, _| {
            paths
                .iter()
                .all(|p| ws.project_identities.values.contains_key(p))
        })
        .await;
        let key = ws.update(cx, |ws, _| {
            assert_eq!(
                ws.project_group_key(&paths[0]),
                ws.project_group_key(&paths[1])
            );
            assert_ne!(
                ws.project_group_key(&paths[0]),
                ws.project_group_key(&paths[2])
            );
            assert_eq!(
                ws.group_label(&paths[0]).as_deref(),
                Some("example.org/team/editor")
            );
            ws.project_group_key(&paths[0])
        });
        let click = |cx: &mut gpui::VisualTestContext, selector| {
            cx.run_until_parked();
            let center = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_click(center, gpui::Modifiers::default());
            cx.run_until_parked();
        };
        click(cx, "settings-project-grouping");
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.snapshot(cx).project_list.grouping.mode, Mode::Separate);
            assert_ne!(
                ws.project_group_key(&paths[0]),
                ws.project_group_key(&paths[1])
            );
            ws.set_settings_section(SettingsSection::Projects, cx);
            for path in &paths[..2] {
                ws.settings_project_scope = path.clone();
                ws.select_settings_option(
                    "settings-project-grouping-override",
                    "repository_path",
                    window,
                    cx,
                );
            }
            assert_eq!(
                ws.project_group_key(&paths[0]),
                ws.project_group_key(&paths[1])
            );
            assert_ne!(ws.project_group_key(&paths[0]), key);
            let group = ws
                .grouped_projects()
                .into_iter()
                .find(|g| g.members.len() == 2)
                .unwrap();
            assert_eq!(group.label, "example.org/team/editor/src");
            let prefs = serde_json::to_string(&ws.snapshot(cx).project_list).unwrap();
            assert!(!prefs.contains("private"));
            ws.project_list = serde_json::from_str(&prefs).unwrap();
            for path in &paths[..2] {
                ws.settings_project_scope = path.clone();
                ws.select_settings_option(
                    "settings-project-grouping-override",
                    "inherit",
                    window,
                    cx,
                );
            }
            ws.set_settings_section(SettingsSection::General, cx);
            ws.reveal_settings_row("desktop-settings-row-project-grouping".into(), cx);
        });
        click(cx, "reset-project-grouping");
        ws.update_in(cx, |ws, _, cx| {
            assert!(ws.project_list.grouping.is_default());
            ws.labs_sidebar.enabled = true;
            ws.close_transient_surfaces();
            cx.notify();
        });
        cx.run_until_parked();
        let selector = |prefix: &str, key: &str| -> &'static str {
            Box::leak(format!("{prefix}-{key}").into_boxed_str())
        };
        assert!(cx
            .debug_bounds(selector("desktop-sidebar-folder", &key))
            .is_some());
        ws.update_in(cx, |ws, window, cx| {
            ws.open_project_picker(&OpenProjectPicker, window, cx)
        });
        click(
            cx,
            selector("desktop-project-group", &element_key(&key).to_string()),
        );
        click(
            cx,
            selector(
                "desktop-project-recent",
                &element_key(&paths[0]).to_string(),
            ),
        );
        cx.condition(&ws, |ws, _| {
            ws.active_project.as_ref().is_some_and(|p| {
                Path::new(&p.path).canonicalize().ok() == Path::new(&paths[0]).canonicalize().ok()
            })
        })
        .await;
    }
}
