//! Post-recording pipeline: mix mic+sys → transcribe → diarize →
//! sub-diarize → write segments. Extracted from routes/capture.rs
//! so it can be unit-tested without an HTTP server.

use std::path::PathBuf;
use std::sync::Arc;

use crate::state::CaptureState;

mod worker;

#[cfg(test)]
mod tests;

/// Typed errors for the diarize/subdiarize/OCR spawn_blocking step.
/// Replaces `Box<dyn std::error::Error + Send + Sync>` so telemetry and
/// callers can distinguish which stage failed. Subdiarize/Ocr stages
/// currently only warn-log internally; if those promote to hard errors,
/// add variants then.
#[derive(Debug, thiserror::Error)]
enum PostprocessStepError {
    #[error("envelope: {0}")]
    Envelope(String),
    #[error("segments: {0}")]
    Segments(String),
    #[error("io: {0}")]
    Io(String),
}

pub struct PostprocessOpts {
    pub meeting_id: String,
    pub started_at_ms: f64,
    pub mic_wav: PathBuf,
    pub sys_wav: PathBuf,
    pub mixed_wav: PathBuf,
    pub transcription: crate::transcribe::TranscriptionJob,
    pub subspeaker_enabled: bool,
    pub max_speakers: crate::state::MaxSpeakers,
    pub video_ocr_enabled: bool,
    pub cross_meeting_linking_auto: bool,
    pub cross_meeting_linking_threshold: crate::state::LinkingThreshold,
}

pub async fn postprocess_meeting(state: Arc<CaptureState>, opts: PostprocessOpts) {
    let meeting_id = opts.meeting_id;
    let started_at_ms = opts.started_at_ms;
    let mic_wav = opts.mic_wav;
    let sys_wav = opts.sys_wav;
    let mixed_wav = opts.mixed_wav;
    let transcription = opts.transcription;
    let subspeaker_enabled = opts.subspeaker_enabled;
    let max_speakers = opts.max_speakers.get();
    let video_ocr_enabled = opts.video_ocr_enabled;
    let linking_auto = opts.cross_meeting_linking_auto;
    let linking_threshold = opts.cross_meeting_linking_threshold.get();

    // 1. Mix mic + sys (if present) into mixed.wav before transcription.
    let mic_path = mic_wav.clone();
    let sys_path = sys_wav.clone();
    let mixed_path = mixed_wav.clone();
    let mix_result =
        tokio::task::spawn_blocking(move || crate::mix::mix(&mic_path, &sys_path, &mixed_path))
            .await;
    if worker::finish_required(&state, &meeting_id, started_at_ms, "mix", mix_result)
        .await
        .is_err()
    {
        return;
    }

    let join_result = match transcription {
        crate::transcribe::TranscriptionJob::Compatible(job) => Ok(job
            .transcribe(state.clone(), meeting_id.clone(), mixed_wav.clone())
            .await
            .map_err(crate::transcribe::TranscribeError::Remote)),
        crate::transcribe::TranscriptionJob::Whisper(model_for_task) => {
            let model_path = match state
                .models
                .ensure_whisper(model_for_task, |downloaded, total| {
                    tracing::info!(
                        "model download: {downloaded} / {} bytes",
                        total.map_or_else(|| "?".into(), |t| t.to_string())
                    );
                })
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("whisper model unavailable: {e}");
                    worker::fail(
                        &state,
                        &meeting_id,
                        started_at_ms,
                        format!("model download failed: {e}"),
                    )
                    .await;
                    return;
                }
            };

            let state_inner = state.clone();
            let mid_inner = meeting_id.clone();
            let wav_inner = mixed_wav.clone();
            tokio::task::spawn_blocking(move || {
                crate::transcribe::transcribe_wav(
                    state_inner,
                    mid_inner,
                    &wav_inner,
                    &model_path,
                    model_for_task,
                )
            })
            .await
        }
    };

    if let Ok(()) = worker::finish_required(
        &state,
        &meeting_id,
        started_at_ms,
        "transcription",
        join_result,
    )
    .await
    {
        // 3. If subspeaker_enabled, ensure the sherpa speaker model is cached.
        let sherpa_path_opt: Option<std::path::PathBuf> = if subspeaker_enabled {
            match state
                .models
                .ensure_sherpa_speaker(|d, t| {
                    tracing::info!(
                        "sherpa model download: {d} / {} bytes",
                        t.map_or_else(|| "?".into(), |t| t.to_string())
                    );
                })
                .await
            {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!(
                        "sherpa speaker model unavailable: {e}. Skipping subdiarization."
                    );
                    None
                }
            }
        } else {
            tracing::info!(
                "subspeaker_enabled=false; skipping sherpa model download and subdiarization."
            );
            None
        };

        // 4. spawn_blocking: diarize + subdiarize.
        let mic_path_for_diarize = mic_wav.clone();
        let sys_path_for_diarize = sys_wav.clone();
        let mid_for_diarize = meeting_id.clone();
        let data = state.data.clone();
        let diarize_result =
                tokio::task::spawn_blocking(move || -> Result<(), PostprocessStepError> {
                    let segments_jsonl = data
                        .segments_jsonl_path(&mid_for_diarize)
                        .map_err(|e| PostprocessStepError::Io(e.to_string()))?;
                    let mut segments =
                        crate::transcribe::segments_log::load_snapshot(&segments_jsonl)
                            .map_err(|e| PostprocessStepError::Segments(e.to_string()))?;
                    let mic_env = crate::diarize::compute_envelope(&mic_path_for_diarize)
                        .map_err(|e| PostprocessStepError::Envelope(e.to_string()))?;
                    let sys_env = if sys_path_for_diarize.exists() {
                        Some(
                            crate::diarize::compute_envelope(&sys_path_for_diarize)
                                .map_err(|e| PostprocessStepError::Envelope(e.to_string()))?,
                        )
                    } else {
                        None
                    };
                    crate::diarize::assign_speakers(&mut segments, &mic_env, sys_env.as_ref());

                    // Refine "them" → "them_1", "them_2", … via sherpa embeddings.
                    if let Some(ref sherpa_path) = sherpa_path_opt {
                        if sys_path_for_diarize.exists() {
                            match crate::subdiarize::assign_subspeakers(
                                &mut segments,
                                &sys_path_for_diarize,
                                sherpa_path,
                                max_speakers,
                            ) {
                                Ok((n, centroids)) => {
                                    tracing::info!("subdiarize: assigned {n} speaker clusters");
                                    if !centroids.is_empty() {
                                        match data.ensure_meeting_dir(&mid_for_diarize).and_then(|_| data.centroids_path(&mid_for_diarize)) {
                                            Ok(path) => {
                                                let file = bruhs_models::CentroidsFile {
                                                    linking_threshold,
                                                    linking_auto,
                                                    centroids,
                                                };
                                                match serde_json::to_string(&file) {
                                                    Ok(json) => {
                                                        if let Err(e) = crate::paths::write_private_atomic(&path, json.as_bytes())
                                                        {
                                                            tracing::warn!(
                                                            "failed to write centroids.json: {e}"
                                                        );
                                                        }
                                                    }
                                                    Err(e) => tracing::warn!(
                                                        "centroid serialization failed: {e}"
                                                    ),
                                                }
                                            }
                                            Err(e) => tracing::warn!(
                                                "centroids path resolution failed: {e}"
                                            ),
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "subdiarize failed: {e}. Keeping Phase 3 'them' label."
                                    );
                                }
                            }
                        }
                    }

                    // 5. OCR produces unverified hints only. Keep the canonical
                    // cluster IDs aligned with centroids; visible names do not
                    // establish who spoke and must not enroll a voice identity.
                    if video_ocr_enabled {
                        if let Ok(ocr_path) = data.ocr_path(&mid_for_diarize) {
                            if ocr_path.exists() {
                                match std::fs::read_to_string(&ocr_path) {
                                    Ok(text) => {
                                        match serde_json::from_str::<
                                            Vec<crate::video_ocr::OcrObservation>,
                                        >(&text)
                                        {
                                            Ok(observations) => {
                                                let hints = crate::subdiarize::ocr_name_hints(
                                                    &segments,
                                                    &observations,
                                                );
                                                tracing::info!(
                                                hints = hints.len(),
                                                "Unverified OCR name hints; speaker identities unchanged"
                                            );
                                            }
                                            Err(e) => tracing::warn!("ocr.json parse failed: {e}"),
                                        }
                                    }
                                    Err(e) => tracing::warn!("ocr.json read failed: {e}"),
                                }
                            }
                        }
                    }

                    // 6. Persist final segments.
                    crate::transcribe::segments_log::write_all(&segments_jsonl, &segments)
                        .map_err(|e| PostprocessStepError::Io(e.to_string()))?;
                    Ok(())
                })
                .await;

        worker::finish_final(&state, &meeting_id, started_at_ms, diarize_result).await;
    }
}
