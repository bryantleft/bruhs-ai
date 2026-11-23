//! Project display names belong to physical checkouts and survive regrouping.
use super::*;

use bruhs_core::linked_projects::names::{self, Rename, Titles};

#[derive(Default)]
pub(super) struct SyncState {
    pub(super) busy: bool,
    revision: u64,
    error: Option<String>,
    last_target: Option<Target>,
}

type Destination = (Option<uuid::Uuid>, String);

#[derive(Clone, PartialEq)]
struct Target {
    group: Option<String>,
    members: BTreeMap<Destination, Option<String>>,
}

impl Workspace {
    fn project_title_state_path(&self) -> Option<PathBuf> {
        #[cfg(test)]
        {
            self.persistence_test_path.clone()
        }
        #[cfg(not(test))]
        {
            desktop_state_path()
        }
    }

    fn name_target(
        &self,
        group: Option<String>,
        members: impl IntoIterator<Item = Destination>,
    ) -> Target {
        let members = members
            .into_iter()
            .map(|(host, path)| {
                let title = match host {
                    Some(id) => self
                        .linked_projects
                        .name_access(id, &path)
                        .and_then(|(_, title)| title),
                    None => self.project_list.title(&path).map(str::to_owned),
                };
                ((host, path), title)
            })
            .collect();
        Target { group, members }
    }

    fn save_project_name(
        &mut self,
        target: &Target,
        title: Option<&str>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.project_names.busy {
            return;
        }
        self.project_names.last_target = Some(target.clone());
        let members: Vec<_> = if let Some(key) = &target.group {
            self.grouped_projects()
                .into_iter()
                .find(|g| &g.key == key)
                .map(|g| g.members.into_iter().map(|m| (m.host, m.path)).collect())
                .unwrap_or_default()
        } else {
            target.members.keys().cloned().collect()
        };
        let current = self.name_target(target.group.clone(), members);
        if current.members != target.members || current.members.is_empty() {
            self.project_names.error =
                Some("The project changed. Review its current checkouts and names.".into());
            cx.notify();
            return;
        }
        let mut by_host = BTreeMap::<Option<uuid::Uuid>, Titles>::new();
        for ((host, path), expected) in &target.members {
            if host.is_none()
                && self.settings_project_scope != *path
                && !self.listed_projects().iter().any(|p| &p.path == path)
            {
                self.project_names.error = Some("This checkout is no longer available.".into());
                cx.notify();
                return;
            }
            by_host
                .entry(*host)
                .or_default()
                .insert(path.clone(), expected.clone());
        }
        let title = title.map(str::to_owned);
        let mut jobs = Vec::new();
        let local = by_host.remove(&None).map(|expected| Rename {
            expected,
            title: title.clone(),
        });
        for (host, expected) in by_host {
            let edit = Rename {
                expected,
                title: title.clone(),
            };
            let Some(job) = host.and_then(|id| self.linked_projects.rename_on_host(id, edit))
            else {
                self.project_names.error = Some("A linked checkout requires rename access.".into());
                cx.notify();
                return;
            };
            jobs.push(job);
        }
        self.project_names.busy = true;
        self.project_names.revision += 1;
        self.project_names.error = None;
        let state = self.project_title_state_path();
        let work = crate::app_state::spawn_tokio(cx, async move {
            let local_save = async {
                if let (Some(path), Some(edit)) = (state, local.clone()) {
                    tokio::task::spawn_blocking(move || names::rename(&path, &edit, true))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                }
                Ok::<_, String>(local)
            };
            tokio::join!(local_save, futures_util::future::join_all(jobs))
        });
        cx.spawn(async move |this, cx| {
            let (local, remote) = work.await;
            let _ = this.update(cx, |ws, cx| {
                ws.project_names.busy = false;
                let mut errors: Vec<_> = remote.into_iter().filter_map(Result::err).collect();
                match local {
                    Ok(Some(edit)) => {
                        for path in edit.expected.keys() {
                            if ws.settings_project_scope == *path
                                || ws.listed_projects().iter().any(|p| p.path == *path)
                            {
                                ws.project_list.set_title(path, edit.title.as_deref());
                            }
                        }
                        ws.persist(cx);
                    }
                    Err(e) => errors.push(e),
                    Ok(None) => {}
                }
                ws.project_names.error = (!errors.is_empty()).then(|| errors.join(" "));
                ws.refresh_name_inventory(cx);
                ws.refresh_after_project_rename(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_name_inventory(&mut self, cx: &mut Context<'_, Self>) {
        self.settings_search_catalog.clear();
        if !self.settings_search.read(cx).value().trim().is_empty() {
            self.refresh_settings_search(cx);
        }
    }

    fn apply_pending_project_names(&mut self, pending: Titles, cx: &mut Context<'_, Self>) {
        if pending.is_empty() {
            return;
        }
        let listed = self.listed_projects();
        let mut changed = false;
        for (path, title) in pending {
            let known = listed.iter().any(|p| p.path == path)
                || self.active_project.as_ref().is_some_and(|p| p.path == path)
                || self.threads.iter().any(|thread| {
                    let chat = thread.chat.read(cx);
                    chat.project_scoped && chat.settings_project_path() == path
                });
            if known && self.project_list.title(&path) != title.as_deref() {
                self.project_list.set_title(&path, title.as_deref());
                changed = true;
            }
        }
        if changed {
            self.refresh_name_inventory(cx);
            self.persist_and_notify(cx);
        }
    }

    #[cfg(not(test))]
    pub(super) fn watch_project_names(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = self.project_title_state_path() else {
            return;
        };
        cx.spawn(async move |this, cx| loop {
            let Ok(revision) = this.read_with(cx, |ws, _| ws.project_names.revision) else {
                break;
            };
            let file = path.clone();
            let pending = cx
                .background_spawn(async move { names::pending(&file) })
                .await;
            let Ok(()) = this.update(cx, |ws, cx| {
                if ws.project_names.busy || ws.project_names.revision != revision {
                    return;
                }
                let Ok(pending) = pending else { return };
                ws.apply_pending_project_names(pending, cx);
            }) else {
                break;
            };
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
        })
        .detach();
    }

    pub(super) fn checkout_name_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let path = &self.settings_project_scope;
        let name = self
            .listed_projects()
            .into_iter()
            .find(|p| p.path == *path)
            .map_or_else(
                || project_picker_ui::path_display_name(Path::new(path)),
                |p| p.name,
            );
        self.project_name_row(
            self.name_target(None, [(None, path.clone())]),
            "settings-project-name".into(),
            name,
            "The name shown for this checkout in project lists.",
            false,
            cx,
        )
    }

    pub(super) fn linked_checkout_name_row(
        &self,
        member: &project_grouping::Member,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let editable = member
            .host
            .and_then(|id| self.linked_projects.name_access(id, &member.path))
            .is_some_and(|(allowed, _)| allowed);
        self.project_name_row(
            self.name_target(None, [(member.host, member.path.clone())]),
            "linked-checkout-name".into(),
            member.name.clone(),
            if editable {
                "The display name saved on the linked environment."
            } else {
                "This name is set on the linked environment."
            },
            !editable,
            cx,
        )
    }

    pub(super) fn group_name_row(
        &self,
        group: &project_grouping::Group,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let read_only = group.members.iter().any(|m| {
            m.host.is_some_and(|id| {
                !self
                    .linked_projects
                    .name_access(id, &m.path)
                    .is_some_and(|(allowed, _)| allowed)
            })
        });
        let description = if read_only {
            "Each linked checkout needs rename access to edit the shared name."
        } else {
            "The shared name for this project group in the sidebar and project lists."
        };
        self.project_name_row(
            self.name_target(
                Some(group.key.clone()),
                group.members.iter().map(|m| (m.host, m.path.clone())),
            ),
            format!("settings-group-name-{}", element_key(&group.key)),
            group.label.clone(),
            description,
            read_only,
            cx,
        )
    }

    fn project_name_row(
        &self,
        target: Target,
        id: String,
        name: String,
        detail: &str,
        read_only: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let read_only = read_only || self.project_names.busy;
        let reset = (!read_only && target.members.values().any(Option::is_some)).then(|| {
            let target = target.clone();
            let button = web_button(
                SharedString::from(format!("{id}-reset")),
                WebButtonVariant::Ghost,
                WebButtonSize::IconSmall,
                ButtonState::Enabled,
            )
            .debug_selector({
                let id = id.clone();
                move || format!("{id}-reset")
            })
            .aria_label("Reset project name")
            .tooltip(|w, cx| Tooltip::new("Reset project name").build(w, cx))
            .child(
                svg()
                    .path(assets::ROTATE_CCW_ICON_PATH)
                    .size(bruhs_ui::zoom::px(14.))
                    .text_color(theme::rambutan_400()),
            );
            on_press_view(button, cx, move |ws, _, cx| {
                ws.save_project_name(&target, None, cx)
            })
            .into_any_element()
        });
        let error = self.project_names.last_target.as_ref().and_then(|last| {
            let same_row = last.group == target.group
                && (last.group.is_some() || last.members.keys().eq(target.members.keys()));
            self.project_names.error.clone().filter(|_| same_row)
        });
        let control = NameField {
            error,
            owner: cx.entity().downgrade(),
            target,
            name,
            id: id.clone(),
            read_only,
        };
        self.settings_row_with_reset(
            id,
            "Name",
            detail.to_owned(),
            None,
            false,
            Some(control.into_any_element()),
            reset,
        )
    }
}

#[derive(IntoElement)]
struct NameField {
    error: Option<String>,
    owner: WeakEntity<Workspace>,
    target: Target,
    name: String,
    id: String,
    read_only: bool,
}
struct NameDraft {
    target: Target,
    displayed: String,
    input: Entity<InputState>,
    invalid: bool,
    dirty: bool,
    _events: Subscription,
}
impl RenderOnce for NameField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (&id, &self.target.group, self.read_only).hash(&mut hash);
        if self.target.group.is_none() {
            for member in self.target.members.keys() {
                member.hash(&mut hash);
            }
        }
        let key = SharedString::from(format!("project-name-draft-{}", hash.finish()));
        let read_only = self.read_only;
        let draft = window.use_keyed_state(key, cx, |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(self.name.clone()));
            let sub = cx.subscribe_in(
                &input,
                window,
                move |draft: &mut NameDraft, input, event, window, cx| {
                    if read_only {
                        return;
                    }
                    match event {
                        InputEvent::Change => {
                            draft.dirty = true;
                            if draft.invalid && !input.read(cx).value().trim().is_empty() {
                                draft.invalid = false;
                                cx.notify();
                            }
                        }
                        InputEvent::Blur | InputEvent::PressEnter { .. } if draft.dirty => {
                            let value = input.read(cx).value();
                            let title = value.trim();
                            if title.is_empty() {
                                draft.invalid = true;
                                cx.notify();
                                return;
                            }
                            draft.dirty = false;
                            let _ = self.owner.update(cx, |ws, cx| {
                                ws.save_project_name(&draft.target, Some(title), cx)
                            });
                            if matches!(event, InputEvent::PressEnter { .. }) {
                                window.blur(cx);
                            }
                            if title != value.as_ref() {
                                input.update(cx, |i, cx| i.set_value(title.to_owned(), window, cx));
                            }
                        }
                        _ => {}
                    }
                },
            );
            NameDraft {
                target: self.target.clone(),
                displayed: self.name.clone(),
                input,
                invalid: false,
                dirty: false,
                _events: sub,
            }
        });
        // Keep the original expected titles while a person is typing. A host
        // refresh may change the label before blur submits the draft.
        let state = draft.read(cx);
        if !state.dirty && (state.target != self.target || state.displayed != self.name) {
            draft.update(cx, |draft, cx| {
                if draft.input.read(cx).value().as_ref() != self.name {
                    draft.input.update(cx, |input, cx| {
                        input.set_value(self.name.clone(), window, cx)
                    });
                }
                draft.displayed = self.name;
                draft.target = self.target;
            });
        }
        let state = draft.read(cx);
        div()
            .id(SharedString::from(id.clone()))
            .w(bruhs_ui::zoom::px(256.))
            .max_w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("project-name-input")
                    .debug_selector(move || format!("{id}-field"))
                    .child(
                        Input::new(&state.input)
                            .aria_label("Project name")
                            .readonly(read_only),
                    ),
            )
            .when_some(self.error, |el, error| {
                el.child(
                    div()
                        .id("project-name-save-error")
                        .role(Role::Alert)
                        .debug_selector(|| "project-name-save-error".into())
                        .text_xs()
                        .text_color(theme::persimmon_400())
                        .child(error),
                )
            })
            .when(state.invalid, |el| {
                el.child(
                    div()
                        .id("project-name-error")
                        .debug_selector(|| "project-name-error".into())
                        .text_xs()
                        .text_color(theme::persimmon_400())
                        .child("Project name cannot be empty."),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;
    use ::core::prelude::v1::test;

    #[gpui::test]
    async fn project_name_edits_group_and_checkout_without_retargeting_threads(
        cx: &mut gpui::TestAppContext,
    ) {
        launch::init(cx);
        let mut second_launch = cx.clone();
        let temp = tempfile::tempdir().unwrap();
        let saved = temp.path().join("workspace.json");
        let (ws, win) = launch::launch(cx, &saved);
        win.update(|window, _| window.activate_window());
        win.simulate_resize(gpui::size(px(1040.), px(820.)));
        let mut paths = Vec::new();
        for folder in ["one", "two"] {
            let path = temp.path().join(folder);
            std::fs::create_dir(&path).unwrap();
            for args in [
                vec!["init", "-q"],
                vec![
                    "remote",
                    "add",
                    "origin",
                    "https://example.org/team/editor.git",
                ],
            ] {
                assert!(std::process::Command::new("git")
                    .args(args)
                    .current_dir(&path)
                    .status()
                    .unwrap()
                    .success());
            }
            let path = path.to_str().unwrap().to_owned();
            ws.update_in(win, |ws, w, cx| {
                ws.create_project_thread(
                    DesktopProjectLocation {
                        path: path.clone(),
                        name: folder.into(),
                    },
                    w,
                    cx,
                )
            });
            paths.push(path);
        }
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Projects, cx);
        });
        win.condition(&ws, |ws, _| {
            ws.grouped_projects().iter().any(|g| g.members.len() == 2)
        })
        .await;
        let row = ws.update_in(win, |ws, _, cx| {
            let group = ws
                .grouped_projects()
                .into_iter()
                .find(|g| g.members.len() == 2)
                .unwrap();
            let row = format!("settings-group-name-{}", element_key(&group.key));
            ws.settings_project_scope.clear();
            ws.reveal_settings_row(row.clone().into(), cx);
            row
        });
        let click = |win: &mut gpui::VisualTestContext, id: &str| {
            win.run_until_parked();
            let center = win
                .debug_bounds(role_settings_ui::project_value(id))
                .unwrap_or_else(|| panic!("missing {id}"))
                .center();
            win.simulate_click(center, Default::default());
            win.run_until_parked();
        };
        click(win, &format!("{row}-field"));
        win.simulate_keystrokes("cmd-a");
        win.simulate_input("  Studio editor  ");
        win.simulate_keystrokes("enter");
        win.condition(&ws, |ws, _| {
            !ws.project_names.busy && ws.project_list.title(&paths[0]) == Some("Studio editor")
        })
        .await;
        ws.read_with(win, |ws, cx| {
            for p in &paths {
                assert_eq!(ws.project_list.title(p), Some("Studio editor"));
            }
            assert!(ws
                .grouped_projects()
                .iter()
                .any(|g| g.label == "Studio editor" && g.members.len() == 2));
            assert_eq!(ws.matching_thread_items("Studio editor", cx).len(), 2);
        });
        launch::saved(&ws, win).await;
        launch::quit(&ws, win);
        let (ws, win) = launch::launch(&mut second_launch, &saved);
        win.update(|window, _| window.activate_window());
        win.simulate_resize(gpui::size(px(820.), px(640.)));
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Projects, cx);
            ws.settings_project_scope = paths[0].clone();
            ws.reveal_settings_row("settings-project-name".into(), cx);
        });
        click(win, "settings-project-name-field");
        win.simulate_keystrokes("cmd-a");
        win.simulate_keystrokes("backspace");
        win.simulate_keystrokes("enter");
        win.run_until_parked();
        ws.read_with(win, |ws, _| {
            assert_eq!(ws.project_list.title(&paths[0]), Some("Studio editor"))
        });
        assert!(
            win.debug_bounds("project-name-error").is_some(),
            "invalid input marked"
        );
        click(win, "settings-project-name-field");
        win.simulate_keystrokes("cmd-a");
        win.simulate_input("Editing");
        win.run_until_parked();
        assert!(
            win.debug_bounds("project-name-error").is_none(),
            "editing clears validation"
        );
        win.simulate_keystrokes("tab");
        win.condition(&ws, |ws, _| {
            ws.project_list.title(&paths[0]) == Some("Editing")
        })
        .await;
        ws.read_with(win, |ws, _| {
            assert_eq!(ws.project_list.title(&paths[0]), Some("Editing"));
            assert_eq!(ws.project_list.title(&paths[1]), Some("Studio editor"));
        });
        click(win, "settings-project-name-field");
        win.simulate_keystrokes("cmd-a");
        win.simulate_input("Unsubmitted draft");
        win.run_until_parked();
        names::rename(
            &saved,
            &Rename {
                expected: [(paths[0].clone(), Some("Editing".to_owned()))].into(),
                title: Some("Host edit".into()),
            },
            false,
        )
        .unwrap();
        ws.update_in(win, |ws, _, cx| {
            ws.apply_pending_project_names(names::pending(&saved).unwrap(), cx)
        });
        win.run_until_parked();
        win.simulate_keystrokes("tab");
        win.run_until_parked();
        ws.read_with(win, |ws, _| {
            assert_eq!(ws.project_list.title(&paths[0]), Some("Host edit"));
            assert!(ws
                .project_names
                .error
                .as_ref()
                .is_some_and(|error| error.contains("changed")));
        });
        launch::saved(&ws, win).await;
        ws.update_in(win, |ws, _, cx| {
            ws.reveal_settings_row("settings-project-name".into(), cx);
            cx.notify();
        });
        click(win, "settings-project-name-reset");
        win.condition(&ws, |ws, _| !ws.project_names.busy).await;
        ws.read_with(win, |ws, _| {
            assert!(
                ws.project_names.error.is_none(),
                "{:?}",
                ws.project_names.error
            )
        });
        ws.read_with(win, |ws, cx| {
            assert_eq!(ws.project_list.title(&paths[0]), None);
            assert!(ws
                .listed_projects()
                .iter()
                .any(|p| p.path == paths[0] && p.name == "one"));
            for p in &paths {
                assert!(ws
                    .threads
                    .iter()
                    .any(|t| t.chat.read(cx).settings_project_path() == p));
            }
        });
        launch::saved(&ws, win).await;
    }
}
