//! Temporary composer audio. The local route always uses Whisper. A distinct
//! authenticated route sends audio to the host-configured compatible service.
pub mod runtime;
pub mod cloud;
pub mod models;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use bruhs_models::local_tools::{DictationOptions, DictationTranscript};
use std::{io::Write, sync::Arc};

static INFERENCE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

pub async fn transcribe(
    State(state): State<Arc<crate::state::CaptureState>>,
    Query(options): Query<DictationOptions>,
    body: Bytes,
) -> Result<Json<DictationTranscript>, (StatusCode, String)> {
    run(state, options, body, None).await
}

#[derive(serde::Deserialize)]
pub struct CompatibleModel {
    remote_model: String,
}

pub async fn compatible(
    State(state): State<Arc<crate::state::CaptureState>>,
    Query(options): Query<DictationOptions>,
    Query(model): Query<CompatibleModel>,
    body: Bytes,
) -> Result<Json<DictationTranscript>, (StatusCode, String)> {
    let model = model.remote_model.trim();
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Enter a valid server transcription model in Voice settings.".into(),
        ));
    }
    let job = state
        .remote_transcription
        .prepare(model)
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    run(state, options, body, Some((model.to_owned(), job))).await
}

async fn run(
    state: Arc<crate::state::CaptureState>,
    options: DictationOptions,
    body: Bytes,
    remote: Option<(String, crate::transcribe::remote::RemoteJob)>,
) -> Result<Json<DictationTranscript>, (StatusCode, String)> {
    let permit = INFERENCE.try_acquire().map_err(|_| {
        (
            StatusCode::CONFLICT,
            "Another dictation is transcribing. Wait for it to finish and retry.".into(),
        )
    })?;
    if options.language.as_ref().is_some_and(|language| {
        language.is_empty()
            || !language
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose a language code such as en or es.".into(),
        ));
    }
    // Decode before loading a model. Invalid or silent input does not download one.
    let samples = tokio::task::spawn_blocking(move || {
        let mut wav = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
        wav.write_all(&body).map_err(|error| error.to_string())?;
        crate::transcribe::decode::decode_wav_from_sample(wav.path(), 0)
            .map(|(samples, _)| samples)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(internal)?
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("Dictation needs a 16-bit PCM WAV recording: {error}"),
        )
    })?;
    let model = options.model;
    let model_name = remote
        .as_ref()
        .map_or_else(|| model.key().to_owned(), |(name, _)| name.clone());
    if samples.is_empty() || samples.iter().all(|sample| *sample == 0.0) {
        return Ok(Json(DictationTranscript {
            text: String::new(),
            model: model_name,
        }));
    }
    // The decoder returns mono 16 kHz audio. Leave some room around detected speech
    // for quiet consonants, and preserve pauses between words.
    let samples = if options.silence_filtering {
        let frames = samples.chunks(320);
        let Some(first) = frames.clone().position(crate::live_transcribe::has_speech) else {
            return Ok(Json(DictationTranscript {
                text: String::new(),
                model: model_name,
            }));
        };
        let last = frames
            .enumerate()
            .rev()
            .find(|(_, frame)| crate::live_transcribe::has_speech(frame))
            .map_or(first, |(index, _)| index);
        let start = first.saturating_sub(10) * 320;
        let end = ((last + 11) * 320).min(samples.len());
        samples[start..end].to_vec()
    } else {
        samples
    };
    let text = if let Some((_, job)) = remote {
        let wav = tokio::task::spawn_blocking(move || {
            bruhs_models::logic::wav::encode_wav_16bit_mono(&samples, 16_000)
        })
        .await
        .map_err(internal)?
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        job.dictate(wav, &options)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?
    } else {
        let path = state.models.ensure_whisper(model, |_, _| {}).await.map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, format!("Local dictation model is unavailable: {error}. Check the model download and retry.")))?;
        let language = options
            .language.clone()
            .or_else(|| (!model.descriptor().multilingual).then(|| "en".into()));
        runtime::infer(&state.dictation, &path, model, samples, language, permit).await?
    };
    Ok(Json(DictationTranscript {
        text: if options.remove_filler_words && !options.client_filters_text {
            bruhs_models::voice_dictionary::without_fillers(&text, options.language.as_deref(), "")
        } else {
            text
        },
        model: model_name,
    }))
}

fn internal(error: tokio::task::JoinError) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Local dictation worker stopped: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filler_cleanup_preserves_words_containing_a_filler() {
        assert_eq!(
            bruhs_models::voice_dictionary::without_fillers("Um, update the album, uh, using uh-huh.", Some("en"), ""),
            "update the album, using uh-huh."
        );
    }

    #[tokio::test]
    async fn silent_audio_returns_no_invented_text_and_creates_no_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::CaptureState::new(crate::paths::DataRoot::at(dir.path()));
        let bytes =
            bruhs_models::logic::wav::encode_wav_16bit_mono(&[0.0; 16_000], 16_000).unwrap();
        let result = transcribe(
            State(Arc::new(state)),
            Query(DictationOptions {
                client_filters_text: false,
                model: bruhs_models::WhisperModel::BaseEn,
                language: Some("en".into()),
                remove_filler_words: true,
                silence_filtering: true,
            }),
            Bytes::from(bytes),
        )
        .await
        .unwrap();
        assert!(result.text.is_empty());
    }
}
