//! OS-held credentials and speech requests for the built-in voice destinations.
use super::*;
use axum::extract::{FromRequest, Multipart, Path, Request};
use bruhs_models::cloud_speech::{KeyChange, KeyStatus, Provider, SpeechHints, SpeechModel};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

type Error = (StatusCode, String);
const CATALOG: &str =
    "https://openrouter.ai/api/v1/models?output_modalities=transcription&zdr=true";
static MODELS: Mutex<Option<(Instant, Vec<SpeechModel>)>> = Mutex::const_new(None);
fn unavailable() -> Error {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "The capture host’s credential store is unavailable.".into(),
    )
}
fn entry(provider: Provider) -> Result<keyring::Entry, Error> {
    keyring::Entry::new("bruhs-ai-dictation", provider.id()).map_err(|_| unavailable())
}
fn read_key(entry: &keyring::Entry) -> Result<Option<String>, Error> {
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}
pub async fn key_status(Path(provider): Path<Provider>) -> Result<Json<KeyStatus>, Error> {
    tokio::task::spawn_blocking(move || {
        Ok(Json(KeyStatus {
            configured: read_key(&entry(provider)?)?.is_some(),
        }))
    })
    .await
    .map_err(internal)?
}
fn change_key(entry: &keyring::Entry, change: KeyChange) -> Result<KeyStatus, Error> {
    let configured = match change {
        KeyChange::Save(key) => {
            let key = key.trim();
            if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
                return Err((StatusCode::BAD_REQUEST, "Enter a valid API key.".into()));
            }
            entry.set_password(key).map_err(|_| unavailable())?;
            true
        }
        KeyChange::Remove => {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err(unavailable()),
            }
            false
        }
    };
    Ok(KeyStatus { configured })
}
pub async fn set_key(
    Path(provider): Path<Provider>,
    Json(change): Json<KeyChange>,
) -> Result<Json<KeyStatus>, Error> {
    tokio::task::spawn_blocking(move || change_key(&entry(provider)?, change).map(Json))
        .await
        .map_err(internal)?
}
async fn models() -> Result<Vec<SpeechModel>, Error> {
    let mut cache = MODELS.lock().await;
    if let Some((when, list)) = cache.as_ref() {
        if when.elapsed() < Duration::from_secs(300) {
            return Ok(list.clone());
        }
    }
    let error = || {
        (
            StatusCode::BAD_GATEWAY,
            "Could not read the OpenRouter transcription model list.".into(),
        )
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| error())?;
    let mut response = client
        .get(CATALOG)
        .send()
        .await
        .map_err(|_| error())?
        .error_for_status()
        .map_err(|_| error())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| error())? {
        if bytes.len() + chunk.len() > 2_000_000 {
            return Err(error());
        }
        bytes.extend_from_slice(&chunk);
    }
    #[derive(serde::Deserialize)]
    struct Listing {
        data: Vec<Row>,
    }
    #[derive(serde::Deserialize)]
    struct Row {
        id: String,
        name: Option<String>,
        architecture: Architecture,
    }
    #[derive(serde::Deserialize)]
    struct Architecture {
        output_modalities: Vec<String>,
    }
    let listing: Listing = serde_json::from_slice(&bytes).map_err(|_| error())?;
    let mut list: Vec<_> = listing
        .data
        .into_iter()
        .filter(|r| {
            r.architecture
                .output_modalities
                .iter()
                .any(|m| m == "transcription")
                && !r.id.is_empty()
                && r.id.len() <= 256
                && !r.id.chars().any(char::is_control)
        })
        .map(|r| SpeechModel {
            name: r
                .name
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| r.id.clone()),
            id: r.id,
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    *cache = Some((Instant::now(), list.clone()));
    drop(cache);
    Ok(list)
}
pub async fn catalog() -> Result<Json<Vec<SpeechModel>>, Error> {
    models().await.map(Json)
}
pub struct CloudAudio {
    audio: Bytes,
    hints: SpeechHints,
}

impl<S: Send + Sync> FromRequest<S> for CloudAudio {
    type Rejection = Error;

    async fn from_request(request: Request, state: &S) -> Result<Self, Error> {
        let multipart = request
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("multipart/form-data"));
        if !multipart {
            let audio = Bytes::from_request(request, state)
                .await
                .map_err(|error| (error.status(), error.body_text()))?;
            return Ok(Self {
                audio,
                hints: SpeechHints::default(),
            });
        }
        let invalid = || {
            (
                StatusCode::BAD_REQUEST,
                "Invalid dictation audio or vocabulary.".into(),
            )
        };
        let mut form = Multipart::from_request(request, state)
            .await
            .map_err(|_| invalid())?;
        let mut hints = None;
        let mut audio = None;
        while let Some(mut field) = form
            .next_field()
            .await
            .map_err(|e| (e.status(), e.body_text()))?
        {
            match field.name() {
                Some("audio") if audio.is_none() => {
                    audio = Some(
                        field
                            .bytes()
                            .await
                            .map_err(|e| (e.status(), e.body_text()))?,
                    );
                }
                Some("hints") if hints.is_none() => {
                    let mut bytes = Vec::new();
                    while let Some(chunk) = field
                        .chunk()
                        .await
                        .map_err(|e| (e.status(), e.body_text()))?
                    {
                        // Bound metadata independently of the larger audio upload.
                        if bytes.len() + chunk.len() > 128_000 {
                            return Err(invalid());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    let parsed: SpeechHints =
                        serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                    if parsed.terms.len() > 100
                        || parsed.terms.iter().any(|term| {
                            term.trim().is_empty()
                                || term.chars().count() > 50
                                || term.chars().any(char::is_control)
                        })
                    {
                        return Err(invalid());
                    }
                    hints = Some(parsed);
                }
                _ => return Err(invalid()),
            }
        }
        Ok(Self {
            audio: audio.ok_or_else(invalid)?,
            hints: hints.unwrap_or_default(),
        })
    }
}

pub async fn transcribe(
    State(state): State<Arc<crate::state::CaptureState>>,
    Path(provider): Path<Provider>,
    Query(mut options): Query<DictationOptions>,
    Query(selection): Query<CompatibleModel>,
    body: CloudAudio,
) -> Result<Json<DictationTranscript>, Error> {
    let model = match provider.fixed_model() {
        Some(m) => m.to_string(),
        None => {
            if !models()
                .await?
                .iter()
                .any(|m| m.id == selection.remote_model)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Choose an available OpenRouter transcription model in Voice settings.".into(),
                ));
            }
            selection.remote_model
        }
    };
    let key = tokio::task::spawn_blocking(move || read_key(&entry(provider)?))
        .await
        .map_err(internal)??
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("Add your {} API key in Voice settings.", provider.label()),
            )
        })?;
    let job =
        crate::transcribe::remote::RemoteJob::cloud(provider, model.clone(), &key, body.hints)
            .map_err(|_| unavailable())?;
    if provider == Provider::OpenAi {
        options.silence_filtering = false;
    }
    run(state, options, body.audio, Some((model, job))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cloud_audio_accepts_legacy_wav_and_scoped_hints_without_query_parameters() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/transcribe", listener.local_addr().unwrap());
        let router = axum::Router::new().route(
            "/transcribe",
            axum::routing::post(|uri: axum::http::Uri, body: CloudAudio| async move {
                assert!(uri.query().is_none());
                assert_eq!(&*body.audio, b"WAV");
                Json(body.hints)
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::new();
        let plain = client
            .post(&url)
            .header("Content-Type", "audio/wav")
            .body("WAV")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<SpeechHints>()
            .await
            .unwrap();
        assert!(plain.terms.is_empty());
        let request = |terms| {
            client.post(&url).multipart(
                reqwest::multipart::Form::new()
                    .part("audio", reqwest::multipart::Part::bytes(b"WAV".to_vec()))
                    .text(
                        "hints",
                        serde_json::json!({"terms": terms, "preserve_correction_words": true})
                            .to_string(),
                    ),
            )
        };
        let hints = request(vec!["Montréal"])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<SpeechHints>()
            .await
            .unwrap();
        assert_eq!(hints.terms, ["Montréal"]);
        assert!(hints.preserve_correction_words);
        for invalid in [vec!["term"; 101], vec!["two\nlines"]] {
            assert_eq!(
                request(invalid).send().await.unwrap().status(),
                StatusCode::BAD_REQUEST
            );
        }
        server.abort();
    }

    #[test]
    fn private_key_round_trip_never_returns_the_secret_to_settings() {
        let key =
            keyring::Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        assert!(read_key(&key).unwrap().is_none());
        assert!(change_key(&key, KeyChange::Save("two lines\nprivate".into())).is_err());
        let saved = change_key(&key, KeyChange::Save("synthetic-api-key".into())).unwrap();
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::json!({"configured":true})
        );
        assert_eq!(
            read_key(&key).unwrap().as_deref(),
            Some("synthetic-api-key")
        );
        assert!(!change_key(&key, KeyChange::Remove).unwrap().configured);
        assert!(read_key(&key).unwrap().is_none());
    }
}
