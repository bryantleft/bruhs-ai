use super::*;
use crate::voice_capture::dictionary::VoiceDictionary;
use bruhs_models::voice_dictionary::{
    DictionaryEdit, DictionaryOperation, DictionaryRequest, HostDictionaries,
};
use bruhs_ui::press::on_press_view;
#[derive(Clone, PartialEq)]
struct Scope {
    project: Option<String>,
    shared: bool,
}

pub(super) struct DictionaryEditor {
    terms: Entity<InputState>,
    alias: Entity<InputState>,
    scope: Option<(Scope, Option<String>)>,
    shared: HostDictionaries,
    shared_project: Option<String>,
    saving: bool,
    revision: u64,
    host_error: Option<String>,
    error: Option<String>,
}

impl DictionaryEditor {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            terms: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Add words, separated by commas")
            }),
            alias: cx.new(|cx| InputState::new(window, cx).placeholder("Misspelling")),
            scope: None,
            shared: Default::default(),
            shared_project: None,
            saving: false,
            revision: 0,
            host_error: None,
            error: None,
        }
    }
}

impl Workspace {
    fn dictionary_mut(&mut self, project: Option<&str>) -> &mut VoiceDictionary {
        match project {
            Some(path) => self
                .dictation_prefs
                .project_dictionaries
                .entry(path.to_owned())
                .or_default(),
            None => &mut self.dictation_prefs.dictionary,
        }
    }

    fn edit_dictionary(
        &mut self,
        project: Scope,
        term: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let field = if term.is_some() {
            &self.voice_dictionary_editor.alias
        } else {
            &self.voice_dictionary_editor.terms
        };
        field.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.voice_dictionary_editor.scope = Some((project, term));
        self.voice_dictionary_editor.error = None;
        cx.notify();
    }

    fn save_dictionary_entry(&mut self, cx: &mut Context<'_, Self>) {
        let Some((project, term)) = self.voice_dictionary_editor.scope.clone() else {
            return;
        };
        let field = if term.is_some() {
            &self.voice_dictionary_editor.alias
        } else {
            &self.voice_dictionary_editor.terms
        };
        let value = field.read(cx).value().to_string();
        if project.shared {
            let dictionaries = &self.voice_dictionary_editor.shared;
            let mut current = if project.project.is_some() {
                dictionaries.project.clone()
            } else {
                dictionaries.environment.clone()
            };
            if let Err(message) = current.add(&value, term.as_deref()) {
                self.voice_dictionary_editor.error = Some(message);
                cx.notify();
                return;
            }
            let active = self.active_settings_project(cx);
            self.shared_voice_request(
                project.project.clone().or(active),
                Some(DictionaryEdit {
                    for_project: project.project.is_some(),
                    operation: DictionaryOperation::Add {
                        words: value,
                        alias_for: term,
                    },
                }),
                cx,
            );
            return;
        }
        match self
            .dictionary_mut(project.project.as_deref())
            .add(&value, term.as_deref())
        {
            Ok(()) => {
                self.voice_dictionary_editor.scope = None;
                self.voice_dictionary_editor.error = None;
                self.stamp_run_prefs_on_threads(cx);
                self.persist_and_notify(cx);
            }
            Err(message) => {
                self.voice_dictionary_editor.error = Some(message);
                cx.notify();
            }
        }
    }

    pub(super) fn reload_voice_dictionary(&mut self, cx: &mut Context<'_, Self>) {
        let project = self.active_settings_project(cx);
        self.shared_voice_request(project, None, cx);
    }
    fn shared_voice_request(
        &mut self,
        project: Option<String>,
        edit: Option<DictionaryEdit>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.voice_dictionary_editor.saving {
            return;
        }
        let mutation = edit.is_some();
        let editor = &mut self.voice_dictionary_editor;
        editor.saving = mutation;
        editor.revision = editor.revision.wrapping_add(1);
        let revision = editor.revision;
        let edited_entry = editor.scope.clone();
        if editor.shared_project != project {
            editor.shared.project = Default::default();
        }
        editor.shared_project = project.clone();
        let request = crate::app_state::spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .voice_dictionaries(&DictionaryRequest { project, edit })
                .await
        });
        cx.spawn(async move |this, cx| {
            let reply = request.await;
            let _ = this.update(cx, |this, cx| {
                let active_project = this.active_settings_project(cx);
                let editor = &mut this.voice_dictionary_editor;
                if editor.revision != revision {
                    return;
                }
                editor.saving = false;
                match reply {
                    Ok(data) => {
                        editor.shared = data;
                        editor.host_error = None;
                        if mutation && editor.scope == edited_entry {
                            editor.scope = None;
                            editor.error = None;
                        }
                    }
                    Err(e) => {
                        editor.host_error = Some(format!("Host dictionary unavailable. {e}"));
                    }
                }
                let changed_scope = editor.shared_project != active_project;
                if changed_scope
                    && this.settings_visible()
                    && this.settings_section == SettingsSection::Voice
                {
                    this.reload_voice_dictionary(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn voice_dictionary_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let project = self.active_settings_project(cx);
        let mut rows = Vec::new();
        let mut scopes = vec![
            Scope {
                project: None,
                shared: false,
            },
            Scope {
                project: None,
                shared: true,
            },
        ];
        if let Some(path) = project {
            if self
                .dictation_prefs
                .project_dictionaries
                .contains_key(&path)
            {
                scopes.push(Scope {
                    project: Some(path.clone()),
                    shared: false,
                });
            }
            scopes.push(Scope {
                project: Some(path),
                shared: true,
            });
        }
        for scope in scopes {
            let (id, title, description, entries) = match (scope.shared, scope.project.as_deref()) {
                (true, None) => (
                    "settings-voice-shared-dictionary",
                    "Shared dictionary",
                    "Preferred spellings for everyone who dictates on this host.".into(),
                    Some(&self.voice_dictionary_editor.shared.environment),
                ),
                (true, Some(path)) => (
                    "settings-voice-project-dictionary",
                    "Project dictionary",
                    format!("Preferred spellings shared by people dictating in {path}."),
                    self.voice_dictionary_editor
                        .shared_project
                        .as_deref()
                        .filter(|p| *p == path)
                        .map(|_| &self.voice_dictionary_editor.shared.project),
                ),
                (false, Some(path)) => (
                    "settings-voice-project-device",
                    "Project dictionary on this device",
                    format!("Previously saved spellings for {path}."),
                    self.dictation_prefs.project_dictionaries.get(path),
                ),
                (false, None) => (
                    "settings-voice-dictionary",
                    "Your dictionary",
                    "Names and terms to spell your way on this device.".into(),
                    Some(&self.dictation_prefs.dictionary),
                ),
            };
            let busy = scope.shared && self.voice_dictionary_editor.saving;
            let mut editor = div().w_full().flex().flex_col().gap_2();
            for (index, (term, aliases)) in entries
                .into_iter()
                .flat_map(|dictionary| &dictionary.0)
                .enumerate()
            {
                let mut row = div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(div().flex_1().child(term.clone()));
                if !aliases.is_empty() {
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(theme::rambutan_400())
                            .child(aliases.join(", ")),
                    );
                }
                let alias_term = term.clone();
                let alias_scope = scope.clone();
                let add = web_button(
                    SharedString::from(format!("{id}-alias-{index}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(busy || aliases.len() >= 8),
                )
                .aria_label(format!("Add an alias for {term}"))
                .child("Add alias");
                row = row.child(on_press_view(add, cx, move |this, window, cx| {
                    this.edit_dictionary(alias_scope.clone(), Some(alias_term.clone()), window, cx)
                }));
                let remove_term = term.clone();
                let remove_scope = scope.clone();
                let remove = web_button(
                    SharedString::from(format!("{id}-remove-{index}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(busy),
                )
                .aria_label(format!("Remove {term}"))
                .child("Remove");
                editor = editor.child(row.child(on_press_view(remove, cx, move |this, _, cx| {
                    if remove_scope.shared {
                        let project = remove_scope
                            .project
                            .clone()
                            .or_else(|| this.active_settings_project(cx));
                        this.shared_voice_request(
                            project,
                            Some(DictionaryEdit {
                                for_project: remove_scope.project.is_some(),
                                operation: DictionaryOperation::Remove {
                                    term: remove_term.clone(),
                                },
                            }),
                            cx,
                        );
                        return;
                    }
                    this.dictionary_mut(remove_scope.project.as_deref())
                        .0
                        .remove(&remove_term);
                    this.voice_dictionary_editor.scope = None;
                    this.stamp_run_prefs_on_threads(cx);
                    this.persist_and_notify(cx);
                })));
            }
            if let Some((_, term)) = self
                .voice_dictionary_editor
                .scope
                .as_ref()
                .filter(|(project, _)| project == &scope)
            {
                let (field, label) = match term {
                    Some(_) => (&self.voice_dictionary_editor.alias, "Alias"),
                    None => (&self.voice_dictionary_editor.terms, "Dictionary words"),
                };
                let save = web_button(
                    SharedString::from(format!("{id}-save")),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(busy),
                )
                .aria_label(format!("Add {title} entry"))
                .child("Add");
                let cancel = web_button(
                    SharedString::from(format!("{id}-cancel")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(busy),
                )
                .child("Cancel");
                editor = editor
                    .child(
                        div()
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if event.keystroke.key == "enter" {
                                    this.save_dictionary_entry(cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .child(Input::new(field).aria_label(label)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(on_press_view(save, cx, |this, _, cx| {
                                this.save_dictionary_entry(cx)
                            }))
                            .child(on_press_view(cancel, cx, |this, _, cx| {
                                this.voice_dictionary_editor.scope = None;
                                cx.notify();
                            })),
                    );
                if let Some(error) = &self.voice_dictionary_editor.error {
                    editor = editor.child(
                        div()
                            .id("dictionary-error")
                            .role(Role::Alert)
                            .child(error.clone()),
                    );
                }
            } else {
                let add = web_button(
                    SharedString::from(format!("{id}-add")),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(busy),
                )
                .aria_label(format!("Add words to {title}"))
                .child("Add words");
                let add_scope = scope.clone();
                editor = editor.child(on_press_view(add, cx, move |this, window, cx| {
                    this.edit_dictionary(add_scope.clone(), None, window, cx)
                }));
            }
            if let Some(message) = self
                .voice_dictionary_editor
                .host_error
                .clone()
                .filter(|_| scope.shared)
            {
                editor = editor.child(div().text_xs().child(message));
            }
            rows.push(self.settings_row(
                id,
                title,
                description,
                settings_ui::control_column(editor.into_any_element()),
            ));
        }
        rows
    }
}
