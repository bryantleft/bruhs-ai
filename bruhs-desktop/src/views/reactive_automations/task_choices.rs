//! Projects and provider models for a scheduled task on the selected host.
use super::*;
use crate::views::model_browser::{self, Browser, FAVORITES, Favorites};
pub(in crate::views) type ToggleFavorite = Rc<dyn Fn(&str, &mut App)>;
use bruhs_models::agent::{AgentProvider, ModelId};
use bruhs_models::delivery::writer_catalog::WriterCatalog;
use bruhs_ui::dropdown::{Dropdown, DropdownGroup, DropdownOption, Listbox, saved_option_value};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskField {
    Project,
    Model,
    Thread,
}

impl TaskField {
    fn search_label(self) -> &'static str {
        match self {
            Self::Project => "Search projects or enter a folder path",
            Self::Model => "Search models or enter a model ID",
            Self::Thread => "Search all threads…",
        }
    }
}

pub(super) struct TaskPickerState {
    pub(super) open: Option<TaskField>,
    project_trigger: FocusHandle,
    model_trigger: FocusHandle,
    model_tab: Option<String>,
    model_rail: FocusHandle,
    thread_trigger: FocusHandle,
    thread_results: Vec<TaskThreadChoice>,
    thread_search: Option<Task<()>>,
    thread_error: Option<String>,
    pub(super) chosen_thread: Option<TaskThreadChoice>,
    pub(super) original_thread: Option<TaskThreadChoice>,
    reset_thread: Option<bruhs_models::ids::ThreadId>,
    search: Entity<InputState>,
    _search_changes: gpui::Subscription,
    catalog_project: Option<String>,
    catalog: Option<Result<WriterCatalog, String>>,
    listing_request: Option<uuid::Uuid>,
    follow_project_model: bool,
}

impl TaskPickerState {
    pub(super) fn model_problem(&self, project: &str, model: &str) -> Option<&'static str> {
        let provider = AgentProvider::for_model(model);
        if provider.is_external() {
            return Some(
                "This provider cannot run unattended tasks. Choose a Claude or Codex model, or a conversation using one.",
            );
        }
        let catalog = self.catalog.as_ref()?.as_ref().ok()?;
        if self.catalog_project.as_deref() != Some(project) {
            return None;
        }
        if catalog.models.is_empty() {
            return Some("Enable Claude or Codex for this project in Providers & updates.");
        }
        if !catalog.models.iter().any(|row| row.provider == provider) {
            return Some(
                "The selected provider is disabled for this project. Choose another model or enable the provider in Settings.",
            );
        }
        None
    }

    pub(super) fn displayed_thread(&self) -> Option<&TaskThreadChoice> {
        self.chosen_thread
            .as_ref()
            .or(self.original_thread.as_ref())
    }

    pub(super) fn clear_thread(&mut self) {
        self.chosen_thread = None;
        if let Some(original) = self.original_thread.take() {
            self.reset_thread = Some(original.thread_id);
        }
    }

    pub(super) fn destination_edit(&self, mode: TaskConversation) -> Option<TaskDestinationEdit> {
        if !mode.has_destination() {
            return None;
        }
        if let Some(choice) = &self.chosen_thread {
            Some(if choice.desktop_model.is_some() {
                TaskDestinationEdit::Desktop {
                    thread_id: choice.thread_id.clone(),
                }
            } else {
                TaskDestinationEdit::Existing {
                    thread_id: choice.thread_id.clone(),
                }
            })
        } else {
            self.reset_thread
                .clone()
                .map(|previous| TaskDestinationEdit::FirstRun { previous })
        }
    }

    fn trigger(&self, field: TaskField) -> &FocusHandle {
        match field {
            TaskField::Project => &self.project_trigger,
            TaskField::Model => &self.model_trigger,
            TaskField::Thread => &self.thread_trigger,
        }
    }

    pub(super) fn new(
        is_new: bool,
        window: &mut Window,
        cx: &mut Context<'_, ReactiveAutomations>,
    ) -> Self {
        let search = edit_input("", window, cx);
        let subscription = cx.subscribe_in(&search, window, |panel, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                panel.search_task_threads(cx);
                cx.notify();
            }
        });
        Self {
            open: None,
            project_trigger: cx.focus_handle().tab_stop(true),
            model_trigger: cx.focus_handle().tab_stop(true),
            model_tab: None,
            model_rail: cx.focus_handle().tab_stop(true),
            thread_trigger: cx.focus_handle().tab_stop(true),
            thread_results: Vec::new(),
            thread_search: None,
            thread_error: None,
            chosen_thread: None,
            original_thread: None,
            reset_thread: None,
            search,
            _search_changes: subscription,
            catalog_project: None,
            catalog: None,
            listing_request: None,
            follow_project_model: is_new,
        }
    }
}

// The option value stays stable while the user types. Only a chosen draft value is saved.
const TYPED: &str = "\0typed";
const CURRENT: &str = "\0current";

fn option(key: &'static str, label: &str, detail: Option<&str>) -> DropdownOption {
    DropdownOption {
        value: key,
        label: label.into(),
        description: detail.map(str::to_owned),
    }
}

impl setup::AutomationEditor {
    fn select_task_choice(
        &mut self,
        field: TaskField,
        key: &str,
        defaults: &HashMap<String, String>,
        default_model: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.picker.open != Some(field) {
            return;
        }
        self.picker.open = None;
        if key == CURRENT {
            return;
        }
        let selected = if key == TYPED {
            value(&self.picker.search, cx).trim().to_owned()
        } else {
            key.to_owned()
        };
        match field {
            TaskField::Project if self.project_path(cx) != selected => {
                self.picker.clear_thread();
                self.set_project_path(&selected, window, cx);
                self.checkout.close();
                if self.picker.follow_project_model {
                    self.set_task_model(
                        defaults
                            .get(&selected)
                            .map(String::as_str)
                            .unwrap_or(default_model),
                        window,
                        cx,
                    );
                }
            }
            TaskField::Project | TaskField::Thread => {}
            TaskField::Model => {
                self.set_task_model(&selected, window, cx);
                self.picker.follow_project_model = false;
            }
        }
    }
}

impl ReactiveAutomations {
    /// The window's modal Tab handler runs before the popup's own key handler.
    /// Resume traversal at the field that owns the popup.
    pub(crate) fn close_task_picker_for_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(Editor::Automation(editor)) = &mut self.editor {
            if editor.host_menu_open {
                editor.host_menu_open = false;
                cx.notify();
            }
            if let Some(field) = editor.picker.open.take() {
                editor.picker.trigger(field).focus(window, cx);
                cx.notify();
            }
        }
    }

    pub(super) fn refresh_task_models(&mut self, refresh: bool, cx: &mut Context<'_, Self>) {
        let Some(Editor::Automation(editor)) = &mut self.editor else {
            return;
        };
        if editor.locked_agent().is_some() {
            return;
        }
        let project = editor.project_path(cx);
        let picker = &mut editor.picker;
        if picker.catalog_project.as_ref() == Some(&project) {
            if !refresh || picker.listing_request.is_some() {
                return;
            }
        } else {
            picker.catalog = None;
            picker.catalog_project = Some(project.clone());
        }
        let request = uuid::Uuid::new_v4();
        picker.listing_request = Some(request);
        let client = self.client.clone();
        let host = self.linked_task.as_ref().map(|edit| edit.host.clone());
        let listing = spawn_tokio(cx, async move {
            if let Some(host) = host {
                host.models(&project).await
            } else {
                client.task_models(&project).await.map_err(reactive_error)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = listing.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(Editor::Automation(editor)) = &mut this.editor {
                    if editor.picker.listing_request == Some(request) {
                        editor.picker.catalog = Some(result);
                        editor.picker.listing_request = None;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn toggle_task_picker(
        &mut self,
        field: TaskField,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(Editor::Automation(editor)) = &mut self.editor else {
            return;
        };
        if editor.picker.open == Some(field) {
            editor.picker.open = None;
        } else {
            editor.picker.open = Some(field);
            editor.security.encoding_open = false;
            editor.checkout.close();
            editor.open_filter = None;
            editor.task_thread_open = false;
            editor.picker.search.update(cx, |input, cx| {
                input.set_placeholder(field.search_label(), window, cx);
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            if field == TaskField::Model {
                editor.picker.model_tab = None;
                self.refresh_task_models(true, cx);
            }
        }
        if field == TaskField::Thread {
            self.search_task_threads(cx);
        }
        cx.notify();
    }

    fn search_task_threads(&mut self, cx: &mut Context<'_, Self>) {
        let Some(Editor::Automation(editor)) = &mut self.editor else {
            return;
        };
        if editor.picker.open != Some(TaskField::Thread) {
            return;
        }
        let project = editor.project_path(cx);
        let query = value(&editor.picker.search, cx);
        let search = query.trim().to_lowercase();
        // Bound threads are already in the snapshot. The host search also finds
        // earlier conversations after their originating schedule has gone away.
        editor.picker.thread_results = self
            .snapshot
            .iter()
            .flat_map(|snapshot| {
                snapshot.subscriptions.iter().filter_map(|sub| {
                    let agent = snapshot
                        .agents
                        .iter()
                        .find(|a| a.id == sub.agent_id && a.project_path == project)?;
                    let id = snapshot.task_threads.get(&sub.id)?;
                    Some(
                        snapshot
                            .selected_task_threads
                            .as_ref()
                            .and_then(|all| all.get(&sub.id))
                            .cloned()
                            .unwrap_or_else(|| TaskThreadChoice {
                                agent_model: Some(agent.model.clone()),
                                desktop_model: None,
                                thread_id: id.clone(),
                                title: sub.title(agent).into(),
                                agent_id: agent.id.clone(),
                                checkout: sub.task_checkout.clone(),
                            }),
                    )
                })
            })
            .filter(|choice| choice.title.to_lowercase().contains(&search))
            .collect();
        editor.picker.thread_error = None;
        editor.picker.thread_search = None;
        if cfg!(test) || project.is_empty() {
            return;
        }
        let client = self.client.clone();
        let host = self.linked_task.as_ref().map(|edit| edit.host.clone());
        editor.picker.thread_search = Some(cx.spawn(async move |panel, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            let request = cx.update(|cx| {
                spawn_tokio(
                    cx,
                    async move {
                        if let Some(host) = host { host.conversations(&project, &query).await }
                        else { client.task_threads(&project, &query).await.map_err(reactive_error) }
                    },
                )
            });
            let result = request.await;
            let _ = panel.update(cx, |p, cx| {
                let Some(Editor::Automation(e)) = &mut p.editor else {
                    return;
                };
                if e.picker.open != Some(TaskField::Thread) {
                    return;
                }
                match result {
                    Ok(threads) => e.picker.thread_results = threads,
                    Err(error) => e.picker.thread_error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn task_thread_picker(
        &self,
        editor: &setup::AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let state = &editor.picker;
        let selected = state.displayed_thread();
        let mut choices = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let current = selected.map(|c| {
            state
                .thread_results
                .iter()
                .find(|item| item.thread_id == c.thread_id)
                .unwrap_or(c)
        });
        for choice in current.into_iter().chain(&state.thread_results) {
            if seen.insert(&choice.thread_id) {
                choices.push(choice.clone());
            }
        }
        let mut options = Vec::new();
        if state.open != Some(TaskField::Thread) || value(&state.search, cx).trim().is_empty() {
            options.push(option("new_thread", "New thread each run", None));
        }
        let mut key = "";
        let mut selection = HashMap::new();
        for (i, choice) in choices.into_iter().enumerate() {
            // A bounded set of row keys; thread IDs themselves are never interned.
            let row = saved_option_value(&format!("destination-{i}"));
            if selected.is_some_and(|c| c.thread_id == choice.thread_id) {
                key = row
            }
            let title = if choice.title.trim().is_empty() {
                "Untitled thread"
            } else {
                &choice.title
            };
            options.push(option(row, title, None));
            selection.insert(row, choice);
        }
        let toggle = cx.weak_entity();
        let select = toggle.clone();
        let open = state.open == Some(TaskField::Thread);
        let picker = Dropdown::new(
            "task-destination-thread",
            "Destination thread",
            key,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |w, cx| {
                let _ = toggle.update(cx, |p, cx| p.toggle_task_picker(TaskField::Thread, w, cx));
            }),
            Rc::new(move |key, _, cx| {
                let _ = select.update(cx, |p, cx| {
                    let Some(Editor::Automation(e)) = &mut p.editor else {
                        return;
                    };
                    if e.picker.open != Some(TaskField::Thread) || p.busy {
                        return;
                    }
                    if key == "new_thread" {
                        e.subscription_mut().subscription.task_conversation =
                            TaskConversation::NewThread;
                        e.picker.clear_thread();
                    } else if let Some(choice) = selection.get(key) {
                        if e.picker.original_thread.as_ref() != Some(choice)
                            || e.picker.chosen_thread.is_some()
                        {
                            e.picker.chosen_thread = Some(choice.clone());
                        }
                    }
                    e.picker.open = None;
                    cx.notify();
                });
            }),
        )
        .leading_icon(crate::assets::MESSAGE_SQUARE_ICON_PATH)
        .empty_label("No threads found.")
        .placeholder("Started by the first run")
        .searchable(
            &state.search,
            TaskField::Thread.search_label(),
            &state.thread_trigger,
        );
        div().flex().flex_col().gap_1().child(picker).when_some(
            state.thread_error.as_ref().filter(|_| open),
            |el, error| {
                el.child(
                    muted(error.clone())
                        .id("task-thread-search-error")
                        .role(Role::Status),
                )
            },
        )
    }

    pub(super) fn render_task_choices(
        &self,
        editor: &setup::AutomationEditor,
        mut projects: Vec<DropdownOption>,
        project_models: HashMap<String, String>,
        default_model: String,
        favorites: Favorites,
        toggle_favorite: ToggleFavorite,
        cx: &Context<'_, Self>,
    ) -> (Div, Div) {
        let project = editor.project_path(cx);
        let model = editor.task_model(cx);
        let search = value(&editor.picker.search, cx);
        let query = search.trim();
        let mut project_field = div();
        let mut model_field = div().flex().flex_col().gap_1();

        let project_key = if project.is_empty() || projects.iter().any(|p| p.value == project) {
            project.as_str()
        } else {
            projects.push(option(CURRENT, &project, None));
            CURRENT
        };
        if editor.picker.open == Some(TaskField::Project)
            && self.linked_task.is_none()
            && !query.is_empty()
            && !projects.iter().any(|p| p.value == query)
            && (std::path::Path::new(query).is_absolute()
                || !projects.iter().any(|p| p.matches_search(None, query)))
        {
            projects.push(option(TYPED, &format!("Use folder “{query}”"), Some(query)));
        }
        let catalog = editor
            .picker
            .catalog
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        let models = catalog
            .map(|catalog| catalog.models.as_slice())
            .unwrap_or_default();
        let mut groups: Vec<DropdownGroup> = Vec::new();
        for provider in [AgentProvider::Claude, AgentProvider::Codex] {
            let options: Vec<_> = models
                .iter()
                .filter(|m| m.provider == provider)
                .map(|m| option(saved_option_value(&m.id), &m.label, None))
                .collect();
            if !options.is_empty() {
                groups.push(DropdownGroup {
                    label: Some(crate::views::workspace::provider_label(provider)),
                    mark: Some(crate::views::workspace::provider_mark(provider, cx)),
                    options,
                });
            }
        }
        let mut extra = Vec::new();
        let selected_model = catalog.and_then(|c| c.model(&model));
        let model_key = selected_model.map_or(model.as_str(), |m| m.id.as_str());
        let model_label = selected_model
            .map_or_else(|| model.clone(), |m| m.label.clone());
        let browser = Browser::new(
            models.iter().map(|m| m.id.as_str()),
            editor.picker.model_tab.as_deref(),
            &model,
            &favorites,
        );
        let browse = editor.picker.open == Some(TaskField::Model) && query.is_empty();
        if browse {
            browser.filter_groups(&mut groups, &favorites);
        }
        if editor.picker.open == Some(TaskField::Model)
            && query != model
            && query.parse::<ModelId>().is_ok()
            && models
                .iter()
                .any(|m| m.provider == AgentProvider::for_model(query))
            && !models.iter().any(|m| m.id == query)
            && (bruhs_models::agent::custom_model_selection(query).is_some()
                || !groups.iter().any(|g| {
                    g.options
                        .iter()
                        .any(|option| option.matches_search(g.label, query))
                }))
        {
            extra.push(option(TYPED, &format!("Use model “{query}”"), Some(query)));
        }
        if !extra.is_empty() {
            groups.push(DropdownGroup {
                label: Some("Custom"),
                mark: None,
                options: extra,
            });
        }

        let project_groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: projects,
        }];
        let defaults = Rc::new(project_models);
        for (field, id, label, key, groups) in [
            (
                TaskField::Project,
                "task-project",
                "Project",
                project_key,
                project_groups,
            ),
            (
                TaskField::Model,
                "task-model",
                "Model",
                model_key,
                groups,
            ),
        ] {
            if field == TaskField::Model {
                if let Some(choice) = editor.picker.displayed_thread() {
                    let model = choice.desktop_model.as_deref().or(choice.agent_model.as_deref()).unwrap_or_else(|| {
                        self.snapshot
                            .iter()
                            .flat_map(|s| &s.agents)
                            .find(|a| a.id == choice.agent_id)
                            .map_or("Unavailable", |a| a.model.as_str())
                    });
                    model_field = model_field.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(muted("Model"))
                            .child(muted(model.to_owned())),
                    );
                    continue;
                }
            }
            let toggle = cx.weak_entity();
            let select = toggle.clone();
            let defaults = defaults.clone();
            let default_model = default_model.clone();
            let open = editor.picker.open == Some(field);
            let picker = Dropdown::new(
                id,
                &format!("Task {}", label.to_lowercase()),
                key,
                groups,
                if open {
                    Listbox::Popover
                } else {
                    Listbox::Closed
                },
                Rc::new(move |window, cx| {
                    let _ =
                        toggle.update(cx, |this, cx| this.toggle_task_picker(field, window, cx));
                }),
                Rc::new(move |key, window, cx| {
                    let _ = select.update(cx, |this, cx| {
                        if this.busy {
                            return;
                        }
                        let Some(Editor::Automation(editor)) = &mut this.editor else {
                            return;
                        };
                        editor.select_task_choice(
                            field,
                            key,
                            &defaults,
                            &default_model,
                            window,
                            cx,
                        );
                        cx.notify();
                    });
                }),
            )
            .searchable(
                &editor.picker.search,
                field.search_label(),
                editor.picker.trigger(field),
            )
            .placeholder(if field == TaskField::Project {
                "Select a project"
            } else {
                "Select a model"
            });
            let picker = if field == TaskField::Model {
                let panel = cx.weak_entity();
                let rail = browse.then(|| {
                    let focus = &editor.picker.model_rail;
                    let rail = browser.rail(
                        "task-model",
                        focus,
                        editor.picker.search.focus_handle(cx),
                        |id| (id.to_owned(), None),
                        move |tab, window, cx| {
                            let _ = panel.update(cx, |panel, cx| {
                                if let Some(Editor::Automation(e)) = &mut panel.editor {
                                    e.picker.model_tab = Some(tab);
                                    e.picker
                                        .search
                                        .update(cx, |input, cx| input.focus(window, cx));
                                    cx.notify();
                                }
                            });
                        },
                    );
                    (rail, focus.clone())
                });
                let stars = favorites.clone();
                let toggle = toggle_favorite.clone();
                let active = browser.active.clone();
                let panel = cx.weak_entity();
                picker
                    .selection_label(model_label.clone(), None)
                    .empty_label(if browse && active == FAVORITES {
                        "No favorite models for this project."
                    } else {
                        "No matching models."
                    })
                    .option_accessories(rail, move |id| {
                        if id == TYPED {
                            return None;
                        }
                        let selected = stars.contains(&model_browser::favorite_key(id));
                        let model = id.to_owned();
                        let toggle = toggle.clone();
                        let panel = panel.clone();
                        let active = active.clone();
                        Some(model_browser::star(
                            "task-model",
                            id,
                            selected,
                            move |_, cx| {
                                let _ = panel.update(cx, |panel, _| {
                                    if let Some(Editor::Automation(e)) = &mut panel.editor {
                                        e.picker.model_tab = Some(active.clone());
                                    }
                                });
                                toggle(&model, cx);
                            },
                        ))
                    })
            } else {
                picker
            };
            let control = div().min_w_0().flex().flex_col().gap_1().child(muted(label)).child(picker);
            if field == TaskField::Project {
                project_field = control;
            } else {
                model_field = control;
            }
        }
        let status = if editor.picker.open == Some(TaskField::Model)
            && editor.picker.listing_request.is_some()
        {
            Some("Checking models on the task host…".to_owned())
        } else if let Some(Err(error)) = &editor.picker.catalog {
            Some(format!(
                "Could not load task models. {error} Reopen Model to retry."
            ))
        } else {
            editor
                .picker
                .model_problem(&project, &model)
                .map(str::to_owned)
        };
        if let Some(status) = status {
            model_field = model_field.child(
                muted(status.clone())
                    .id("task-model-status")
                    .role(Role::Status)
                    .aria_label(status),
            );
        }
        (project_field, model_field)
    }
}
