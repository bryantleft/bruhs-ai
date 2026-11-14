use super::*;
use std::sync::mpsc;

/// Product bound matches T3's documented five-minute composer dictation limit.
/// See <https://github.com/pingdotgg/t3code/blob/main/docs/user/voice-input.md>.
const MAX_RECORDING: std::time::Duration = std::time::Duration::from_secs(5 * 60);

#[derive(Default)]
pub(super) enum DictationPhase {
    #[default]
    Idle,
    Recording(mpsc::Sender<bool>),
    Transcribing,
}

#[derive(Default)]
pub(super) struct ComposerDictation {
    pub phase: DictationPhase,
    pub error: Option<String>,
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
        self.task = None;
    }
}

impl ChatView {
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
        let original = input.value().to_string();
        let selection = input.selected_range();
        let thread = self.thread_id.clone();
        self.dictation.error = None;
        self.dictation.generation = self.dictation.generation.wrapping_add(1);
        let generation = self.dictation.generation;
        let (stop, receiver) = mpsc::channel();
        self.dictation.phase = DictationPhase::Recording(stop);
        let capture = crate::app_state::spawn_tokio(cx, async move {
            tokio::task::spawn_blocking(move || {
                crate::voice_capture::capture_voice_until_stop(receiver, MAX_RECORDING)
            })
            .await
            .map_err(|error| format!("Microphone worker stopped: {error}"))?
        });
        self.dictation.task=Some(cx.spawn(async move |this,cx| {
            let audio=match capture.await {
                Ok(audio)=>audio,
                Err(error)=>{let _=this.update(cx,|this,cx|{if this.dictation.generation==generation {this.dictation.phase=DictationPhase::Idle;this.dictation.error=Some(error);cx.notify();}});return;},
            };
            let request=this.update(cx,|this,cx| {
                if this.dictation.generation!=generation || this.thread_id!=thread {return None;}
                this.dictation.phase=DictationPhase::Transcribing;
                cx.notify();
                Some(crate::app_state::spawn_tokio(cx,async move {
                    bruhs_core::capture_client::transcribe_dictation(
                        bruhs_core::capture_client::endpoint(),
                        &bruhs_models::local_tools::DictationOptions{model:bruhs_models::WhisperModel::BaseEn,language:Some("en".into())},audio,
                    ).await.map_err(|error|format!("Local dictation failed: {error}. Start the capture daemon and check the local Whisper model."))
                }))
            });
            let Ok(Some(request))=request else{return;};
            let result=request.await;
            let _=this.update(cx,|this,cx| {
                if this.dictation.generation!=generation || this.thread_id!=thread {return;}
                this.dictation.phase=DictationPhase::Idle;
                match result {
                    Ok(reply)=> {
                        let current=this.pending_composer.clone().unwrap_or_else(||this.input_state.read(cx).value().to_string());
                        match bruhs_models::local_tools::insert_dictation(&current,&original,selection,&reply.text) {
                            Ok(draft)=>{this.pending_composer=Some(draft);this.focus_pending_composer=true;},
                            Err(error)=>this.dictation.error=Some(format!("{error} Transcript: {}",reply.text)),
                        }
                    },
                    Err(error)=>this.dictation.error=Some(error),
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
