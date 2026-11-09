//! Voice enrollment modal.
//!
//! Records a 5-second microphone sample via Web Audio API's AudioContext +
//! ScriptProcessorNode (at 16kHz mono), encodes it as a minimal WAV, and
//! POSTs it to `/api/people/{id}/enroll`. On success the PeopleUpdated SSE
//! event from the server will refresh the People page.
//!
//! Flow: "Record" button → 3-2-1 countdown → 5s recording indicator →
//! uploading → success / error message.

use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::state::{PersonProfile, SetPeopleSignal};

#[derive(Debug, Clone, Copy)]
enum EnrollState {
    Idle,
    Countdown(i32), // seconds left before recording starts
    Recording,
    Uploading,
    Success,
    Error,
}

impl PartialEq for EnrollState {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (EnrollState::Idle, EnrollState::Idle)
                | (EnrollState::Recording, EnrollState::Recording)
                | (EnrollState::Uploading, EnrollState::Uploading)
                | (EnrollState::Success, EnrollState::Success)
                | (EnrollState::Error, EnrollState::Error)
        ) || matches!((self, other), (EnrollState::Countdown(a), EnrollState::Countdown(b)) if a == b)
    }
}

/// Modal for enrolling a person's voice.
/// `show` — reactive bool controlling visibility.
/// `on_close` — called when the modal should close (idle/success/error states).
#[component]
pub fn VoiceEnrollModal(
    person_id: String,
    person_name: Signal<String>,
    #[prop(into)] show: Signal<bool>,
    on_close: impl Fn() + 'static + Clone,
) -> impl IntoView {
    let SetPeopleSignal(set_people) = expect_context::<SetPeopleSignal>();
    let (enroll_state, set_enroll_state) = signal(EnrollState::Idle);
    let (error_msg, set_error_msg) = signal(String::new());

    // Store person_id so closures can clone on every invocation (Fn requirement).
    let pid = StoredValue::new(person_id);

    let start_enrollment = move |_| {
        if enroll_state.get_untracked() != EnrollState::Idle {
            return;
        }
        set_enroll_state.set(EnrollState::Countdown(3));

        let person_id_clone = pid.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            // 3 → 2 → 1 countdown.
            for remaining in (1..=3i32).rev() {
                set_enroll_state.set(EnrollState::Countdown(remaining));
                gloo_timers::future::TimeoutFuture::new(1_000).await;
            }

            set_enroll_state.set(EnrollState::Recording);

            // Capture 5 seconds via AudioContext.
            match capture_voice_sample_wav(5_000).await {
                Ok(wav_bytes) => {
                    set_enroll_state.set(EnrollState::Uploading);
                    match api::enroll_voice(&person_id_clone, &wav_bytes).await {
                        Ok(updated_profile) => {
                            set_enroll_state.set(EnrollState::Success);
                            set_people.update(|list: &mut Vec<PersonProfile>| {
                                if let Some(slot) =
                                    list.iter_mut().find(|p| p.id == updated_profile.id)
                                {
                                    *slot = updated_profile;
                                }
                            });
                            // Auto-close after 2s on success.
                            gloo_timers::future::TimeoutFuture::new(2_000).await;
                            set_enroll_state.set(EnrollState::Idle);
                        }
                        Err(e) => {
                            set_error_msg.set(format!("Upload failed: {e}"));
                            set_enroll_state.set(EnrollState::Error);
                        }
                    }
                }
                Err(e) => {
                    set_error_msg.set(format!("Microphone error: {e}"));
                    set_enroll_state.set(EnrollState::Error);
                }
            }
        });
    };

    // Use a signal as a "close requested" flag.  The click handlers only flip
    // signals (which are Copy + Send) and an Effect fires `on_close` — keeping
    // the non-Send `on_close` out of any event-handler closure.
    let (close_requested, set_close_requested) = signal(false);
    Effect::new(move |_| {
        if close_requested.get() {
            set_close_requested.set(false);
            on_close();
        }
    });

    let close = move |_: web_sys::MouseEvent| {
        let s = enroll_state.get_untracked();
        if matches!(
            s,
            EnrollState::Idle | EnrollState::Success | EnrollState::Error
        ) {
            set_enroll_state.set(EnrollState::Idle);
            set_error_msg.set(String::new());
            set_close_requested.set(true);
        }
    };

    view! {
        <Show when=move || show.get()>
            // Backdrop
            <div
                class="fixed inset-0 z-40 bg-longan-950/80"
                on:click=close
            />
            // Modal panel
            <div class="fixed inset-0 z-50 flex items-center justify-center p-4 pointer-events-none">
                <div
                    class="pointer-events-auto bg-longan-900 inset-ring inset-ring-longan-700 \
                           rounded-grape p-5 w-full max-w-sm space-y-4"
                    on:click=|e: web_sys::MouseEvent| e.stop_propagation()
                >
                    <div class="flex items-center justify-between">
                        <div class="text-h4 text-rambutan-100">
                            "Enroll voice"
                        </div>
                        <button
                            type="button"
                            class="text-label text-rambutan-500 hover:text-rambutan-300 cursor-pointer"
                            on:click=close
                        >
                            "✕"
                        </button>
                    </div>

                    <div class="text-label text-rambutan-400">
                        "Record a clean 5-second voice sample for "
                        <span class="text-rambutan-200">
                            {move || person_name.get()}
                        </span>
                        ". Speak naturally. This replaces any existing voice reference."
                    </div>

                    // Status area
                    <div class="flex flex-col items-center gap-3 py-3 min-h-[60px]">
                        {move || match enroll_state.get() {
                            EnrollState::Idle => view! {
                                <div class="text-label text-rambutan-300">
                                    "Click Record to begin."
                                </div>
                            }.into_any(),
                            EnrollState::Countdown(n) => view! {
                                <div class="text-display-sm text-persimmon-400 leading-none">
                                    {n.to_string()}
                                </div>
                                <div class="text-label text-rambutan-400">
                                    "Get ready…"
                                </div>
                            }.into_any(),
                            EnrollState::Recording => view! {
                                <div class="flex items-center gap-2">
                                    <span class="w-2.5 h-2.5 rounded-orb bg-persimmon-500 animate-pulse" />
                                    <span class="text-body-sm text-persimmon-400">
                                        "Recording 5s…"
                                    </span>
                                </div>
                                <div class="text-label-sm text-rambutan-300">
                                    "Speak clearly and naturally."
                                </div>
                            }.into_any(),
                            EnrollState::Uploading => view! {
                                <div class="text-label text-rambutan-400 animate-pulse">
                                    "Uploading…"
                                </div>
                            }.into_any(),
                            EnrollState::Success => view! {
                                <div class="text-body-sm text-pandan-400">
                                    "✓ Voice enrolled successfully"
                                </div>
                            }.into_any(),
                            EnrollState::Error => view! {
                                <div class="text-label text-lychee-400 text-center">
                                    {move || error_msg.get()}
                                </div>
                            }.into_any(),
                        }}
                    </div>

                    // Buttons
                    <div class="flex justify-end gap-2">
                        {move || {
                            let s = enroll_state.get();
                            let is_idle = s == EnrollState::Idle;
                            let is_error = s == EnrollState::Error;
                            view! {
                                <Show when=move || !matches!(enroll_state.get(), EnrollState::Recording | EnrollState::Uploading)>
                                    <button
                                        type="button"
                                        class="text-label text-rambutan-400 hover:text-rambutan-200 \
                                               px-3 py-1 rounded-seed cursor-pointer"
                                        on:click=close
                                    >
                                        "Cancel"
                                    </button>
                                </Show>
                                <Show when=move || is_idle>
                                    <button
                                        type="button"
                                        class="text-label bg-persimmon-600 hover:bg-persimmon-500 \
                                               text-white px-3 py-1 rounded-seed cursor-pointer \
                                               transition-colors"
                                        on:click=start_enrollment
                                    >
                                        "Record (5s)"
                                    </button>
                                </Show>
                                <Show when=move || is_error>
                                    <button
                                        type="button"
                                        class="text-label bg-longan-800 hover:bg-longan-700 \
                                               text-rambutan-200 px-3 py-1 rounded-seed cursor-pointer \
                                               transition-colors"
                                        on:click=move |_| {
                                            set_enroll_state.set(EnrollState::Idle);
                                            set_error_msg.set(String::new());
                                        }
                                    >
                                        "Try again"
                                    </button>
                                </Show>
                            }
                        }}
                    </div>
                </div>
            </div>
        </Show>
    }
}

// ── Audio capture helpers ────────────────────────────────────────────────────

/// Why a voice sample could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum VoiceCaptureError {
    /// A browser audio API refused (`step` says which); the browser's reason
    /// follows.
    #[error("{step} failed: {reason}")]
    Browser { step: &'static str, reason: String },
    #[error("no audio samples were captured")]
    NoSamples,
    #[error(transparent)]
    TooLong(#[from] bruhs_models::logic::wav::WavTooLong),
}

/// A browser failure at `step`.
fn browser(step: &'static str) -> impl FnOnce(JsValue) -> VoiceCaptureError {
    move |error| VoiceCaptureError::Browser {
        step,
        reason: error
            .dyn_ref::<js_sys::Error>()
            .and_then(|error| error.message().as_string())
            .unwrap_or_else(|| format!("{error:?}")),
    }
}

/// Owns everything a recording holds open: the microphone tracks, the audio
/// graph and the callback that fills the sample buffer. Dropping it — on
/// success, on an error part-way through set-up, or when the enrolling
/// future is dropped — turns the microphone off and releases them all.
struct Recorder {
    stream: web_sys::MediaStream,
    context: Option<web_sys::AudioContext>,
    processor: Option<web_sys::ScriptProcessorNode>,
    on_audio: Option<Closure<dyn FnMut(web_sys::AudioProcessingEvent)>>,
    samples: std::rc::Rc<std::cell::RefCell<Vec<f32>>>,
}

impl Recorder {
    fn new(stream: web_sys::MediaStream) -> Self {
        Self {
            stream,
            context: None,
            processor: None,
            on_audio: None,
            samples: Default::default(),
        }
    }

    /// The samples recorded so far, leaving the buffer empty.
    fn take_samples(&self) -> Vec<f32> {
        std::mem::take(&mut *self.samples.borrow_mut())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let tracks = self.stream.get_audio_tracks();
        for i in 0..tracks.length() {
            if let Ok(track) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                track.stop();
            }
        }
        if let Some(processor) = &self.processor {
            // Detach the callback before `on_audio` (the closure) is freed.
            processor.set_onaudioprocess(None);
            // Teardown is best-effort: a node that is already disconnected or
            // a context that is already closed has nothing left to release.
            let _ = processor.disconnect();
        }
        if let Some(context) = &self.context {
            let _ = context.close();
        }
        self.on_audio = None;
    }
}

/// Capture `duration_ms` milliseconds of mono audio from the microphone via
/// Web Audio API's AudioContext + ScriptProcessorNode. Encodes the result as a
/// 16-bit PCM WAV. The AudioContext requests 16kHz; browsers that round up to
/// 48kHz still produce valid WAV — the daemon resamples to 16kHz via hound/rubato.
async fn capture_voice_sample_wav(duration_ms: u32) -> Result<Vec<u8>, VoiceCaptureError> {
    let window = web_sys::window().ok_or(VoiceCaptureError::Browser {
        step: "window",
        reason: "no window".into(),
    })?;

    // Request microphone.
    let media_devices = window
        .navigator()
        .media_devices()
        .map_err(browser("mediaDevices"))?;
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::TRUE);
    constraints.set_video(&JsValue::FALSE);
    let stream_promise = media_devices
        .get_user_media_with_constraints(&constraints)
        .map_err(browser("getUserMedia"))?;
    let stream: web_sys::MediaStream = wasm_bindgen_futures::JsFuture::from(stream_promise)
        .await
        .map_err(browser("getUserMedia"))?
        .dyn_into()
        .map_err(browser("getUserMedia"))?;
    // From here on every exit path turns the microphone off.
    let mut recorder = Recorder::new(stream);

    // AudioContext at 16kHz (browsers may clamp to 44100/48000; handled by daemon).
    let ctx_opts = web_sys::AudioContextOptions::new();
    ctx_opts.set_sample_rate(16_000.0);
    let context = web_sys::AudioContext::new_with_context_options(&ctx_opts)
        .map_err(browser("AudioContext"))?;
    let sample_rate = context.sample_rate().to_u32_saturating();
    let context = recorder.context.insert(context);

    let source = context
        .create_media_stream_source(&recorder.stream)
        .map_err(browser("createMediaStreamSource"))?;

    // 4096 buffer, 1 input channel (mono), 1 output channel.
    let processor = context
        .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
            4096, 1, 1,
        )
        .map_err(browser("createScriptProcessor"))?;
    let destination = context.destination();
    let processor = recorder.processor.insert(processor);

    let samples = recorder.samples.clone();
    let on_audio = Closure::<dyn FnMut(web_sys::AudioProcessingEvent)>::new(
        move |ev: web_sys::AudioProcessingEvent| {
            if let Ok(buf) = ev.input_buffer() {
                if let Ok(data) = buf.get_channel_data(0) {
                    samples.borrow_mut().extend(data);
                }
            }
        },
    );
    processor.set_onaudioprocess(Some(on_audio.as_ref().unchecked_ref()));
    recorder.on_audio = Some(on_audio);

    source
        .connect_with_audio_node(processor)
        .map_err(browser("connect source"))?;
    processor
        .connect_with_audio_node(&destination)
        .map_err(browser("connect processor"))?;

    // Record for requested duration.
    gloo_timers::future::TimeoutFuture::new(duration_ms).await;

    let pcm = recorder.take_samples();
    drop(recorder);
    if pcm.is_empty() {
        return Err(VoiceCaptureError::NoSamples);
    }
    Ok(bruhs_models::logic::wav::encode_wav_16bit_mono(
        &pcm,
        sample_rate,
    )?)
}
