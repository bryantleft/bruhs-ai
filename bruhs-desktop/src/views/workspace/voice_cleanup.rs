use super::*;
use crate::voice_capture::Cleanup;
use bruhs_models::agent::MODEL_CATALOG;
use bruhs_ui::{input::TextareaState, text_field::TextField};

pub(super) struct Fields {
    cue: TextField,
    instructions: TextField,
}
impl Fields {
    pub(super) fn new(
        prefs: &Cleanup,
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let cue: TextField = cx
            .new(|cx| {
                InputState::new(window, cx)
                    .placeholder("sorry")
                    .default_value(prefs.correction_cue.clone())
            })
            .into();
        let instructions: TextField = cx
            .new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 7)
                    .placeholder("How should the model edit your transcript?")
                    .default_value(prefs.instructions.clone())
            })
            .into();
        for (field, is_cue) in [(&cue, true), (&instructions, false)] {
            let changed = field.clone();
            subscriptions.push(field.subscribe_in(
                window,
                cx,
                move |this, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let mut value = changed.value(cx).to_string();
                    let limit = if is_cue { 50 } else { 4096 };
                    if let Some((end, _)) = value.char_indices().nth(limit) {
                        value.truncate(end);
                        changed.set_value(value.clone(), window, cx);
                    }
                    let target = if is_cue {
                        &mut this.dictation_prefs.cleanup.correction_cue
                    } else {
                        &mut this.dictation_prefs.cleanup.instructions
                    };
                    if *target != value {
                        *target = value;
                        this.stamp_run_prefs_on_threads(cx);
                        this.persist_and_notify(cx);
                    }
                },
            ));
        }
        Self { cue, instructions }
    }
    pub(super) fn reset(&self, window: &mut Window, cx: &mut App) {
        self.cue
            .set_value(Cleanup::default().correction_cue, window, cx);
        self.instructions.set_value("", window, cx);
    }
}
impl Workspace {
    pub(super) fn cleanup_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let prefs = &self.dictation_prefs.cleanup;
        let mut rows = vec![self.settings_row(
            "voice-row-clean-transcripts",
            "Clean up transcripts",
            "Send the transcript and nearby draft text to the cleanup model before inserting it.",
            settings_ui::switch(
                "voice-cleanup-enabled",
                prefs.enabled,
                false,
                "Clean up transcripts",
                cx,
                |this, _, cx| {
                    this.dictation_prefs.cleanup.enabled = !this.dictation_prefs.cleanup.enabled;
                    this.stamp_run_prefs_on_threads(cx);
                    this.persist_and_notify(cx);
                },
            ),
        )];
        if !prefs.enabled {
            return rows;
        }
        rows.push(self.settings_row(
            "voice-row-cleanup-style",
            "Instructions",
            "Choose how the model edits punctuation, lists and spoken corrections.",
            self.choice_control(
                "settings-voice-cleanup-style",
                "Cleanup instructions",
                if prefs.custom { "custom" } else { "default" },
                &[("default", "Default"), ("custom", "Custom")],
                cx,
            ),
        ));
        if prefs.custom {
            rows.push(
                self.settings_row(
                    "voice-row-cleanup-prompt",
                    "Custom instructions",
                    "Describe the edits to apply to dictated text.",
                    settings_ui::control_column(
                        self.voice_cleanup_fields
                            .instructions
                            .element("Custom cleanup instructions", false)
                            .into_any_element(),
                    ),
                ),
            );
        }
        rows.push(self.settings_row(
            "voice-row-cleanup-model",
            "Cleanup model",
            "Use the text generation model for this project or choose a separate model.",
            self.text_model_control(
                "settings-voice-cleanup-model",
                "Cleanup model",
                prefs.model.as_deref().filter(|model|!self.generator_unavailable(model)).unwrap_or("inherit"),
                &[("inherit","Use text generation model")],
                cx,
            ),
        ));
        rows.push(self.settings_row("voice-row-correction-cue","Correction cue","Say this word before correcting something you just dictated. Leave it blank to disable the cue.",
            settings_ui::control_column(self.voice_cleanup_fields.cue.element("Correction cue",false).into_any_element())));
        rows
    }
    pub(super) fn select_cleanup(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match id {
            "settings-voice-cleanup-style" => {
                self.dictation_prefs.cleanup.custom = value == "custom"
            }
            "settings-voice-cleanup-model" => {
                self.dictation_prefs.cleanup.model = if value == "inherit" {
                    None
                } else if self.generator_model(value) || MODEL_CATALOG
                    .iter()
                    .any(|m| !m.provider.is_external() && m.id == value)
                {
                    Some(value.to_owned())
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
        true
    }
}
