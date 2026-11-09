//! Timestamp-preserving multipart transcription. Service configuration and keys
//! stay in the daemon; browser preferences contain only the provider and model.

use super::{segment_sink::SegmentSink, whisper::DecodedSegment};
use crate::state::CaptureState;
use bruhs_models::{num::ToIntSaturating, TranscriptSegment};
use reqwest::{header, multipart, redirect, Client, Url};
use serde::Deserialize;
use std::{fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};

// The compatible API's file limit is 25 MB. PCM16 WAV uses a 44-byte header.
// https://developers.openai.com/api/docs/guides/speech-to-text#longer-inputs
const MAX_UPLOAD_BYTES: usize = 25_000_000;
const WAV_HEADER_BYTES: usize = 44;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("Transcription provider configuration: {0}")]
    Configuration(&'static str),
    #[error("Transcription server request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("Transcription server returned HTTP {0}. Check the endpoint, API key, model, and verbose_json support.")]
    Status(reqwest::StatusCode),
    #[error("Transcription server response: {0}")]
    Response(&'static str),
    #[error("transcription audio: {0}")]
    Audio(#[from] hound::Error),
    #[error("transcription file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transcription worker was cancelled; recording files were retained.")]
    WorkerCancelled,
    #[error("Transcription worker stopped unexpectedly; recording files were retained.")]
    WorkerPanicked,
    #[error("transcription log: {0}")]
    Log(#[from] super::segments_log::LogError),
}

impl From<tokio::task::JoinError> for RemoteError {
    fn from(error: tokio::task::JoinError) -> Self {
        if error.is_cancelled() {
            Self::WorkerCancelled
        } else {
            Self::WorkerPanicked
        }
    }
}

#[derive(Clone)]
struct RemoteClient {
    client: Client,
    endpoint: Url,
}

pub struct RemoteProvider {
    configuration: Result<Option<RemoteClient>, &'static str>,
}

impl RemoteProvider {
    #[cfg(test)]
    pub fn unconfigured() -> Self {
        Self {
            configuration: Ok(None),
        }
    }

    #[cfg(test)]
    pub fn synthetic(base: &str, key: Option<&str>) -> Self {
        Self {
            configuration: remote_client(base, key, Duration::from_secs(5)).map(Some),
        }
    }

    pub fn from_env() -> Self {
        Self {
            configuration: configure(),
        }
    }

    pub fn prepare(&self, model: &str) -> Result<RemoteJob, RemoteError> {
        match &self.configuration {
            Ok(Some(remote)) => Ok(RemoteJob {
                remote: remote.clone(),
                model: model.trim().to_owned(),
            }),
            Ok(None) => Err(RemoteError::Configuration(
                "Set BRUHS_TRANSCRIPTION_BASE_URL and BRUHS_TRANSCRIPTION_TIMEOUT_SECS in the capture daemon environment, then restart it. See docs/transcription-providers.md.",
            )),
            Err(message) => Err(RemoteError::Configuration(message)),
        }
    }
}

fn configure() -> Result<Option<RemoteClient>, &'static str> {
    let base = match std::env::var("BRUHS_TRANSCRIPTION_BASE_URL") {
        Ok(base) => base,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(_) => return Err("BRUHS_TRANSCRIPTION_BASE_URL must be valid UTF-8."),
    };
    let timeout = std::env::var("BRUHS_TRANSCRIPTION_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|seconds| *seconds > 0)
        .ok_or("Set BRUHS_TRANSCRIPTION_TIMEOUT_SECS to a positive number of seconds based on your provider's request deadline.")?;
    let key = match std::env::var("BRUHS_TRANSCRIPTION_API_KEY") {
        Ok(key) => Some(key),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("BRUHS_TRANSCRIPTION_API_KEY must be valid UTF-8."),
    };
    remote_client(
        &base,
        key.as_deref(),
        Duration::from_secs(u64::from(timeout)),
    )
    .map(Some)
}

fn remote_client(
    base: &str,
    key: Option<&str>,
    timeout: Duration,
) -> Result<RemoteClient, &'static str> {
    let mut endpoint = Url::parse(base)
        .map_err(|_| "BRUHS_TRANSCRIPTION_BASE_URL must be an absolute HTTP(S) API base URL.")?;
    let loopback = endpoint.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback))
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err("Use an HTTPS API base URL (HTTP is allowed only on loopback), without credentials, query parameters, or a fragment.");
    }
    endpoint.set_path(&format!(
        "{}/audio/transcriptions",
        endpoint.path().trim_end_matches('/')
    ));
    let mut headers = header::HeaderMap::new();
    if let Some(key) = key.filter(|key| !key.is_empty()) {
        let mut authorization = header::HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "BRUHS_TRANSCRIPTION_API_KEY is not a valid HTTP credential.")?;
        authorization.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, authorization);
    }
    let builder = Client::builder()
        .default_headers(headers)
        .redirect(redirect::Policy::none())
        .timeout(timeout);
    let builder = if loopback {
        builder.no_proxy()
    } else {
        builder
    };
    let client = builder
        .build()
        .map_err(|_| "Could not initialize the transcription HTTP client.")?;
    Ok(RemoteClient { client, endpoint })
}

#[derive(Clone)]
pub struct RemoteJob {
    remote: RemoteClient,
    model: String,
}

impl std::fmt::Debug for RemoteJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteJob")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl RemoteJob {
    pub async fn transcribe(
        self,
        state: Arc<CaptureState>,
        meeting_id: String,
        path: std::path::PathBuf,
    ) -> Result<(), RemoteError> {
        let mut source = tokio::task::spawn_blocking(move || WavChunks::open(&path)).await??;
        loop {
            let (returned_source, chunk) = tokio::task::spawn_blocking(move || {
                let chunk = source.next_chunk();
                (source, chunk)
            })
            .await?;
            source = returned_source;
            let Some(chunk) = chunk? else { break };
            let form = multipart::Form::new()
                .text("model", self.model.clone())
                .text("response_format", "verbose_json")
                .text("timestamp_granularities[]", "segment")
                .part(
                    "file",
                    multipart::Part::bytes(chunk.bytes)
                        .file_name("meeting.wav")
                        .mime_str("audio/wav")
                        .map_err(|error| RemoteError::Request(error.without_url()))?,
                );
            let response = self
                .remote
                .client
                .post(self.remote.endpoint.clone())
                .multipart(form)
                .send()
                .await
                .map_err(|error| RemoteError::Request(error.without_url()))?;
            if !response.status().is_success() {
                // Provider bodies may echo audio, credentials, or other private input.
                return Err(RemoteError::Status(response.status()));
            }
            let response: VerboseTranscript = response.json().await.map_err(|_| {
                RemoteError::Response("Expected verbose_json with text and timestamped segments. Choose a model that supports this format.")
            })?;
            let segments = response.decode(chunk.start_ms, chunk.duration_ms)?;
            let (state, meeting_id) = (state.clone(), meeting_id.clone());
            tokio::task::spawn_blocking(move || -> Result<(), RemoteError> {
                let path = state.data.segments_jsonl_path(&meeting_id)?;
                let mut sink = SegmentSink::new(&state, &path);
                for segment in segments {
                    sink.push(TranscriptSegment {
                        id: uuid::Uuid::new_v4().to_string(),
                        meeting_id: meeting_id.clone(),
                        speaker_id: None,
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        text: segment.text,
                        confidence: 1.0,
                        partial: false,
                        speaker_confidence: None,
                    });
                }
                sink.finish()?;
                Ok(())
            })
            .await??;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct VerboseTranscript {
    text: String,
    segments: Vec<RemoteSegment>,
}

#[derive(Deserialize)]
struct RemoteSegment {
    start: f64,
    end: f64,
    text: String,
}

impl VerboseTranscript {
    fn decode(self, offset_ms: u32, duration_ms: u32) -> Result<Vec<DecodedSegment>, RemoteError> {
        if self.segments.is_empty() && !self.text.trim().is_empty() {
            return Err(RemoteError::Response("The model returned text without segment timestamps. Choose a model that supports verbose_json segment timestamps."));
        }
        self.segments
            .into_iter()
            .map(|segment| {
                let (start, end) = (segment.start * 1000.0, segment.end * 1000.0);
                if !start.is_finite()
                    || !end.is_finite()
                    || start < 0.0
                    || end < start
                    || end.round() > f64::from(duration_ms)
                {
                    return Err(RemoteError::Response(
                        "Segment timestamps are invalid or outside the uploaded audio.",
                    ));
                }
                let start_ms = offset_ms.checked_add(start.round().to_u32_saturating());
                let end_ms = offset_ms.checked_add(end.round().to_u32_saturating());
                match (start_ms, end_ms) {
                    (Some(start_ms), Some(end_ms)) => Ok(DecodedSegment {
                        start_ms,
                        end_ms,
                        text: segment.text.trim().to_owned(),
                    }),
                    _ => Err(RemoteError::Response(
                        "The recording exceeds the supported timestamp range.",
                    )),
                }
            })
            .collect()
    }
}

struct WavChunks {
    reader: hound::WavReader<BufReader<File>>,
    offset_frames: u64,
}

struct AudioChunk {
    bytes: Vec<u8>,
    start_ms: u32,
    duration_ms: u32,
}

impl WavChunks {
    fn open(path: &Path) -> Result<Self, RemoteError> {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
            || spec.channels == 0
            || spec.sample_rate == 0
        {
            return Err(RemoteError::Response(
                "Expected recorded PCM16 audio with a valid sample rate and channel count.",
            ));
        }
        Ok(Self {
            reader,
            offset_frames: 0,
        })
    }

    fn next_chunk(&mut self) -> Result<Option<AudioChunk>, RemoteError> {
        let spec = self.reader.spec();
        let channels = usize::from(spec.channels);
        let samples_per_chunk = (MAX_UPLOAD_BYTES - WAV_HEADER_BYTES) / 2 / channels * channels;
        let samples = self
            .reader
            .samples::<i16>()
            .take(samples_per_chunk)
            .collect::<Result<Vec<_>, _>>()?;
        if samples.is_empty() {
            return Ok(None);
        }
        if samples.len() % channels != 0 {
            return Err(RemoteError::Response(
                "Recorded audio ends in an incomplete frame.",
            ));
        }
        let frames = u64::try_from(samples.len() / channels)
            .map_err(|_| RemoteError::Response("Audio chunk is too large."))?;
        let start_ms = u32::try_from(self.offset_frames * 1000 / u64::from(spec.sample_rate))
            .map_err(|_| {
                RemoteError::Response("Recording exceeds the supported timestamp range.")
            })?;
        let duration_ms = u32::try_from((frames * 1000).div_ceil(u64::from(spec.sample_rate)))
            .map_err(|_| {
                RemoteError::Response("Audio chunk exceeds the supported timestamp range.")
            })?;
        let mut output =
            std::io::Cursor::new(Vec::with_capacity(WAV_HEADER_BYTES + samples.len() * 2));
        let mut writer = hound::WavWriter::new(&mut output, spec)?;
        for sample in samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        self.offset_frames += frames;
        Ok(Some(AudioChunk {
            bytes: output.into_inner(),
            start_ms,
            duration_ms,
        }))
    }
}
