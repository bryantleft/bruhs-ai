//! POST /capture/start and POST /capture/stop.

use axum::{
    extract::{rejection::JsonRejection, State},
    http::StatusCode,
    Json,
};
use bruhs_models::{MeetingEvent, MeetingStatus, WhisperModel};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::audio::MicRecorder;
use crate::state::{CaptureState, CurrentSession, LinkingThreshold, MaxSpeakers};
use crate::transcribe::TranscriptionJob;
#[cfg(target_os = "macos")]
use crate::video_ocr::OcrError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub meeting_id: String,
    pub started_at_ms: f64,
    #[serde(default)]
    pub model: Option<WhisperModel>,
    #[serde(default)]
    pub live_model: Option<WhisperModel>,
    #[serde(default)]
    pub transcription: Option<bruhs_models::TranscriptionSettings>,
    #[serde(default)]
    pub subspeaker_enabled: Option<bool>,
    /// Phase 3d: whether to auto-link clusters to existing profiles.
    #[serde(default)]
    pub cross_meeting_linking_auto: Option<bool>,
    /// Phase 3d: cosine similarity threshold for cross-meeting linking.
    #[serde(default)]
    pub cross_meeting_linking_threshold: Option<f32>,
    /// Voice ID v2: maximum number of sub-speaker clusters (2–20, default 6).
    #[serde(default)]
    pub max_speakers: Option<u32>,
    /// Phase Vision: enable video-frame OCR (2fps) to extract participant
    /// names. Default false (opt-in, privacy).
    #[serde(default)]
    pub video_ocr_enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct StartResponse {
    pub meeting_id: String,
    pub wav_path: String,
}

#[derive(Debug, Serialize)]
pub struct StopResponse {
    pub meeting_id: String,
    pub ended_at_ms: f64,
    pub wav_path: String,
}

/// POST /capture/start. Every invalid body is a 400: malformed JSON, a
/// wrong type or unknown option (axum's default would be 422) and an
/// out-of-range option alike.
pub async fn start_capture(
    State(state): State<Arc<CaptureState>>,
    body: Result<Json<StartRequest>, JsonRejection>,
) -> Result<Json<StartResponse>, (StatusCode, String)> {
    let Json(req) = body.map_err(|rejection| (StatusCode::BAD_REQUEST, rejection.body_text()))?;
    crate::paths::validate_meeting_id(&req.meeting_id)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    // A session must be representable by its durable recovery identity before
    // any paths or recording devices are opened (also rejects NaN/infinity).
    if !(0.0..=8_640_000_000_000_000.0).contains(&req.started_at_ms) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid recording start time".into(),
        ));
    }
    let bad_request =
        |error: crate::state::StartOptionError| (StatusCode::BAD_REQUEST, error.to_string());
    let transcription = req.transcription.clone().unwrap_or_default();
    transcription
        .validate()
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    let transcription = match transcription.provider {
        bruhs_models::TranscriptionProvider::LocalWhisper => {
            TranscriptionJob::Whisper(req.model.unwrap_or_default())
        }
        bruhs_models::TranscriptionProvider::OpenAiCompatible => TranscriptionJob::Compatible(
            state
                .remote_transcription
                .prepare(&transcription.remote_model)
                .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?,
        ),
    };
    let options = ValidatedOptions {
        transcription,
        max_speakers: req
            .max_speakers
            .map(MaxSpeakers::new)
            .transpose()
            .map_err(bad_request)?,
        linking_threshold: req
            .cross_meeting_linking_threshold
            .map(LinkingThreshold::new)
            .transpose()
            .map_err(bad_request)?,
    };
    let lifecycle = state.lifecycle.clone();
    lifecycle
        .run(move || start_capture_blocking(state, req, options))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("capture start worker: {error}"),
            )
        })?
}

/// Range-checked start options (400 when out of range).
struct ValidatedOptions {
    transcription: TranscriptionJob,
    max_speakers: Option<MaxSpeakers>,
    linking_threshold: Option<LinkingThreshold>,
}

fn start_capture_blocking(
    state: Arc<CaptureState>,
    req: StartRequest,
    options: ValidatedOptions,
) -> Result<Json<StartResponse>, (StatusCode, String)> {
    if state.with_current(|cur| cur.is_some()) {
        return Err((StatusCode::CONFLICT, "already recording".into()));
    }

    let internal = |what: &str, error: std::io::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{what}: {error}"),
        )
    };
    let data = &state.data;
    let mic_wav_path = data
        .mic_wav_path(&req.meeting_id)
        .map_err(|e| internal("mic path", e))?;
    let sys_wav_path = data
        .sys_wav_path(&req.meeting_id)
        .map_err(|e| internal("sys path", e))?;
    let mixed_wav_path = data
        .mixed_wav_path(&req.meeting_id)
        .map_err(|e| internal("mixed path", e))?;
    data.ensure_meeting_dir(&req.meeting_id)
        .map_err(|e| internal("meeting dir", e))?;

    let mic_recorder = MicRecorder::start(
        &mic_wav_path,
        Some(state.tx.clone()),
        Some(req.meeting_id.clone()),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mic recorder: {e}"),
        )
    })?;

    let video_ocr_enabled = req.video_ocr_enabled.unwrap_or(false);

    // System audio is non-fatal — fall back to mic-only on failure.
    // When video_ocr_enabled, use start_with_opts so SCK captures video frames.
    let sys_recorder_result = if video_ocr_enabled {
        crate::capture::start_with_opts(&sys_wav_path, true)
    } else {
        crate::capture::start(&sys_wav_path)
    };
    let sys_recorder = match sys_recorder_result {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!("system audio capture unavailable: {e}");
            state.emit(MeetingEvent::CaptureWarning {
                meeting_id: req.meeting_id.clone(),
                message: format!("system audio unavailable: {e}"),
            });
            None
        }
    };

    let model = req.model.unwrap_or_default();
    let live_model = req.live_model.unwrap_or(model);
    let subspeaker_enabled = req.subspeaker_enabled.unwrap_or(true);

    let generation = uuid::Uuid::new_v4();
    let session = CurrentSession {
        generation,
        meeting_id: req.meeting_id.clone(),
        started_at_ms: req.started_at_ms,
        mic_wav_path: mic_wav_path.clone(),
        sys_wav_path: sys_wav_path.clone(),
        mixed_wav_path: mixed_wav_path.clone(),
        transcription: options.transcription,
        live_model: Some(live_model),
        subspeaker_enabled,
        cross_meeting_linking_auto: req.cross_meeting_linking_auto,
        cross_meeting_linking_threshold: options.linking_threshold,
        max_speakers: options.max_speakers,
        video_ocr_enabled,
    };

    // Store the sys recorder. On macOS with video_ocr_enabled, also extract
    // the video frame channel and spin up an OcrWorker.
    if let Some(mut sys) = sys_recorder {
        #[cfg(target_os = "macos")]
        if video_ocr_enabled {
            if let Some(frames) = sys.take_video_frames() {
                let start_instant = std::time::Instant::now();
                let worker = state
                    .data
                    .ocr_path(&req.meeting_id)
                    .map_err(OcrError::from)
                    .and_then(|ocr_path| {
                        crate::video_ocr::OcrWorker::start(frames, ocr_path, start_instant)
                    });
                match worker {
                    Ok(worker) => {
                        state.set_ocr_worker(worker);
                        tracing::info!("OcrWorker started for meeting {}", req.meeting_id);
                    }
                    Err(e) => {
                        tracing::warn!("OcrWorker unavailable: {e}");
                        state.emit(MeetingEvent::CaptureWarning {
                            meeting_id: req.meeting_id.clone(),
                            message: format!("video OCR unavailable: {e}"),
                        });
                    }
                }
            } else {
                tracing::warn!("video_ocr_enabled but no video frames available");
            }
        }
        state.set_sys_recorder(sys);
    }

    state.set_mic_recorder(mic_recorder);
    state.with_current_mut(|cur| *cur = Some(session));
    state.emit(MeetingEvent::Started {
        meeting_id: req.meeting_id.clone(),
        started_at_ms: req.started_at_ms,
    });

    // Resolve the live Whisper model and spawn the live transcriber.
    // Non-fatal: capture continues without live transcripts if model is unavailable.
    {
        let meeting_id_for_lt = req.meeting_id.clone();
        let sys_wav_path_for_lt = sys_wav_path.clone();
        tokio::spawn(async move {
            let model_path_opt = match state
                .models
                .ensure_whisper(live_model, |d, t| {
                    tracing::info!(
                        "live model download: {d} / {} bytes",
                        t.map_or_else(|| "?".into(), |t| t.to_string())
                    );
                })
                .await
            {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!(
                        "live model unavailable at /capture/start: {e}. Live transcription disabled this session."
                    );
                    None
                }
            };

            if let Some(model_path) = model_path_opt {
                // Fence installation against stop/start, including a repeated
                // meeting ID. Model resolution does not hold this reservation.
                let _reservation = state.lifecycle.enter().await;
                // Guard against orphan: if /capture/stop fired during the
                // model resolve (or a new session started), don't install
                // the transcriber — it would write partials past Ended.
                let still_current = state.is_current_generation(generation);
                if !still_current {
                    tracing::info!(
                        "session for {meeting_id_for_lt} no longer current; skipping LiveTranscriber install"
                    );
                    return;
                }
                match crate::live_transcribe::LiveTranscriber::start(
                    mic_wav_path,
                    model_path.clone(),
                    live_model,
                    "me",
                    state.clone(),
                    meeting_id_for_lt.clone(),
                ) {
                    Ok(lt) => state.set_live_transcriber(lt),
                    Err(e) => tracing::warn!("LiveTranscriber::start (mic) failed: {e}"),
                }

                // Spawn a second transcriber for sys.wav when system audio
                // capture is active. Same orphan guard: meeting must still
                // be current at install time.
                if state.has_sys_recorder() {
                    let sys_wav = sys_wav_path_for_lt.clone();
                    match crate::live_transcribe::LiveTranscriber::start(
                        sys_wav,
                        model_path,
                        live_model,
                        "them",
                        state.clone(),
                        meeting_id_for_lt,
                    ) {
                        Ok(lt) => state.set_sys_live_transcriber(lt),
                        Err(e) => tracing::warn!("LiveTranscriber::start (sys) failed: {e}"),
                    }
                }
            }
        });
    }

    Ok(Json(StartResponse {
        meeting_id: req.meeting_id,
        wav_path: mixed_wav_path.to_string_lossy().into_owned(),
    }))
}

#[cfg(test)]
mod tests;

/// POST /capture/stop
pub async fn stop_capture(
    State(state): State<Arc<CaptureState>>,
) -> Result<Json<StopResponse>, (StatusCode, String)> {
    let lifecycle = state.lifecycle.clone();
    lifecycle
        .run(move || stop_capture_blocking(state))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("capture stop worker: {error}"),
            )
        })?
}

fn stop_capture_blocking(
    state: Arc<CaptureState>,
) -> Result<Json<StopResponse>, (StatusCode, String)> {
    let session = state
        .with_current(|cur| cur.cloned())
        .ok_or((StatusCode::CONFLICT, "not recording".into()))?;
    let mic_recorder = state.take_mic_recorder();
    let sys_recorder = state.take_sys_recorder();

    // Halt both live transcribers before stopping recorders so SSE events
    // remain ordered: live partials quiesce before Ended/SpeakersAssigned arrive.
    if let Some(live) = state.take_live_transcriber() {
        if let Err(e) = live.stop() {
            tracing::warn!("mic live transcriber stop failed: {e}");
        }
    }
    if let Some(live) = state.take_sys_live_transcriber() {
        if let Err(e) = live.stop() {
            tracing::warn!("sys live transcriber stop failed: {e}");
        }
    }

    let mic_stop = mic_recorder
        .ok_or_else(|| "mic recorder missing".to_string())
        .and_then(|recorder| {
            recorder
                .stop()
                .map_err(|error| format!("mic stop: {error}"))
        });
    // A recorder that never started is optional. Once started, its lost audio
    // must not become a successfully completed meeting.
    let sys_stop = sys_recorder.map_or(Ok(()), |sys| {
        sys.stop().map_err(|error| format!("sys stop: {error}"))
    });
    // Phase Vision: stop the OcrWorker only after the system recorder, whose
    // stream owned the frame sender. The worker writes ocr.json and exits.
    if let Some(worker) = state.take_ocr_worker() {
        match worker.stop() {
            Ok(obs) => tracing::info!("OcrWorker stopped: {} observations", obs.len()),
            Err(e) => tracing::warn!("OcrWorker stop failed: {e}"),
        }
    }

    // Keep recording status true until every recorder has been stopped. A
    // microphone error must not skip cleanup of the remote-audio recorder.
    state.with_current_mut(|cur| *cur = None);
    let stopped = match (mic_stop, sys_stop) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(mic), Err(sys)) => Err(format!("{mic}; {sys}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
    };
    if let Err(message) = stopped {
        let failed_at_ms = bruhs_models::EpochMs::now()
            .as_f64()
            .max(session.started_at_ms);
        if let Err(error) = state.recovery.record_failure_blocking(
            &session.meeting_id,
            session.started_at_ms,
            failed_at_ms,
        ) {
            tracing::warn!("capture failure recovery could not be saved: {error}");
            state.emit(MeetingEvent::CaptureWarning {
                meeting_id: session.meeting_id.clone(),
                message: "Recording failed and its recovery status could not be saved. Keep the recording files and check this meeting before closing the app.".into(),
            });
        }
        state.emit(MeetingEvent::CaptureError {
            meeting_id: session.meeting_id,
            message: message.clone(),
        });
        return Err((StatusCode::INTERNAL_SERVER_ERROR, message));
    }

    let ended_at_ms = bruhs_models::EpochMs::now().as_f64();
    state.emit(MeetingEvent::Ended {
        meeting_id: session.meeting_id.clone(),
        ended_at_ms,
        status: MeetingStatus::Ended,
    });

    let mixed_path_for_response = session.mixed_wav_path.clone();
    let opts = crate::postprocess::PostprocessOpts {
        meeting_id: session.meeting_id.clone(),
        started_at_ms: session.started_at_ms,
        mic_wav: session.mic_wav_path.clone(),
        sys_wav: session.sys_wav_path.clone(),
        mixed_wav: session.mixed_wav_path.clone(),
        transcription: session.transcription,
        subspeaker_enabled: session.subspeaker_enabled,
        max_speakers: session.max_speakers.unwrap_or(MaxSpeakers::DEFAULT),
        video_ocr_enabled: session.video_ocr_enabled,
        cross_meeting_linking_auto: session.cross_meeting_linking_auto.unwrap_or(true),
        cross_meeting_linking_threshold: session
            .cross_meeting_linking_threshold
            .unwrap_or(LinkingThreshold::DEFAULT),
    };
    let state_clone = state.clone();
    tokio::spawn(async move {
        crate::postprocess::postprocess_meeting(state_clone, opts).await;
    });

    Ok(Json(StopResponse {
        meeting_id: session.meeting_id,
        ended_at_ms,
        wav_path: mixed_path_for_response.to_string_lossy().into_owned(),
    }))
}
