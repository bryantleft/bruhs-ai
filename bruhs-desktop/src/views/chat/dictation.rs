use super::*;
use std::sync::mpsc;

/// Maximum recording time for one composer dictation.
const MAX_RECORDING: std::time::Duration = std::time::Duration::from_secs(5 * 60);

#[derive(Default)]
pub(super) enum DictationPhase {
    #[default]
    Idle,
    Recording(mpsc::Sender<bool>),
    Transcribing,
    Cleaning(crate::voice_capture::SkipCleanup),
}

#[derive(Default)]
pub(super) struct ComposerDictation {
    pub phase: DictationPhase,
    pub error: Option<String>,
    pub preview: String,
    generation: u64,
    task: Option<Task<()>>,
}

impl ComposerDictation {
    pub fn active(&self) -> bool {
        !matches!(self.phase, DictationPhase::Idle)
    }
    pub fn recording(&self) -> bool {
        matches!(self.phase, DictationPhase::Recording(_))
    }
    pub fn cancel(&mut self) {
        if let DictationPhase::Recording(stop) = std::mem::take(&mut self.phase) {
            let _ = stop.send(false);
        }
        self.generation = self.generation.wrapping_add(1);
        self.preview.clear();
        self.task = None;
    }
}

impl ChatView {
    pub(crate) fn begin_dictation_shortcut(&mut self, cx: &mut Context<'_, Self>) -> Option<u64> {
        if self.dictation.recording() {
            self.stop_dictation(cx);
            return None;
        }
        if self.dictation.active() {
            return None;
        }
        self.start_dictation(cx);
        self.dictation
            .recording()
            .then_some(self.dictation.generation)
    }

    pub(crate) fn finish_dictation_shortcut(
        &mut self,
        generation: u64,
        cx: &mut Context<'_, Self>,
    ) {
        if self.dictation.generation == generation {
            self.stop_dictation(cx);
        }
    }

    pub(crate) fn cancel_dictation(&mut self, cx: &mut Context<'_, Self>) {
        let active = self.dictation.active();
        if !active {
            return;
        }
        self.dictation.cancel();
        if active {
            cx.notify();
        }
    }

    pub(super) fn skip_dictation_cleanup(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.dictation.phase, DictationPhase::Cleaning(_)) {
            if let DictationPhase::Cleaning(skip) =
                std::mem::replace(&mut self.dictation.phase, DictationPhase::Transcribing)
            {
                let _ = skip.send(());
                cx.notify();
            }
        }
    }
    pub(super) fn stop_dictation(&mut self, cx: &mut Context<'_, Self>) {
        if !self.dictation.recording() {
            return;
        }
        if let DictationPhase::Recording(stop) =
            std::mem::replace(&mut self.dictation.phase, DictationPhase::Transcribing)
        {
            let _ = stop.send(true);
            cx.notify();
        }
    }

    pub(super) fn start_dictation(&mut self, cx: &mut Context<'_, Self>) {
        if self.dictation.active() {
            return;
        }
        self.dismiss_prompt_suggestion(cx);
        let input = self.input_state.read(cx);
        if let Err(message) = self.dictation_prefs.selected_service() {
            self.dictation.error = Some(message.into());
            cx.notify();
            return;
        }
        let original = input.value().to_string();
        let selection = input.selected_range();
        let cleanup_model = self.text_generation_model.clone();
        let nearby = if self.dictation_prefs.cleanup.enabled {
            bruhs_core::dictation_cleanup::DraftContext::around(&original, selection.clone())
        } else {
            Default::default()
        };
        let thread = self.thread_id.clone();
        let microphone = self.dictation_prefs.ordered_microphones();
        let preferences = self.dictation_prefs.clone();
        let project_name = self.project_scoped.then(|| self.project_name.clone());
        let project = self
            .project_scoped
            .then(|| self.settings_project_path().to_owned());
        self.dictation.error = None;
        self.dictation.preview.clear();
        self.dictation.generation = self.dictation.generation.wrapping_add(1);
        let generation = self.dictation.generation;
        let (stop, receiver) = mpsc::channel();
        self.dictation.phase = DictationPhase::Recording(stop);
        let (chunks, preview_worker) = if preferences.streaming
            && preferences.destination == crate::voice_capture::Destination::Local
        {
            let (chunks, audio) = tokio::sync::mpsc::channel(1);
            let (updates, mut previews) = tokio::sync::watch::channel(String::new());
            let preview_worker = crate::app_state::spawn_tokio(
                cx,
                crate::voice_capture::preview::transcribe(audio, preferences.options(), updates),
            );
            cx.spawn(async move |this, cx| {
                while previews.changed().await.is_ok() {
                    let preview = previews.borrow_and_update().clone();
                    if !this
                        .update(cx, |this, cx| {
                            if this.dictation.generation != generation
                                || !this.dictation.recording()
                            {
                                return false;
                            }
                            this.dictation.preview = preview;
                            cx.notify();
                            true
                        })
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
            })
            .detach();
            (Some(chunks), Some(preview_worker))
        } else {
            (None, None)
        };
        let capture = crate::app_state::spawn_tokio(cx, async move {
            tokio::task::spawn_blocking(move || {
                crate::voice_capture::capture_voice_until_stop(
                    receiver,
                    MAX_RECORDING,
                    &microphone,
                    chunks,
                )
            })
            .await
            .map_err(|error| format!("Microphone worker stopped: {error}"))?
        });
        self.dictation.task = Some(cx.spawn(async move |this, cx| {
            let audio = match capture.await {
                Ok(audio) => audio,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.dictation.generation == generation {
                            this.dictation.phase = DictationPhase::Idle;
                            this.dictation.error = Some(error);
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            // The capture host serializes Whisper inference. Let an already-running
            // preview finish before submitting the full audio for the final pass.
            if let Some(worker) = preview_worker {
                worker.await;
            }
            let request = this.update(cx, |this, cx| {
                if this.dictation.generation != generation || this.thread_id != thread {
                    return None;
                }
                this.dictation.phase = DictationPhase::Transcribing;
                cx.notify();
                let control = if preferences.cleanup.enabled {
                    let (ready, signal) = tokio::sync::oneshot::channel();
                    cx.spawn(async move |this, cx| {
                        if let Ok(skip) = signal.await {
                            let _ = this.update(cx, |this, cx| {
                                if this.dictation.generation == generation
                                    && matches!(this.dictation.phase, DictationPhase::Transcribing)
                                {
                                    this.dictation.phase = DictationPhase::Cleaning(skip);
                                    cx.notify();
                                }
                            });
                        }
                    })
                    .detach();
                    Some(ready)
                } else {
                    None
                };
                Some(crate::app_state::spawn_tokio(cx, async move {
                    preferences
                        .transcribe(
                            audio,
                            project.as_deref(),
                            project_name.as_deref(),
                            &cleanup_model,
                            nearby,
                            control,
                        )
                        .await
                }))
            });
            let Ok(Some(request)) = request else {
                return;
            };
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if this.dictation.generation != generation || this.thread_id != thread {
                    return;
                }
                this.dictation.phase = DictationPhase::Idle;
                this.dictation.preview.clear();
                match result {
                    Ok(result) => {
                        let text = result.text;
                        this.dictation.error = result.warning.map(String::from);
                        let current = this
                            .pending_composer
                            .clone()
                            .unwrap_or_else(|| this.input_state.read(cx).value().to_string());
                        match bruhs_models::local_tools::insert_dictation(
                            &current, &original, selection, &text,
                        ) {
                            Ok(draft) => {
                                this.pending_composer = Some(draft);
                                this.focus_pending_composer = true;
                            }
                            Err(error) => {
                                this.dictation.error = Some(format!("{error} Transcript: {text}"))
                            }
                        }
                    }
                    Err(error) => this.dictation.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposerDictation, DictationPhase};
    use std::sync::mpsc;

    #[test]
    fn cancellation_stops_capture_and_invalidates_pending_results() {
        let (stop, receiver) = mpsc::channel();
        let mut dictation = ComposerDictation {
            phase: DictationPhase::Recording(stop),
            generation: 3,
            ..Default::default()
        };
        dictation.cancel();
        assert!(!dictation.active());
        assert!(!receiver.recv().unwrap());
        assert_ne!(dictation.generation, 3);
    }
}
