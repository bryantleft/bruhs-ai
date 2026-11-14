//! Local-only dictation. WAV data is temporary and never becomes a meeting.
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
    if samples.is_empty() || samples.iter().all(|sample| *sample == 0.0) {
        return Ok(Json(DictationTranscript {
            text: String::new(),
            model: model.key().into(),
        }));
    }
    let path = state.models.ensure_whisper(model, |_, _| {}).await.map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, format!("Local dictation model is unavailable: {error}. Check the model download and retry.")))?;
    let language = options
        .language
        .or_else(|| (!model.descriptor().multilingual).then(|| "en".into()));
    let text = tokio::task::spawn_blocking(move || {
        // Hold the single inference slot until the blocking worker exits, including
        // when its HTTP caller disconnects.
        let _permit = permit;
        crate::transcribe::whisper::transcribe_samples(&path, &samples, language.as_deref(), |_| {})
            .map(|segments| {
                segments
                    .into_iter()
                    .map(|segment| segment.text)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
    })
    .await
    .map_err(internal)?
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Local dictation failed: {error}"),
        )
    })?;
    Ok(Json(DictationTranscript {
        text,
        model: model.key().into(),
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

    #[tokio::test]
    async fn silent_audio_returns_no_invented_text_and_creates_no_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::CaptureState::new(crate::paths::DataRoot::at(dir.path()));
        let bytes =
            bruhs_models::logic::wav::encode_wav_16bit_mono(&[0.0; 16_000], 16_000).unwrap();
        let result = transcribe(
            State(Arc::new(state)),
            Query(DictationOptions {
                model: bruhs_models::WhisperModel::BaseEn,
                language: Some("en".into()),
            }),
            Bytes::from(bytes),
        )
        .await
        .unwrap();
        assert!(result.text.is_empty());
    }
}
