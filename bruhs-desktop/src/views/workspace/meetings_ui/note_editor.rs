use super::*;
use crate::views::workspace::{form_textarea, InputEvent, InputState};
use bruhs_models::EpochMs;
use bruhs_models::{MeetingNote, MeetingNoteDraft};

pub(in crate::views::workspace) struct NoteEditor {
    draft: MeetingNoteDraft,
    pub(in crate::views::workspace) title: Entity<InputState>,
    pub(in crate::views::workspace) body: Entity<InputState>,
    message: Option<String>,
    failed: bool,
    _subscriptions: [Subscription; 2],
}

#[derive(Clone, Copy)]
enum EditorAction {
    New,
    Save,
    Close,
    Unlink,
}

impl Workspace {
    pub(in crate::views::workspace) fn begin_linked_note(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .meetings_hub
            .read(cx)
            .meetings
            .iter()
            .filter(|meeting| meeting.id == id)
            .count()
            != 1
        {
            return;
        }
        let occupied = self.note_editor.is_some();
        self.begin_note_edit(None, window, cx);
        if !occupied {
            if let Some(editor) = &mut self.note_editor {
                editor.draft.recording_id = Some(id.to_string());
            }
        }
        self.active_view = AppView::Meetings;
        cx.notify();
    }

    pub(super) fn linked_note_button(
        &self,
        meeting_id: &str,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let id = meeting_id.to_string();
        let key_id = id.clone();
        web_button(
            "desktop-new-linked-note",
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::Enabled,
        )
        .debug_selector(|| "desktop-new-linked-note".into())
        .aria_label("New linked note")
        .min_h(px(44.0))
        .on_click(cx.listener(move |this, _, window, cx| this.begin_linked_note(&id, window, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                window.prevent_default();
                cx.stop_propagation();
                this.begin_linked_note(&key_id, window, cx);
            }
        }))
        .child("New linked note")
    }
    pub(in crate::views::workspace) fn begin_note_edit(
        &mut self,
        note: Option<MeetingNote>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(editor) = &mut self.note_editor {
            editor.message =
                Some("Finish or discard the open draft before editing another note.".into());
            editor.failed = true;
            cx.defer_in(window, |this, window, cx| {
                if let Some(editor) = &this.note_editor {
                    editor.title.update(cx, |input, cx| input.focus(window, cx));
                }
            });
            cx.notify();
            return;
        }
        let draft = note
            .as_ref()
            .map_or_else(|| MeetingNoteDraft::new(None), MeetingNoteDraft::edit);
        let title = cx.new(|cx| InputState::new(window, cx).default_value(draft.title.clone()));
        let content = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(10)
                .default_value(draft.body.clone())
        });
        let subscriptions = [&title, &content].map(|input| {
            cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        if let Some(editor) = &mut this.note_editor {
                            editor.message = None;
                            editor.failed = false;
                        }
                        cx.notify();
                    }
                },
            )
        });
        self.note_editor = Some(NoteEditor {
            draft,
            title,
            body: content,
            message: None,
            failed: false,
            _subscriptions: subscriptions,
        });
        // Finish the palette's Enter dispatch before focusing the new input;
        // otherwise that same key can insert a newline into the title.
        cx.defer_in(window, |this, window, cx| {
            if let Some(editor) = &this.note_editor {
                editor.title.update(cx, |input, cx| input.focus(window, cx));
            }
        });
        cx.notify();
    }

    pub(in crate::views::workspace) fn edit_note_by_id(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Ok(note) = MeetingNote::find_unique(&self.meeting_notes, id) {
            self.begin_note_edit(Some(note.clone()), window, cx);
        }
    }

    pub(in crate::views::workspace) fn save_note_edit(&mut self, cx: &mut Context<'_, Self>) {
        let Some(editor) = &mut self.note_editor else {
            return;
        };
        editor.draft.title = editor.title.read(cx).value().to_string();
        editor.draft.body = editor.body.read(cx).value().to_string();
        match editor
            .draft
            .apply(&mut self.meeting_notes, EpochMs::now().as_millis())
        {
            Ok(note) => {
                editor.draft = MeetingNoteDraft::edit(&note);
                editor.failed = false;
                editor.message = Some(
                    "Changes applied locally. Check workspace save status before closing the app."
                        .into(),
                );
                self.persist(cx);
            }
            Err(error) => {
                editor.failed = true;
                editor.message = Some(error.to_string());
            }
        }
        cx.notify();
    }

    fn note_editor_action(
        &mut self,
        action: EditorAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match action {
            EditorAction::Unlink => {
                if let Some(editor) = &mut self.note_editor {
                    editor.draft.recording_id = None;
                    editor.message =
                        Some("Recording link removed from the draft. Save to apply.".into());
                }
                cx.notify();
            }
            EditorAction::New => self.begin_note_edit(None, window, cx),
            EditorAction::Save => self.save_note_edit(cx),
            EditorAction::Close => {
                self.note_editor = None;
                self.search_record_focus.focus(window, cx);
                cx.notify();
            }
        }
    }

    fn note_editor_button(
        &self,
        id: &'static str,
        title: &'static str,
        action: EditorAction,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::Enabled,
        )
        .debug_selector(move || id.into())
        .aria_label(title)
        .min_h(px(44.0))
        .on_click(
            cx.listener(move |this, _, window, cx| this.note_editor_action(action, window, cx)),
        )
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                window.prevent_default();
                cx.stop_propagation();
                this.note_editor_action(action, window, cx);
            }
        }))
        .child(title)
    }

    pub(in crate::views::workspace) fn new_note_button(
        &self,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        self.note_editor_button(
            "desktop-new-meeting-note",
            "New meeting note",
            EditorAction::New,
            cx,
        )
    }

    pub(in crate::views::workspace) fn render_note_editor(
        &self,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(editor) = &self.note_editor else {
            return div().into_any_element();
        };
        div()
            .id("desktop-meeting-note-editor")
            .debug_selector(|| "desktop-meeting-note-editor".into())
            .aria_label("Edit local meeting note")
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .py_4()
            .child(body(
                "Edit local meeting note",
                BODY_PX,
                theme::rambutan_100(),
            ))
            .child(body(
                "Draft stays available during navigation. Save before closing the app.",
                BODY_SM_PX,
                theme::rambutan_400(),
            ))
            .child(label("Title", LABEL_SM_PX, theme::rambutan_300()))
            .child(
                Input::new(&editor.title)
                    .w_full()
                    .aria_label("Meeting note title"),
            )
            .child(label("Notes", LABEL_SM_PX, theme::rambutan_300()))
            .child(form_textarea(Input::new(&editor.body), 220.0).aria_label("Meeting note text"))
            .when_some(editor.draft.recording_id.as_ref(), |el, id| {
                el.child(body(
                    format!("Linked recording: {id}"),
                    BODY_SM_PX,
                    theme::rambutan_300(),
                ))
                .child(self.note_editor_button(
                    "desktop-unlink-note",
                    "Remove recording link",
                    EditorAction::Unlink,
                    cx,
                ))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.note_editor_button(
                        "desktop-save-meeting-note",
                        "Save note",
                        EditorAction::Save,
                        cx,
                    ))
                    .child(self.note_editor_button(
                        "desktop-close-note-editor",
                        "Close editor / discard unsaved changes",
                        EditorAction::Close,
                        cx,
                    )),
            )
            .when_some(editor.message.clone(), |el, message| {
                el.child(
                    body(message, BODY_SM_PX, theme::rambutan_200())
                        .id("desktop-note-editor-message")
                        .role(if editor.failed {
                            Role::Alert
                        } else {
                            Role::Status
                        }),
                )
            })
            .into_any_element()
    }
}
