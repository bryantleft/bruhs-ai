//! Remove the selected desktop project records while preserving files and Git worktrees.
use super::*;
use bruhs_models::ids::ThreadId;
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct RemovalState {
    busy: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement {
    MemberRow,
    Section,
}

#[derive(Clone)]
pub(super) enum Target {
    Checkout(String),
    Group {
        key: String,
        paths: BTreeSet<String>,
    },
}
impl Target {
    pub(super) fn group(group: &project_grouping::Group) -> Self {
        Self::Group {
            key: group.key.clone(),
            paths: group.members.iter().map(|m| m.path.clone()).collect(),
        }
    }
    fn paths(&self) -> BTreeSet<String> {
        match self {
            Self::Checkout(path) => [path.clone()].into(),
            Self::Group { paths, .. } => paths.clone(),
        }
    }
    fn available(&self, ws: &Workspace) -> bool {
        match self {
            Self::Checkout(path) => ws.listed_projects().iter().any(|p| &p.path == path),
            Self::Group { key, paths } => ws.grouped_projects().iter().any(|g| {
                &g.key == key
                    && g.members.len() == paths.len()
                    && g.members
                        .iter()
                        .all(|m| m.host.is_none() && paths.contains(&m.path))
            }),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Contents {
    threads: BTreeSet<String>,
    schedules: BTreeMap<String, BTreeMap<String, String>>,
}
impl Workspace {
    fn removal_contents(&self, paths: &BTreeSet<String>, cx: &App) -> Contents {
        let live = self
            .threads
            .iter()
            .filter(|t| {
                let chat = t.chat.read(cx);
                chat.project_scoped && paths.contains(chat.settings_project_path())
            })
            .map(|t| t.id.clone());
        let archived = self
            .archived_threads
            .iter()
            .filter(|t| t.project_root(cx).is_some_and(|p| paths.contains(p)))
            .map(|t| t.id().to_owned());
        let threads = live.chain(archived).collect::<BTreeSet<_>>();
        let schedules = threads
            .iter()
            .filter_map(|id| {
                self.desktop_task_links
                    .get(id)
                    .map(|tasks| (id.clone(), tasks.clone()))
            })
            .collect();
        Contents { threads, schedules }
    }

    pub(super) fn remove_checkout_button(
        &self,
        id: String,
        placement: Placement,
        target: Target,
        permitted: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let disabled = !permitted || self.removal.busy;
        let label = match (&target, placement) {
            (_, Placement::MemberRow) => "Remove",
            (Target::Checkout(_), _) => "Remove checkout",
            (Target::Group { paths, .. }, _) if paths.len() > 1 => "Remove all entries",
            (Target::Group { .. }, _) => "Remove project",
        };
        let accessible = match &target {
            Target::Checkout(path) => format!("Remove checkout {path}"),
            Target::Group { .. } => label.to_owned(),
        };
        let button = web_button(
            SharedString::from(id.clone()),
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .when(placement == Placement::Section, |el| {
            el.border_color(theme::lychee_400().opacity(0.55))
                .text_color(theme::lychee_400())
                .child(Icon::new(IconName::Delete).size(px(14.)))
        })
        .debug_selector(move || id)
        .aria_label(accessible)
        .child(label);
        on_press_view(button, cx, move |ws, w, cx| {
            if !disabled {
                ws.confirm_checkout_removal(target.clone(), w, cx);
            }
        })
        .into_any_element()
    }
    pub(super) fn removal_error(&self) -> Option<&str> {
        self.removal.error.as_deref()
    }

    fn confirm_checkout_removal(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.removal.busy || !target.available(self) {
            return;
        }
        let paths = target.paths();
        let contents = self.removal_contents(&paths, cx);
        let count = contents.threads.len();
        let mut detail = format!(
            "Remove the following {} and {} {} from bruhs? Files and Git worktrees stay on disk.\n\n{}",
            if paths.len() == 1 {
                "checkout"
            } else {
                "checkouts"
            },
            count,
            if count == 1 {
                "conversation"
            } else {
                "conversations"
            },
            paths.iter().cloned().collect::<Vec<_>>().join("\n")
        );
        let tasks = contents
            .schedules
            .values()
            .flat_map(|names| names.values().cloned())
            .collect::<Vec<_>>();
        if !tasks.is_empty() {
            detail.push_str(&format!(
                "\n\nAlso delete {} linked scheduled {}:\n{}",
                tasks.len(),
                if tasks.len() == 1 { "task" } else { "tasks" },
                tasks.join("\n")
            ));
        }
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            if paths.len() == 1 {
                "Remove checkout?"
            } else {
                "Remove all checkout entries?"
            },
            Some(&detail),
            &[
                gpui::PromptButton::Cancel("Cancel".into()),
                gpui::PromptButton::Other("Remove".into()),
            ],
            cx,
        );
        self.removal.busy = true;
        self.removal.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let confirmed = prompt.await == Ok(1);
            let _ = this.update(cx, |ws, cx| {
                ws.removal.busy = false;
                if confirmed {
                    ws.remove_confirmed_checkout(target, contents, cx);
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn remove_confirmed_checkout(
        &mut self,
        target: Target,
        contents: Contents,
        cx: &mut Context<'_, Self>,
    ) {
        let paths = target.paths();
        if !target.available(self) || self.removal_contents(&paths, cx) != contents {
            self.removal.error =
                Some("The checkout or its conversations changed. Review the removal again.".into());
            cx.notify();
            return;
        }
        if contents.threads.is_empty() {
            self.finish_checkout_removal(&paths, &contents.threads, cx);
            return;
        }
        let mut threads = BTreeMap::new();
        for id in &contents.threads {
            let Ok(thread) = ThreadId::try_from(id.as_str()) else {
                self.removal.error = Some(
                    "A saved conversation has an invalid ID. Its schedules were preserved.".into(),
                );
                cx.notify();
                return;
            };
            threads.insert(
                thread,
                contents
                    .schedules
                    .get(id)
                    .into_iter()
                    .flat_map(|tasks| tasks.keys().cloned())
                    .collect(),
            );
        }
        self.removal.busy = true;
        let cached_links = !contents.schedules.is_empty();
        let client = self.reactive_automations.read(cx).client();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let ids = threads.keys().cloned().collect::<Vec<_>>();
            match client.remove_checkout_task_threads(threads).await {
                Ok(_) => Ok(()),
                Err(crate::server_client::ServerError::Transport { source, .. })
                    if source.is_connect() && !cached_links =>
                {
                    let linked = tokio::task::spawn_blocking(move || {
                        bruhs_core::reactive::ReactiveRuntime::has_scheduled_desktop_threads(&ids)
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                    if linked {
                        Err("Reconnect the task host to review its schedules.".to_owned())
                    } else {
                        Ok(())
                    }
                }
                Err(crate::server_client::ServerError::Status { code, .. })
                    if code == reqwest::StatusCode::CONFLICT =>
                {
                    Err("The linked schedules changed. Review the removal again.".into())
                }
                Err(error) => Err(error.to_string()),
            }
        });
        cx.spawn(async move |this, cx| {
            let outcome = job.await;
            let _ = this.update(cx, |ws, cx| {
                ws.removal.busy = false;
                match outcome {
                    Ok(()) => {
                        let current = ws.removal_contents(&paths, cx);
                        let new_links = current.schedules.iter().any(|(id, tasks)| {
                            !tasks.is_empty() && contents.schedules.get(id) != Some(tasks)
                        });
                        if target.available(ws) && contents.threads == current.threads && !new_links {
                            ws.finish_checkout_removal(&paths, &contents.threads, cx);
                        } else {
                            ws.removal.error = Some(
                                "The checkout changed during removal. Review its current conversations and schedules.".into(),
                            );
                        }
                    }
                    Err(error) => {
                        ws.removal.error = Some(format!("Could not remove the checkout. {error}"));
                    }
                }
                ws.reactive_automations.update(cx, |panel, cx| panel.refresh_snapshot(cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_checkout_removal(
        &mut self,
        paths: &BTreeSet<String>,
        threads: &BTreeSet<String>,
        cx: &mut Context<'_, Self>,
    ) {
        for t in &self.threads {
            if threads.contains(&t.id) && t.chat.read(cx).is_streaming {
                t.chat.update(cx, |chat, cx| chat.stop_streaming(cx));
            }
        }
        self.archived_threads.retain(|t| !threads.contains(t.id()));
        for id in threads {
            self.forget_thread(id, cx);
        }
        self.recent_projects.retain(|p| !paths.contains(&p.path));
        if self
            .active_project
            .as_ref()
            .is_some_and(|p| paths.contains(&p.path))
        {
            self.active_project = None;
        }
        for path in paths {
            self.project_list.forget(path);
            self.project_defaults.remove(path);
            self.project_actions.remove(path);
        }
        if paths.contains(&self.settings_project_scope) {
            self.action_editor.open = false;
            self.settings_project_scope.clear();
        }
        self.settings_search_catalog.clear();
        if !self.settings_search.read(cx).value().trim().is_empty() {
            self.refresh_settings_search(cx);
        }
        self.removal.error = None;
        self.sync_thread_catalog(cx);
        self.persist_and_notify(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::launch;
    use super::*;
    use ::core::prelude::v1::test;

    #[gpui::test]
    async fn checkout_removal_confirms_exact_threads_and_keeps_git_files(
        cx: &mut gpui::TestAppContext,
    ) {
        launch::init(cx);
        let mut next = cx.clone();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("editor");
        let sibling = temp.path().join("review");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&root)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.name=Tester",
            "-c",
            "user.email=test@example.org",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "Initial",
        ]);
        git(&["worktree", "add", "--detach", sibling.to_str().unwrap()]);
        let managed = temp.path().join("topic");
        git(&["worktree", "add", "--detach", managed.to_str().unwrap()]);
        let file = managed.join("unsaved.txt");
        std::fs::write(&file, "uncommitted work").unwrap();
        let path = temp.path().join("desktop.json");
        let local = root.to_str().unwrap().to_owned();
        let review = sibling.to_str().unwrap().to_owned();
        let (ws, win) = launch::launch(cx, &path);
        win.update(|w, _| w.activate_window());
        win.simulate_resize(gpui::size(px(1040.), px(820.)));
        let (archived, keep) = ws.update_in(win, |ws, w, cx| {
            ws.cleanup_deleted_worktrees = true;
            ws.confirm_thread_archive = false;
            let new =
                |ws: &mut Workspace, p: &str, w: &mut Window, cx: &mut Context<'_, Workspace>| {
                    ws.create_project_thread(
                        DesktopProjectLocation {
                            path: p.into(),
                            name: "Editor".into(),
                        },
                        w,
                        cx,
                    );
                    ws.active_thread_id.clone().unwrap()
                };
            let archived = new(ws, &local, w, cx);
            ws.active_thread().unwrap().chat.update(cx, |chat, _| {
                chat.worktree_path = Some(managed.to_str().unwrap().into());
                chat.project_settings_root = Some(local.clone());
            });
            ws.request_thread_archive(&archived, w, cx);
            new(ws, &local, w, cx);
            let keep = new(ws, &review, w, cx);
            ws.project_list.set_title(&local, Some("My editor"));
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Projects, cx);
            ws.settings_project_scope = local.clone();
            ws.reveal_settings_row("settings-remove-checkout-row".into(), cx);
            (archived, keep)
        });
        launch::saved(&ws, win).await;
        let click = |ws: &Entity<Workspace>, win: &mut gpui::VisualTestContext| {
            win.run_until_parked();
            ws.update_in(win, |ws, _, cx| {
                let bottom = -ws.settings_scroll.max_offset().y;
                ws.settings_scroll.set_offset(gpui::point(px(0.), bottom));
                cx.notify();
            });
            win.run_until_parked();
            let b = win
                .debug_bounds("settings-remove-checkout")
                .expect("visible checkout action");
            win.simulate_click(b.center(), Default::default());
            win.run_until_parked();
            assert!(win.has_pending_prompt(), "remove button bounds {b:?}");
        };
        click(&ws, win);
        win.simulate_prompt_answer("Cancel");
        win.run_until_parked();
        ws.read_with(win, |ws, _| {
            assert!(ws.archived_threads.iter().any(|t| t.id() == archived))
        });
        click(&ws, win);
        ws.update_in(win, |ws, w, cx| {
            ws.create_project_thread(
                DesktopProjectLocation {
                    path: local.clone(),
                    name: "Editor".into(),
                },
                w,
                cx,
            );
        });
        win.simulate_prompt_answer("Remove");
        win.run_until_parked();
        ws.read_with(win, |ws, _| {
            assert!(ws.removal_error().unwrap().contains("changed"))
        });
        launch::saved(&ws, win).await;
        launch::quit(&ws, win);
        let (ws, win) = launch::launch(&mut next, &path);
        win.update(|w, _| w.activate_window());
        win.simulate_resize(gpui::size(px(820.), px(640.)));
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Projects, cx);
            ws.settings_project_scope = local.clone();
            ws.reactive_automations.update(cx, |panel, _| {
                let offline = "http://127.0.0.1:9";
                panel.set_delivery_test_client(ServerClient::new(offline, offline).unwrap());
            });
            ws.reveal_settings_row("settings-remove-checkout-row".into(), cx);
        });
        click(&ws, win);
        win.simulate_prompt_answer("Remove");
        win.condition(&ws, |ws, _| !ws.removal.busy).await;
        ws.read_with(win, |ws, cx| {
            assert!(ws.removal_error().is_none(), "{:?}", ws.removal_error());
            assert!(!ws.archived_threads.iter().any(|t| t.id() == archived));
            assert!(ws.threads.iter().any(|t| t.id == keep));
            assert!(
                ws.threads
                    .iter()
                    .all(|t| t.chat.read(cx).settings_project_path() != local)
            );
            assert!(ws.listed_projects().iter().all(|p| p.path != local));
            assert!(ws.project_list.title(&local).is_none());
            assert!(ws.pending_scratch_cleanups.is_empty());
        });
        launch::saved(&ws, win).await;
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains(&archived) && saved.contains(&keep));
        assert_eq!(std::fs::read_to_string(file).unwrap(), "uncommitted work");
        git(&[
            "-C",
            managed.to_str().unwrap(),
            "rev-parse",
            "--is-inside-work-tree",
        ]);
        assert!(root.join(".git").exists() && sibling.join(".git").exists());
    }
}
