//! Settings only holds credential metadata and disposable replacement input.
use super::*;
use crate::app_state::spawn_tokio;
use bruhs_models::secrets::{review_text, SavedCredential, SavedCredentialState};
use gpui_component::input::{Input, InputContentType};

enum Load {
    Idle,
    Loading,
    Ready,
    Failed(String),
}
enum Editor {
    Replace {
        id: String,
        input: Entity<InputState>,
    },
    Delete(String),
}

pub(super) struct CredentialSettings {
    rows: Vec<SavedCredential>,
    load: Load,
    search: Entity<InputState>,
    _search_subscription: Subscription,
    editor: Option<Editor>,
    edit_subscription: Option<Subscription>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    task: Option<Task<()>>,
}

impl CredentialSettings {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search by name or project"));
        let subscription = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.clear_editor();
            }
            cx.notify();
        });
        Self {
            rows: Vec::new(),
            load: Load::Idle,
            search,
            _search_subscription: subscription,
            editor: None,
            edit_subscription: None,
            busy: false,
            error: None,
            notice: None,
            task: None,
        }
    }

    pub(super) fn has_editor(&self) -> bool {
        self.editor.is_some()
    }
    pub(super) fn clear_editor(&mut self) {
        self.editor = None;
        self.edit_subscription = None;
        self.error = None;
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.clear_editor();
        self.notice = None;
        self.load = Load::Loading;
        self.busy = true;
        let manager = cx.global::<DesktopState>().app.secrets().clone();
        let operation = spawn_tokio(cx, async move { manager.list_credentials().await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(rows) => {
                        this.rows = rows;
                        this.load = Load::Ready;
                    }
                    Err(error) => this.load = Load::Failed(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn begin_edit(
        &mut self,
        id: String,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        self.clear_editor();
        self.notice = None;
        if delete {
            self.editor = Some(Editor::Delete(id));
        } else {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Enter replacement")
            });
            self.edit_subscription =
                Some(
                    cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                        if matches!(event, InputEvent::PressEnter { .. }) {
                            this.mutate(window, cx);
                        }
                    }),
                );
            self.editor = Some(Editor::Replace { id, input });
        }
        cx.notify();
    }

    fn mutate(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let (id, replacement) = match &self.editor {
            Some(Editor::Replace { id, input }) => {
                let value = input.read(cx).value().to_string();
                if value.is_empty() {
                    self.error = Some("Enter a replacement credential.".into());
                    cx.notify();
                    return;
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
                (id.clone(), Some(value))
            }
            Some(Editor::Delete(id)) => (id.clone(), None),
            None => return,
        };
        self.busy = true;
        self.error = None;
        self.notice = None;
        let delete = replacement.is_none();
        let manager = cx.global::<DesktopState>().app.secrets().clone();
        let update_id = id.clone();
        let operation = spawn_tokio(cx, async move {
            match replacement {
                Some(value) => manager.replace_credential(&id, value).await,
                None => manager.delete_credential(&id).await,
            }
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        if delete {
                            this.rows.retain(|row| row.id != update_id);
                        } else {
                            if let Some(row) = this.rows.iter_mut().find(|row| row.id == update_id)
                            {
                                row.state = SavedCredentialState::Saved;
                            }
                            this.notice = Some("Credential updated.".into());
                        }
                        this.clear_editor();
                    }
                    Err(bruhs_core::secrets::SecretError::NotSaved) => {
                        this.rows.retain(|row| row.id != update_id);
                        this.clear_editor();
                    }
                    Err(error) => {
                        this.error = Some(format!("{error} Re-enter the value or retry deletion."))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn row_button(
        &self,
        entry: &SavedCredential,
        delete: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let id = entry.id.clone();
        let key_id = id.clone();
        web_button(
            SharedString::from(format!(
                "credential-{}-{id}",
                if delete { "delete" } else { "replace" }
            )),
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.busy),
        )
        .min_h(px(40.0))
        .child(if delete { "Delete" } else { "Replace" })
        .on_click(
            cx.listener(move |this, _, window, cx| this.begin_edit(id.clone(), delete, window, cx)),
        )
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                cx.stop_propagation();
                this.begin_edit(key_id.clone(), delete, window, cx);
            }
        }))
    }

    fn row(&self, entry: &SavedCredential, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id(SharedString::from(format!("saved-credential-{}", entry.id)))
            .role(Role::Group)
            .aria_label(format!("Saved credential: {}", review_text(&entry.name)))
            .py_3()
            .min_w_0()
            .border_b_1()
            .border_color(theme::longan_800())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_color(theme::rambutan_100())
                    .child(review_text(&entry.name)),
            )
            .child(
                div()
                    .text_color(theme::rambutan_400())
                    .child(review_text(&entry.project_path)),
            )
            .child(div().text_color(theme::rambutan_400()).child(
                if entry.state == SavedCredentialState::NeedsAttention {
                    "Needs attention · replace or delete to finish a previous change."
                } else {
                    "Stored in OS keychain · value hidden"
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.row_button(entry, false, cx))
                    .child(self.row_button(entry, true, cx)),
            )
            .when(
                self.editor.as_ref().is_some_and(|editor| match editor {
                    Editor::Replace { id, .. } | Editor::Delete(id) => id == &entry.id,
                }),
                |element| {
                    element.child(self.editor_panel(cx)).when_some(
                        self.error.clone(),
                        |element, message| {
                            element.child(
                                div()
                                    .id("credential-edit-error")
                                    .role(Role::Alert)
                                    .text_color(theme::lychee_300())
                                    .child(message),
                            )
                        },
                    )
                },
            )
            .into_any_element()
    }

    fn editor_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        let mut panel = div().flex().flex_col().gap_2();
        panel = match editor {
            Editor::Replace { input, .. } => panel.child("New credential value").child(
                Input::new(input)
                    .h(px(40.0))
                    .aria_label("New credential value")
                    .content_type(InputContentType::Password)
                    .disabled(self.busy),
            ),
            Editor::Delete(_) => panel.child("Delete this saved credential from the OS keychain?"),
        };
        panel
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        web_button(
                            "confirm-credential-change",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(self.busy),
                        )
                        .min_h(px(40.0))
                        .child(if matches!(editor, Editor::Delete(_)) {
                            "Delete credential"
                        } else {
                            "Save replacement"
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.mutate(window, cx)))
                        .on_key_down(cx.listener(
                            |this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    cx.stop_propagation();
                                    this.mutate(window, cx);
                                }
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "cancel-credential-change",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(self.busy),
                        )
                        .min_h(px(40.0))
                        .child("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_editor();
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            |this, event: &KeyDownEvent, _, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    cx.stop_propagation();
                                    this.clear_editor();
                                    cx.notify();
                                }
                            },
                        )),
                    ),
            )
            .into_any_element()
    }
}

impl Render for CredentialSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if matches!(self.load, Load::Idle) {
            self.refresh(cx);
        }
        let query = self.search.read(cx).value().to_lowercase();
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| {
                row.name.to_lowercase().contains(&query)
                    || row.project_path.to_lowercase().contains(&query)
            })
            .map(|row| self.row(row, cx))
            .collect();
        div().id("credential-manager").p_4().min_w_0().font_family(theme::UI_SMALL_FONT_FAMILY).text_size(px(12.0)).text_color(theme::rambutan_200()).flex().flex_col().gap_3()
            .child(Input::new(&self.search).h(px(40.0)).aria_label("Search saved credentials"))
            .child(web_button("refresh-credentials", WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::disabled_if(self.busy)).min_h(px(40.0)).child("Refresh credentials")
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| { if is_web_button_activation_key(event.keystroke.key.as_str()) { cx.stop_propagation(); this.refresh(cx); } })))
            .child("Values are never displayed. Replace a key to rotate it. Deleting removes the saved copy; running commands may finish with the value they already received.")
            .child("Older saved credentials appear here after their next thread request.")
            .when(matches!(self.load, Load::Loading), |element| element.child("Loading saved credentials…"))
            .when(matches!(self.load, Load::Ready) && self.rows.is_empty(), |element| element.child("No saved credentials. Choose Save securely and use in a thread request to add one."))
            .when_some(match &self.load { Load::Failed(error) => Some(error.clone()), _ => None }, |element, message| element.child(div().id("credential-list-error").role(Role::Alert).text_color(theme::lychee_300()).child(format!("{message} Refresh to retry."))))
            .when(rows.is_empty() && !self.rows.is_empty(), |element| element.child("No credentials match your search."))
            .children(rows)
            .when_some(self.notice.clone(), |element, message| element.child(div().id("credential-edit-status").role(Role::Status).child(message)))
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialSettings, Editor, Load, SavedCredential, SavedCredentialState};
    use crate::{app_state::DesktopState, theme};
    use gpui::{AppContext as _, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn replacement_input_starts_blank_is_disposable_and_never_enters_metadata(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let capture = Rc::new(RefCell::new(None));
        let output = capture.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = CredentialSettings::new(window, cx);
                view.load = Load::Ready;
                view.rows = vec![SavedCredential {
                    id: "synthetic-id".into(),
                    name: "Service key".into(),
                    project_path: "/synthetic/project".into(),
                    state: SavedCredentialState::Saved,
                }];
                view
            });
            *output.borrow_mut() = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let view = capture.borrow().clone().unwrap();
        view.update_in(cx, |view, window, cx| {
            view.begin_edit("synthetic-id".into(), false, window, cx);
            let Some(Editor::Replace { input, .. }) = &view.editor else {
                panic!("replacement editor missing");
            };
            assert!(input.read(cx).value().is_empty());
            input.update(cx, |input, cx| {
                input.set_value("private-replacement-canary", window, cx)
            });
            assert!(!serde_json::to_string(&view.rows)
                .unwrap()
                .contains("private-replacement-canary"));
            view.clear_editor();
            assert!(!view.has_editor());
            view.begin_edit("synthetic-id".into(), false, window, cx);
            let Some(Editor::Replace { input, .. }) = &view.editor else {
                panic!("replacement editor missing");
            };
            assert!(input.read(cx).value().is_empty());
            view.begin_edit("synthetic-id".into(), true, window, cx);
            assert!(matches!(view.editor, Some(Editor::Delete(_))));
            assert_eq!(
                view.rows.len(),
                1,
                "delete confirmation does not delete before approval"
            );
            view.clear_editor();
            assert_eq!(view.rows.len(), 1);
        });
    }
}
