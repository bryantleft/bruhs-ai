use super::WorkflowExecutor;
use bruhs_models::{MeetingNote, MeetingNoteDraft};
use leptos::prelude::*;

/// App-owned so navigation does not drop a draft or the completion callback.
#[derive(Clone, Copy)]
pub struct NoteEditor {
    pub draft: RwSignal<Option<MeetingNoteDraft>>,
    pub saving: RwSignal<bool>,
    pub message: RwSignal<Option<String>>,
    pub failed: RwSignal<bool>,
}

impl NoteEditor {
    pub fn begin_linked(self, id: String) {
        let occupied = self.draft.with_untracked(Option::is_some);
        self.begin(None);
        if !occupied {
            self.draft.update(|draft| {
                if let Some(draft) = draft {
                    draft.recording_id = Some(id);
                }
            });
        }
    }

    pub fn new() -> Self {
        Self {
            draft: RwSignal::new(None),
            saving: RwSignal::new(false),
            message: RwSignal::new(None),
            failed: RwSignal::new(false),
        }
    }

    pub fn begin(self, note: Option<&MeetingNote>) {
        if self.draft.with_untracked(Option::is_some) {
            self.report("Finish or discard the open draft before editing another note.".into());
            return;
        }
        self.message.set(None);
        self.failed.set(false);
        self.draft.set(Some(
            note.map_or_else(|| MeetingNoteDraft::new(None), MeetingNoteDraft::edit),
        ));
    }

    pub fn report(self, message: String) {
        self.failed.set(true);
        self.message.set(Some(message));
    }

    pub fn discard(self) {
        if !self.saving.get_untracked() {
            self.draft.set(None);
            self.message.set(None);
            self.failed.set(false);
        }
    }
}

impl WorkflowExecutor {
    pub fn save_note(self) {
        let editor = self.note_editor;
        if editor.saving.get_untracked() {
            return;
        }
        if !self.ready.get_untracked() {
            editor.report(
                "Wait for local notes to load before saving. Your draft is retained.".into(),
            );
            return;
        }
        let Some(draft) = editor.draft.get_untracked() else {
            return;
        };
        let now = crate::clock::now_ms();
        if let Err(error) = draft.prepare(now) {
            editor.report(error.to_string());
            return;
        }
        editor.saving.set(true);
        editor.failed.set(false);
        editor
            .message
            .set(Some("Saving locally… Keep this tab open.".into()));
        wasm_bindgen_futures::spawn_local(async move {
            match crate::storage::meeting_notes::save(&draft, now).await {
                Ok(note) => {
                    self.meeting_notes.update(|notes| {
                        if let Some(current) =
                            notes.iter_mut().find(|current| current.id == note.id)
                        {
                            *current = note.clone();
                        } else {
                            notes.insert(0, note.clone());
                        }
                    });
                    editor.draft.set(Some(MeetingNoteDraft::edit(&note)));
                    editor.message.set(Some(
                        "Saved locally. You can keep editing or close the editor.".into(),
                    ));
                }
                Err(error) => editor.report(error.to_string()),
            }
            editor.saving.set(false);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_another_note_never_replaces_the_active_draft() {
        let owner = Owner::new();
        owner.with(|| {
            let editor = NoteEditor::new();
            editor.begin(None);
            editor
                .draft
                .update(|draft| draft.as_mut().unwrap().body = "Retain this private draft".into());
            let before = editor.draft.get_untracked();
            editor.begin_linked("recording-2".into());
            assert_eq!(editor.draft.get_untracked(), before);
            assert!(editor.failed.get_untracked());
        });
    }

    #[test]
    fn pending_save_prevents_discard_and_failure_keeps_exact_text() {
        let owner = Owner::new();
        owner.with(|| {
            let editor = NoteEditor::new();
            editor.begin_linked("recording-1".into());
            editor
                .draft
                .update(|draft| draft.as_mut().unwrap().body = "  Exact\n🗒  ".into());
            let before = editor.draft.get_untracked();
            editor.saving.set(true);
            editor.discard();
            assert_eq!(editor.draft.get_untracked(), before);
            editor.report("Storage failed".into());
            assert_eq!(editor.draft.get_untracked(), before);
            editor.saving.set(false);
            editor.discard();
            assert!(editor.draft.get_untracked().is_none());
        });
    }
}
