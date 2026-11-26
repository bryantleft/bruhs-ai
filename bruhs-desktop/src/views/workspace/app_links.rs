//! Open thread, view and draft links from the OS.

use super::*;
use bruhs_models::logic::app_link::{
    hosted_prompt_link, hosted_prompt_link_with_context, parse_app_link, parse_prompt_link,
    AppLink, LinkedPrompt, PromptDestination, PromptLink, PromptModel, PromptPullRequest,
    PromptRepository, PromptShareContext, PULL_CHECKOUT_SETUP_NOTICE,
};
use bruhs_ui::dialog::{dialog_backdrop, dialog_card, DialogPlacement};
use bruhs_ui::press::on_press_view;
use futures_util::{stream, StreamExt};

pub(super) const APP_LINK_UNKNOWN: &str = "Bruhs cannot open that link.";
pub(super) const APP_LINK_THREAD_MISSING: &str = "That thread is not on this device.";

pub(super) async fn matching_prompt_paths(
    paths: Vec<String>,
    repository: Option<PromptRepository>,
) -> Vec<String> {
    let Some(repository) = repository else {
        return Vec::new();
    };
    let repository = &repository;
    stream::iter(paths)
        .map(|path| async move {
            let matches = bruhs_core::git::project_identity(Path::new(&path))
                .await
                .is_some_and(|identity| repository.matches_remote(&identity.repository));
            matches.then_some(path)
        })
        .buffered(4)
        .filter_map(std::future::ready)
        .collect()
        .await
}

pub(super) enum TargetChoice {
    Repository {
        repository: PromptRepository,
        model: Option<PromptModel>,
        prompt: LinkedPrompt,
    },
    PullRequest {
        pull: PromptPullRequest,
        project: Option<DesktopProjectLocation>,
        prompt: Option<LinkedPrompt>,
        checkout_id: uuid::Uuid,
        running: bool,
        error: Option<String>,
    },
}

pub(super) enum CloneTarget {
    Repository {
        model: Option<PromptModel>,
        prompt: LinkedPrompt,
    },
    PullRequest {
        pull: PromptPullRequest,
        prompt: Option<LinkedPrompt>,
    },
}

impl Workspace {
    pub fn open_app_link(&mut self, link: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if matches!(
            &self.prompt_link_choice,
            Some(TargetChoice::PullRequest { running: true, .. })
        ) {
            self.app_link_notice(
                "A pull request checkout is in progress. Open the link again when it finishes.",
                cx,
            );
            return;
        }
        self.prompt_link_resolution = None;
        self.dismiss_prompt_target(cx);
        if self.clone_editor.linked_prompt.is_some() && self.project_picker_visible() {
            self.close_transient_surfaces();
        }
        match parse_prompt_link(link) {
            Ok(Some(draft)) => return self.open_linked_prompt(draft, window, cx),
            Err(error) => return self.app_link_notice(error.to_string(), cx),
            Ok(None) => {}
        }
        match parse_app_link(link) {
            Some(AppLink::Thread(id)) if self.threads.iter().any(|thread| thread.id == id) => {
                self.select_thread_by_id(&id, cx);
            }
            Some(AppLink::Thread(_)) => self.app_link_notice(APP_LINK_THREAD_MISSING, cx),
            Some(AppLink::View(view)) => self.open_routed_view(view, window, cx),
            None => self.app_link_notice(APP_LINK_UNKNOWN, cx),
        }
    }

    pub(super) fn copy_active_prompt_link(
        &mut self,
        with_context: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        let view = chat.read(cx);
        let draft = view.prompt_link_text(cx);
        let link = if with_context {
            if !view.project_scoped {
                return;
            }
            let Some(repository) = self.project_prompt_repository(view.settings_project_path())
            else {
                return;
            };
            hosted_prompt_link_with_context(
                &draft,
                PromptShareContext {
                    repository: &repository,
                    provider: AgentProvider::for_model(&view.model_id),
                    model: &view.model_id,
                },
            )
        } else {
            hosted_prompt_link(&draft)
        };
        self.close_transient_surfaces();
        match link {
            Ok(link) => {
                cx.write_to_clipboard(ClipboardItem::new_string(link));
                self.meetings_hub.update(cx, |hub, cx| {
                    hub.show_notice("Prompt link copied.", false, cx);
                });
            }
            Err(error) => self.app_link_notice(error.to_string(), cx),
        }
        chat.update(cx, |chat, cx| chat.focus_composer(window, cx));
        cx.notify();
    }

    fn open_linked_prompt(
        &mut self,
        link: PromptLink,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match &link.destination {
            PromptDestination::Thread(id) => {
                if self.threads.iter().any(|thread| thread.id == *id) {
                    self.select_thread_by_id(id, cx);
                    self.place_linked_prompt(link.prompt, window, cx);
                } else {
                    self.app_link_notice(APP_LINK_THREAD_MISSING, cx);
                }
            }
            PromptDestination::New {
                repository: None,
                model: None,
            } => {
                self.start_linked_thread(None, None, link.prompt, window, cx);
            }
            PromptDestination::New { repository, .. } => {
                self.resolve_prompt_target(repository.clone(), link, window, cx);
            }
            PromptDestination::PullRequest(pull) => {
                self.resolve_prompt_target(Some(pull.repository.clone()), link, window, cx);
            }
        }
    }

    fn resolve_prompt_target(
        &mut self,
        repository: Option<PromptRepository>,
        link: PromptLink,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let paths = self.prompt_project_paths(cx);
        let matches = crate::app_state::spawn_tokio(cx, matching_prompt_paths(paths, repository));
        self.prompt_link_resolution = Some(cx.spawn_in(window, async move |this, cx| {
            let matches = matches.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let project = this.matching_prompt_project(&matches);
                this.finish_prompt_resolution(link, project, &matches, window, cx);
            });
        }));
    }

    pub(super) fn prompt_project_paths(&self, cx: &App) -> Vec<String> {
        let mut projects = self.listed_projects();
        let mut activity = BTreeMap::new();
        for thread in &self.threads {
            let path = thread.chat.read(cx).settings_project_path();
            let time = activity.entry(path).or_insert(thread.updated_at_ms);
            *time = (*time).max(thread.updated_at_ms);
        }
        projects
            .sort_by_key(|project| std::cmp::Reverse(activity.get(project.path.as_str()).copied()));
        let mut paths: Vec<_> = projects
            .iter()
            .map(|project| project.path.clone())
            .collect();
        for pull in self.known_thread_pulls.values() {
            if !paths.contains(&pull.project) {
                paths.push(pull.project.clone());
            }
        }
        paths
    }

    pub(super) fn matching_prompt_project(
        &self,
        matched_paths: &[String],
    ) -> Option<DesktopProjectLocation> {
        let projects = self.listed_projects();
        matched_paths.iter().find_map(|path| {
            projects
                .iter()
                .find(|project| project.path == *path)
                .cloned()
        })
    }

    pub(super) fn linked_pull_thread(
        &self,
        pull: &PromptPullRequest,
        matched_paths: &[String],
        cx: &App,
    ) -> Option<&DesktopThread> {
        self.threads
            .iter()
            .filter(|thread| {
                self.known_thread_pulls
                    .get(&thread.id)
                    .is_some_and(|observation| {
                        observation.number == pull.number
                            && matched_paths.contains(&observation.project)
                            && thread.branch.as_deref() == Some(observation.branch.as_str())
                            && thread.chat.read(cx).settings_project_path() == observation.project
                    })
            })
            .max_by_key(|thread| thread.updated_at_ms)
    }

    fn finish_prompt_resolution(
        &mut self,
        link: PromptLink,
        project: Option<DesktopProjectLocation>,
        matched_paths: &[String],
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match link.destination {
            PromptDestination::New {
                repository: Some(repository),
                model,
            } if project.is_none() => {
                if let Some(prompt) = link.prompt {
                    self.show_prompt_target(
                        TargetChoice::Repository {
                            repository,
                            model,
                            prompt,
                        },
                        window,
                        cx,
                    );
                }
            }
            PromptDestination::New { model, .. } => {
                self.start_linked_thread(project, model, link.prompt, window, cx)
            }
            PromptDestination::PullRequest(pull) => {
                let known = self
                    .linked_pull_thread(&pull, matched_paths, cx)
                    .map(|thread| thread.id.clone());
                if let Some(id) = known {
                    self.select_thread_by_id(&id, cx);
                    self.place_linked_prompt(link.prompt, window, cx);
                } else {
                    self.show_prompt_target(
                        TargetChoice::PullRequest {
                            pull,
                            project,
                            prompt: link.prompt,
                            checkout_id: uuid::Uuid::new_v4(),
                            running: false,
                            error: None,
                        },
                        window,
                        cx,
                    );
                }
            }
            PromptDestination::Thread(_) => {}
        }
    }

    pub(super) fn start_linked_thread(
        &mut self,
        project: Option<DesktopProjectLocation>,
        model: Option<PromptModel>,
        prompt: Option<LinkedPrompt>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let path = project
            .as_ref()
            .map_or(Path::new(""), |project| Path::new(&project.path));
        let preferred = model
            .as_ref()
            .map(|model| self.resolve_prompt_link_model(path, model));
        if let Some(project) = project {
            self.new_thread_in_project(project.name, project.path, window, cx);
        } else {
            self.new_general_thread(window, cx);
        }
        if let Some((model, exact)) = preferred {
            if let Some(thread) = self.threads.last_mut() {
                if let Some(model) = model {
                    thread.model_id = model.clone();
                    thread.chat.update(cx, |chat, cx| chat.set_model(model, cx));
                    self.persist_and_notify(cx);
                }
            }
            if !exact {
                self.app_link_notice(
                    "The requested model is unavailable here. Check the model before sending.",
                    cx,
                );
            }
        }
        self.place_linked_prompt(prompt, window, cx);
    }

    pub(super) fn resolve_prompt_link_model(
        &self,
        path: &Path,
        model: &PromptModel,
    ) -> (Option<String>, bool) {
        let prefs =
            bruhs_core::agent_extensions::availability::ProviderAvailability::load_for_project(
                path,
            )
            .ok();
        let (id, exact) = model.resolve(
            self.thread_catalog
                .models
                .iter()
                .filter(|choice| {
                    prefs
                        .as_ref()
                        .is_some_and(|prefs| prefs.enabled(choice.provider))
                })
                .map(|choice| (choice.provider, choice.id.as_str())),
        );
        (id.map(str::to_owned), exact)
    }

    fn place_linked_prompt(
        &self,
        prompt: Option<LinkedPrompt>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let (Some(prompt), Some(thread)) = (prompt, self.active_thread()) {
            thread.chat.update(cx, |chat, cx| {
                chat.receive_linked_prompt(prompt, window, cx)
            });
        }
    }

    fn show_prompt_target(
        &mut self,
        choice: TargetChoice,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_top_layer(TopLayer::PromptTarget);
        self.prompt_link_choice = Some(choice);
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    fn dismiss_prompt_target(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(
            &self.prompt_link_choice,
            Some(TargetChoice::PullRequest { running: true, .. })
        ) {
            return;
        }
        self.prompt_link_choice = None;
        if self.top_layer == Some(TopLayer::PromptTarget) {
            self.top_layer = None;
            cx.notify();
        }
    }

    fn clone_prompt_target(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (repo, target) = match self.prompt_link_choice.take() {
            Some(TargetChoice::Repository {
                repository,
                model,
                prompt,
            }) => (repository, CloneTarget::Repository { model, prompt }),
            Some(TargetChoice::PullRequest {
                pull,
                project: None,
                prompt,
                ..
            }) => (
                pull.repository.clone(),
                CloneTarget::PullRequest { pull, prompt },
            ),
            other => {
                self.prompt_link_choice = other;
                return;
            }
        };
        self.open_link_clone(&repo, target, window, cx);
    }

    pub(super) fn finish_link_clone(
        &mut self,
        project: DesktopProjectLocation,
        target: CloneTarget,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match target {
            CloneTarget::Repository { model, prompt } => {
                self.start_linked_thread(Some(project), model, Some(prompt), window, cx)
            }
            CloneTarget::PullRequest { pull, prompt } => self.show_prompt_target(
                TargetChoice::PullRequest {
                    pull,
                    project: Some(project),
                    prompt,
                    checkout_id: uuid::Uuid::new_v4(),
                    running: false,
                    error: None,
                },
                window,
                cx,
            ),
        }
    }

    fn use_prompt_without_project(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (model, prompt) = match self.prompt_link_choice.take() {
            Some(TargetChoice::Repository { model, prompt, .. }) => (model, prompt),
            Some(TargetChoice::PullRequest {
                pull,
                prompt,
                running: false,
                ..
            }) => {
                let mut prompt = prompt.unwrap_or(LinkedPrompt {
                    text: String::new(),
                    removed_hidden_characters: false,
                });
                prompt.text = if prompt.text.is_empty() {
                    pull.url()
                } else {
                    format!("{}\n\n{}", pull.url(), prompt.text)
                };
                (None, prompt)
            }
            other => {
                self.prompt_link_choice = other;
                return;
            }
        };
        self.start_linked_thread(None, model, Some(prompt), window, cx);
    }

    fn checkout_prompt_pull(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(TargetChoice::PullRequest {
            pull,
            project: Some(project),
            checkout_id,
            running,
            error,
            ..
        }) = self.prompt_link_choice.as_mut()
        else {
            return;
        };
        if *running {
            return;
        }
        *running = true;
        *error = None;
        let pull = pull.clone();
        let project = project.clone();
        let checkout_id = *checkout_id;
        let path = project.path.clone();
        let setup = self.setup_steps(&path);
        let checkout = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::git::checkout_linked_pull(Path::new(&path), &pull, checkout_id, &setup)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = checkout.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((path, branch)) => {
                    let Some(TargetChoice::PullRequest { pull, prompt, .. }) =
                        this.prompt_link_choice.take()
                    else {
                        return;
                    };
                    this.new_thread_in_project(project.name, project.path.clone(), window, cx);
                    if let Some(thread) = this.active_thread() {
                        let chat = thread.chat.clone();
                        let id = thread.id.clone();
                        chat.update(cx, |chat, cx| {
                            chat.lock_place(Some(path_to_string(&path)), Some(branch.clone()), cx);
                        });
                        this.known_thread_pulls.insert(
                            id.clone(),
                            pull_settlement::Observation::linked(project.path, branch, pull.number),
                        );
                        this.place_linked_prompt(prompt, window, cx);
                        this.launch_thread_hooks(&id, false, cx);
                    }
                    this.persist_and_notify(cx);
                }
                Err(failure) => {
                    if let Some(TargetChoice::PullRequest { running, error, .. }) =
                        this.prompt_link_choice.as_mut()
                    {
                        *running = false;
                        *error = Some(failure.to_string());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_prompt_target(&self, _: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let Some(choice) = &self.prompt_link_choice else {
            return div().into_any_element();
        };
        let (title, detail, prompt, checkout, running, error) = match choice {
            TargetChoice::Repository {
                repository, prompt, ..
            } => (
                "Repository not added",
                format!(
                    "Add {} to open this draft in its project.",
                    repository.slug()
                ),
                Some(prompt),
                false,
                false,
                None,
            ),
            TargetChoice::PullRequest {
                pull,
                project,
                prompt,
                running,
                error,
                ..
            } => (
                "Open pull request",
                if let Some(project) = project {
                    format!(
                        "Check out {} #{} in a new worktree for {}.",
                        pull.repository.slug(),
                        pull.number,
                        project.name
                    )
                } else {
                    format!(
                        "Add {} to check out pull request #{}.",
                        pull.repository.slug(),
                        pull.number
                    )
                },
                prompt.as_ref(),
                project.is_some(),
                *running,
                error.as_deref(),
            ),
        };
        let button = |id, label, variant| {
            web_button(
                id,
                variant,
                WebButtonSize::Small,
                ButtonState::disabled_if(running),
            )
            .child(label)
        };
        let primary = if checkout {
            on_press_view(
                button(
                    "linked-prompt-checkout",
                    if running {
                        "Checking out…"
                    } else {
                        "Check out"
                    },
                    WebButtonVariant::Outline,
                ),
                cx,
                |this, window, cx| this.checkout_prompt_pull(window, cx),
            )
        } else {
            on_press_view(
                button(
                    "linked-prompt-clone",
                    "Clone repository…",
                    WebButtonVariant::Outline,
                ),
                cx,
                |this, window, cx| this.clone_prompt_target(window, cx),
            )
        };
        dialog_backdrop("linked-prompt-target-overlay", DialogPlacement::Centered)
            .on_click(cx.listener(|this, _, _, cx| this.dismiss_prompt_target(cx)))
            .child(
                dialog_card(
                    "linked-prompt-target",
                    title,
                    Role::Dialog,
                    &self.blur_scrim,
                    cx,
                )
                .debug_selector(|| "linked-prompt-target".into())
                .w(px(510.))
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .track_focus(&self.modal_start_focus_handle)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape"
                        && event.keystroke.modifiers == Modifiers::default()
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.dismiss_prompt_target(cx);
                    }
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.trap_modal_tab_focus(event, window, cx)
                }))
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(
                    div()
                        .id("linked-prompt-target-title")
                        .role(Role::Heading)
                        .text_color(theme::rambutan_100())
                        .child(title),
                )
                .child(div().text_color(theme::rambutan_300()).child(detail))
                .when(checkout, |card| {
                    card.child(
                        div()
                            .text_color(theme::rambutan_300())
                            .child(PULL_CHECKOUT_SETUP_NOTICE),
                    )
                })
                .when_some(prompt, |card, prompt| {
                    card.child(
                        div()
                            .truncate()
                            .text_color(theme::rambutan_400())
                            .child(prompt.text.clone()),
                    )
                })
                .when_some(error, |card, error| {
                    card.child(
                        div()
                            .id("linked-prompt-target-error")
                            .role(Role::Alert)
                            .text_color(theme::dragonfruit_400())
                            .child(error.to_owned()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap_2()
                        .child(on_press_view(
                            button("linked-prompt-cancel", "Cancel", WebButtonVariant::Ghost),
                            cx,
                            |this, _, cx| this.dismiss_prompt_target(cx),
                        ))
                        .child(on_press_view(
                            button(
                                "linked-prompt-general",
                                "Use without project",
                                WebButtonVariant::Ghost,
                            ),
                            cx,
                            |this, window, cx| this.use_prompt_without_project(window, cx),
                        ))
                        .child(primary),
                ),
            )
            .into_any_element()
    }

    fn app_link_notice(&mut self, message: impl Into<String>, cx: &mut Context<'_, Self>) {
        self.meetings_hub
            .update(cx, |hub, cx| hub.show_notice(message, true, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AppView, Workspace};
    use super::{APP_LINK_THREAD_MISSING, APP_LINK_UNKNOWN};
    use crate::{app_state::DesktopState, theme};
    use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let entity = Rc::new(RefCell::new(None));
        let captured = entity.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *captured.borrow_mut() = Some(workspace.clone());
            bruhs_ui::Root::new(workspace, window, cx)
        });
        let workspace = entity.borrow().clone().expect("workspace");
        (workspace, cx)
    }

    fn notice(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
        workspace.read_with(cx, |workspace, cx| {
            workspace
                .meetings_hub
                .read(cx)
                .notice
                .as_ref()
                .map(|notice| notice.message.clone())
        })
    }

    #[gpui::test]
    fn missing_repository_waits_for_a_destination_choice(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            let before = workspace.threads.len();
            let link = super::parse_prompt_link(
                "bruhs://new?repo=example%2Fdemo&prompt=Review%20this%20diff",
            )
            .unwrap()
            .unwrap();
            workspace.finish_prompt_resolution(link, None, &[], window, cx);
            assert_eq!(workspace.threads.len(), before);
            assert_eq!(workspace.top_layer, Some(super::TopLayer::PromptTarget));
            workspace.use_prompt_without_project(window, cx);
            assert_eq!(workspace.threads.len(), before + 1);
            let thread = workspace.active_thread().unwrap();
            assert!(thread.project_name.is_empty());
            assert_eq!(
                thread.chat.read(cx).prompt_link_text(cx),
                "Review this diff"
            );
            assert!(thread.chat.read(cx).messages.is_empty());
        });
    }

    #[gpui::test]
    fn pull_link_reuses_the_known_thread_for_that_repository(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            let project = super::DesktopProjectLocation {
                path: "/example/demo".into(),
                name: "demo".into(),
            };
            workspace.new_thread_in_project(project.name.clone(), project.path.clone(), window, cx);
            let thread = workspace.threads.last_mut().unwrap();
            let id = thread.id.clone();
            let branch = "bruhs/pr-42-test".to_owned();
            thread.branch = Some(branch.clone());
            workspace.known_thread_pulls.insert(
                id.clone(),
                super::pull_settlement::Observation::linked(project.path.clone(), branch, 42),
            );
            workspace.new_general_thread(window, cx);
            let count = workspace.threads.len();
            let link = super::parse_prompt_link("bruhs://new?pr=example%2Fdemo%2342")
                .unwrap()
                .unwrap();
            let matches = [project.path.clone()];
            workspace.finish_prompt_resolution(link, Some(project), &matches, window, cx);
            assert_eq!(workspace.active_thread_id.as_deref(), Some(id.as_str()));
            assert_eq!(workspace.threads.len(), count);
            assert!(workspace.prompt_link_choice.is_none());
        });
    }

    #[gpui::test]
    fn links_open_views_and_threads_like_their_web_routes(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_app_link("bruhs://open/inbox", window, cx);
            assert_eq!(workspace.active_view, AppView::Inbox);
            workspace.open_app_link("bruhs://open/workbench", window, cx);
            assert_eq!(workspace.active_view, AppView::Workbench);
            let id = workspace.threads[0].id.clone();
            workspace.open_app_link(&format!("bruhs://thread/{id}"), window, cx);
            assert_eq!(workspace.active_view, AppView::Chat);
            assert_eq!(workspace.active_thread_id.as_deref(), Some(id.as_str()));
            workspace.open_app_link("bruhs://thread/not-here", window, cx);
        });
        assert_eq!(
            notice(&workspace, cx).as_deref(),
            Some(APP_LINK_THREAD_MISSING)
        );
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_app_link("bruhs://open/nowhere", window, cx);
        });
        assert_eq!(notice(&workspace, cx).as_deref(), Some(APP_LINK_UNKNOWN));
    }

    #[gpui::test]
    fn a_prompt_link_is_a_fresh_draft_and_keeps_the_existing_thread(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_general_thread(window, cx);
            let existing = workspace.active_thread().unwrap().chat.clone();
            existing.update(cx, |chat, cx| {
                chat.set_composer_draft("My original draft".into(), window, cx);
            });
            let before = workspace.threads.len();
            workspace.open_app_link(
                "bruhs://new?prompt=%2Fclear&permission=full&branch=other",
                window,
                cx,
            );
            assert_eq!(workspace.threads.len(), before + 1);
            let target = workspace.active_thread().unwrap();
            assert!(target.project_name.is_empty());
            let chat = target.chat.read(cx);
            assert_eq!(chat.prompt_link_text(cx).as_ref(), "/clear");
            assert_eq!(chat.permission_mode, workspace.permission_mode);
            assert!(chat.messages.is_empty());
            assert_eq!(
                existing.read(cx).prompt_link_text(cx).as_ref(),
                "My original draft"
            );
        });
    }

    #[gpui::test]
    fn a_thread_prompt_link_waits_for_the_existing_draft_choice(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        let (id, chat) = workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_general_thread(window, cx);
            let thread = workspace.active_thread().unwrap();
            let id = thread.id.clone();
            let chat = thread.chat.clone();
            chat.update(cx, |chat, cx| {
                chat.set_composer_draft("Original draft".into(), window, cx);
            });
            workspace.new_general_thread(window, cx);
            (id, chat)
        });
        let link = format!("bruhs://thread/{id}?prompt=Review%E2%80%8B+this");
        let open = |cx: &mut VisualTestContext| {
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.open_app_link(&link, window, cx);
                assert_eq!(workspace.active_thread_id.as_deref(), Some(id.as_str()));
            });
            cx.run_until_parked();
        };
        open(cx);
        let keep = cx.debug_bounds("desktop-linked-prompt-dismiss").unwrap();
        cx.simulate_click(keep.center(), Modifiers::default());
        cx.run_until_parked();
        chat.read_with(cx, |chat, cx| {
            assert_eq!(chat.prompt_link_text(cx).as_ref(), "Original draft");
        });
        assert!(cx.debug_bounds("desktop-linked-prompt-banner").is_none());

        open(cx);
        let replace = cx.debug_bounds("desktop-linked-prompt-replace").unwrap();
        cx.simulate_click(replace.center(), Modifiers::default());
        cx.run_until_parked();
        chat.read_with(cx, |chat, cx| {
            assert_eq!(chat.prompt_link_text(cx).as_ref(), "Review this");
            assert!(chat.messages.is_empty());
        });
        assert!(cx.debug_bounds("desktop-linked-prompt-banner").is_some());
        assert!(cx.debug_bounds("desktop-linked-prompt-replace").is_none());
    }

    #[gpui::test]
    fn the_palette_copies_a_portable_prompt_draft(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_general_thread(window, cx);
            workspace
                .active_thread()
                .unwrap()
                .chat
                .update(cx, |chat, cx| {
                    chat.set_composer_draft("Review café & planning".into(), window, cx);
                });
            assert!(workspace
                .filtered_command_actions(cx)
                .iter()
                .any(|action| action.id == "copy-prompt-link"));
            workspace.execute_command_action("copy-prompt-link", window, cx);
            let copied = cx.read_from_clipboard().unwrap().text().unwrap();
            assert!(copied.starts_with("https://bruhs.ai/prompt#/new?prompt="));
            let draft = super::parse_prompt_link(&copied).unwrap().unwrap();
            assert_eq!(
                draft.destination,
                super::PromptDestination::New {
                    repository: None,
                    model: None
                }
            );
            assert_eq!(draft.prompt.unwrap().text, "Review café & planning");
        });
    }

    #[gpui::test]
    async fn the_palette_adds_context_only_when_the_user_chooses_it(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        for args in [
            &["init", "-q"][..],
            &["remote", "add", "origin", "git@github.com:Example/Docs.git"],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(directory.path())
                .status()
                .unwrap()
                .success());
        }
        let (workspace, cx) = workspace(cx);
        let project = super::DesktopProjectLocation {
            path: directory.path().to_str().unwrap().into(),
            name: "Review workspace".into(),
        };
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_thread_in_project(project.name.clone(), project.path.clone(), window, cx);
            workspace.remember_project(project.clone(), cx);
            workspace
                .active_thread()
                .unwrap()
                .chat
                .update(cx, |chat, cx| {
                    chat.set_model("gpt-5.4", cx);
                    chat.set_composer_draft("Review the release notes.".into(), window, cx);
                });
        });
        cx.condition(&workspace, |workspace, _| {
            workspace.project_prompt_repository(&project.path).is_some()
        })
        .await;
        workspace.update_in(cx, |workspace, window, cx| {
            assert!(workspace
                .filtered_command_actions(cx)
                .iter()
                .any(|action| action.id == "copy-prompt-link-with-context"));
            workspace.execute_command_action("copy-prompt-link-with-context", window, cx);
            let link = cx.read_from_clipboard().unwrap().text().unwrap();
            let parsed = super::parse_prompt_link(&link).unwrap().unwrap();
            let super::PromptDestination::New {
                repository: Some(repository),
                model: Some(model),
            } = parsed.destination
            else {
                panic!("expected share context")
            };
            assert_eq!(repository.slug(), "Example/Docs");
            assert_eq!(model.provider.as_deref(), Some("codex"));
            assert_eq!(model.id.as_deref(), Some("gpt-5.4"));
            assert_eq!(parsed.prompt.unwrap().text, "Review the release notes.");
            assert!(!link.contains(directory.path().to_str().unwrap()));
        });
    }
}
