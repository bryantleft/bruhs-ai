//! Microphone capture for voice enrollment — the desktop's stand-in for
//! the web's `capture_voice_sample_wav` (Web Audio). Records the default
//! input device for `duration`, downmixes to mono, and returns 16-bit PCM
//! WAV bytes at the device's sample rate (the capture daemon resamples).

#[cfg(all(test, unix))]
mod cleanup_test;
pub(crate) mod dictionary;
pub(crate) mod preview;
pub(crate) mod sample;
use bruhs_models::num::ToFloatLossy;
use dictionary::VoiceDictionary;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use bruhs_models::logic::wav::encode_wav_16bit_mono;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Destination {
    Off,
    #[default]
    Local,
    Compatible,
    Cloud(bruhs_models::cloud_speech::Provider),
}
impl Destination {
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Local => "local",
            Self::Compatible => "compatible",
            Self::Cloud(provider) => provider.id(),
        }
    }
    pub fn action(self) -> &'static str {
        match self {
            Self::Off => "Dictation off",
            Self::Local => "Dictate locally",
            Self::Compatible => "Dictate with server",
            Self::Cloud(_) => "Dictate with cloud provider",
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ShortcutMode {
    #[default]
    Auto,
    Hold,
    Toggle,
}
impl ShortcutMode {
    pub fn stops_on_release(self, duration: Duration) -> bool {
        self == Self::Hold || (self == Self::Auto && duration >= Duration::from_millis(300))
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Hold => "Hold",
            Self::Toggle => "Toggle",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Auto => "Tap to toggle, hold to talk.",
            Self::Hold => "Record while held.",
            Self::Toggle => "Tap to start, tap to stop.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Cleanup {
    pub enabled: bool,
    pub custom: bool,
    pub instructions: String,
    pub correction_cue: String,
    pub model: Option<String>,
}
impl Default for Cleanup {
    fn default() -> Self {
        Self {
            enabled: false,
            custom: false,
            instructions: String::new(),
            correction_cue: "sorry".into(),
            model: None,
        }
    }
}
pub(crate) type SkipCleanup = tokio::sync::oneshot::Sender<()>;
pub(crate) type CleanupStarted = tokio::sync::oneshot::Sender<SkipCleanup>;

pub(crate) struct Transcript {
    pub text: String,
    pub warning: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct DictationPreferences {
    pub model: bruhs_models::WhisperModel,
    pub language: Option<String>,
    pub microphone: Option<String>,
    pub microphone_order: Vec<Option<String>>,
    pub shortcut_mode: ShortcutMode,
    pub destination: Destination,
    pub cleanup: Cleanup,
    pub remote_model: String,
    pub openrouter_model: String,
    pub silence_filtering: bool,
    pub streaming: bool,
    pub remove_filler_words: bool,
    pub dictionary: VoiceDictionary,
    pub project_dictionaries: std::collections::BTreeMap<String, VoiceDictionary>,
}

impl DictationPreferences {
    /// Older desktop state stored one preferred input. The default device is
    /// always available as a fallback entry, even when its hardware changes.
    pub fn ordered_microphones(&self) -> Vec<Option<String>> {
        let mut order = Vec::new();
        if self.microphone_order.is_empty() {
            if let Some(name) = &self.microphone {
                order.push(Some(name.clone()));
            }
        } else {
            for item in &self.microphone_order {
                if !order.contains(item) {
                    order.push(item.clone());
                }
            }
        }
        if !order.contains(&None) {
            order.push(None);
        }
        order
    }

    pub fn remember_microphones(&mut self, connected: &[String]) -> bool {
        let mut order = self.ordered_microphones();
        let mut insert = order
            .iter()
            .position(Option::is_none)
            .unwrap_or(order.len())
            + 1;
        for name in connected {
            let item = Some(name.clone());
            if !order.contains(&item) {
                order.insert(insert, item);
                insert += 1;
            }
        }
        if order == self.ordered_microphones() {
            return false;
        }
        self.set_microphone_order(order);
        true
    }

    pub fn set_microphone_order(&mut self, order: Vec<Option<String>>) {
        self.microphone = order.first().cloned().flatten();
        self.microphone_order = order;
    }

    pub fn restore_defaults(&mut self) {
        let dictionary = std::mem::take(&mut self.dictionary);
        let project_dictionaries = std::mem::take(&mut self.project_dictionaries);
        *self = Self {
            dictionary,
            project_dictionaries,
            ..Default::default()
        };
    }

    pub fn has_changed_settings(&self) -> bool {
        let mut defaults = self.clone();
        defaults.restore_defaults();
        self != &defaults
    }

    pub async fn spell_transcript(&self, project: Option<&str>, text: &str) -> String {
        let request = bruhs_models::voice_dictionary::DictionaryRequest {
            project: project.map(String::from),
            edit: None,
        };
        let scopes = crate::server_client::ServerClient::workbench_client()
            .voice_dictionaries(&request)
            .await
            .unwrap_or_default();
        let local_project = project.and_then(|path| self.project_dictionaries.get(path));
        VoiceDictionary::apply_scopes(
            [&scopes.environment, &self.dictionary, &scopes.project]
                .into_iter()
                .chain(local_project),
            text,
        )
    }

    pub fn selected_service(&self) -> Result<Option<&str>, &'static str> {
        match self.destination {
            Destination::Off => Err("Dictation is off. Choose a destination in Voice settings."),
            Destination::Local => Ok(None),
            Destination::Cloud(provider) => {
                let model = provider
                    .fixed_model()
                    .unwrap_or(self.openrouter_model.trim());
                if model.is_empty() {
                    Err("Choose an OpenRouter model in Voice settings.")
                } else {
                    Ok(Some(model))
                }
            }
            Destination::Compatible => {
                let model = self.remote_model.trim();
                if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
                    Err("Enter a server transcription model in Voice settings.")
                } else {
                    Ok(Some(model))
                }
            }
        }
    }
    pub async fn transcribe(
        &self,
        wav: Vec<u8>,
        project: Option<&str>,
        text_model: &str,
        draft: bruhs_core::dictation_cleanup::DraftContext,
        cleanup_started: Option<CleanupStarted>,
    ) -> Result<Transcript, String> {
        let model = self.selected_service()?;
        let response = bruhs_core::capture_client::transcribe_selected_dictation(
            bruhs_core::capture_client::endpoint(),
            &self.options(),
            wav,
            model,
            match self.destination {
                Destination::Cloud(provider) => Some(provider),
                _ => None,
            },
        )
        .await
        .map_err(|e| format!("Dictation failed. {e}"))?;
        if response.text.trim().is_empty() {
            return Ok(Transcript {
                text: String::new(),
                warning: None,
            });
        }
        let (text, warning) = if self.cleanup.enabled {
            let model = match self.cleanup.model.as_deref() {
                Some(model) if bruhs_core::text_generators::split_selection(model).is_some() =>
                    match bruhs_core::text_generators::active(model).await { Ok(false) => text_model, _ => model },
                model => model.unwrap_or(text_model),
            };
            let clean = bruhs_core::dictation_cleanup::clean(
                model,
                &response.text,
                &draft,
                self.cleanup
                    .custom
                    .then_some(self.cleanup.instructions.as_str()),
                &self.cleanup.correction_cue,
            );
            let result = if let Some(started) = cleanup_started {
                let (button, skip) = tokio::sync::oneshot::channel();
                let _ = started.send(button);
                tokio::select! {biased; _=skip=>None, result=clean=>Some(result)}
            } else {
                Some(clean.await)
            };
            match result {
                None => (response.text, None),
                Some(Ok(cleaned)) => (cleaned, None),
                Some(Err(_)) => (
                    response.text,
                    Some(
                        "Transcript cleanup was unavailable. The original transcript was retained.",
                    ),
                ),
            }
        } else {
            (response.text, None)
        };
        Ok(Transcript {
            text: self.spell_transcript(project, &text).await,
            warning,
        })
    }

    pub fn options(&self) -> bruhs_models::local_tools::DictationOptions {
        bruhs_models::local_tools::DictationOptions {
            model: self.model,
            silence_filtering: self.silence_filtering,
            remove_filler_words: self.remove_filler_words,
            language: if matches!(
                self.destination,
                Destination::Compatible | Destination::Cloud(_)
            ) || self.model.descriptor().multilingual
            {
                self.language.clone()
            } else {
                Some("en".into())
            },
        }
    }
}

pub fn capture_voice_sample_wav(duration: Duration) -> Result<Vec<u8>, String> {
    record_microphone_sample(&[None], duration)
}

pub fn record_microphone_sample(
    order: &[Option<String>],
    duration: Duration,
) -> Result<Vec<u8>, String> {
    capture_with_stop(order, move |_, _| {
        std::thread::sleep(duration);
        true
    })
}

/// `true` stops and returns the recording; `false` or dropping the sender cancels.
/// The caller runs this on a blocking worker, never the GPUI foreground thread.
pub fn capture_voice_until_stop(
    stop: std::sync::mpsc::Receiver<bool>,
    max_duration: Duration,
    order: &[Option<String>],
    preview: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
) -> Result<Vec<u8>, String> {
    capture_with_stop(order, move |buffer, rate| {
        if let Some(chunks) = preview {
            return preview::record(buffer, rate, stop, max_duration, chunks);
        }
        match stop.recv_timeout(max_duration) {
            Ok(save) => save,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => true,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => false,
        }
    })
}

pub fn microphone_names() -> Result<Vec<String>, String> {
    let mut names = cpal::default_host()
        .input_devices()
        .map_err(|error| error.to_string())?
        .filter_map(|device| device.name().ok())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn preferred_input(
    order: &[Option<String>],
    connected: impl Fn(&str) -> bool,
    has_default: bool,
) -> Option<usize> {
    order
        .iter()
        .position(|name| name.as_deref().map_or(has_default, &connected))
}

fn capture_with_stop(
    order: &[Option<String>],
    finish: impl FnOnce(&Mutex<Vec<f32>>, u32) -> bool,
) -> Result<Vec<u8>, String> {
    let host = cpal::default_host();
    let default = host.default_input_device();
    let device = if order.first().is_none_or(Option::is_none) && default.is_some() {
        default
    } else {
        let mut available: Vec<_> = host
            .input_devices()
            .map_err(|error| error.to_string())?
            .filter_map(|device| Some((device.name().ok()?, device)))
            .collect();
        let index = preferred_input(
            order,
            |name| available.iter().any(|(id, _)| id == name),
            default.is_some(),
        );
        match index.and_then(|i| order.get(i)).and_then(Option::as_ref) {
            Some(name) => available
                .iter()
                .position(|(id, _)| id == name)
                .map(|i| available.swap_remove(i).1),
            None => default,
        }
    }
    .ok_or_else(|| "No connected microphone is available. Check Voice settings.".to_owned())?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("input config: {e}"))?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels().max(1) as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&samples);
    let on_error = |e: cpal::StreamError| tracing::warn!("input stream error: {e}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            &config,
            move |data, _| push_mono(&sink, data.iter().copied(), channels),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            &config,
            move |data, _| {
                push_mono(
                    &sink,
                    data.iter().map(|&s| f32::from(s) / f32::from(i16::MAX)),
                    channels,
                )
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
            &config,
            move |data, _| {
                push_mono(
                    &sink,
                    data.iter().map(|&s| {
                        (f32::from(s) - f32::from(u16::MAX) / 2.0) / (f32::from(u16::MAX) / 2.0)
                    }),
                    channels,
                )
            },
            on_error,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("build input stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("start input stream: {e}"))?;
    let insert = finish(&samples, sample_rate);
    drop(stream);

    if !insert {
        return Err("Recording cancelled".into());
    }

    let pcm = samples.lock().map_err(|_| "sample buffer poisoned")?;
    if pcm.is_empty() {
        return Err("no audio samples captured".to_string());
    }
    let encoded = encode_wav_16bit_mono(&pcm, sample_rate);
    drop(pcm);
    encoded.map_err(|error| error.to_string())
}

/// Average interleaved frames down to one channel.
fn push_mono(sink: &Arc<Mutex<Vec<f32>>>, data: impl Iterator<Item = f32>, channels: usize) {
    let Ok(mut out) = sink.lock() else { return };
    let mut frame = Vec::with_capacity(channels);
    for sample in data {
        frame.push(sample);
        if frame.len() == channels {
            out.push(frame.iter().sum::<f32>() / channels.to_f32_lossy());
            frame.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::push_mono;
    use std::sync::{Arc, Mutex};

    #[test]
    fn saved_microphones_preserve_order_and_fall_back_when_disconnected() {
        use super::{preferred_input, DictationPreferences};
        let mut prefs: DictationPreferences =
            serde_json::from_str(r#"{"microphone":"USB"}"#).unwrap();
        let usb = Some("USB".to_owned());
        let headset = Some("Headset".to_owned());
        assert_eq!(prefs.ordered_microphones(), [usb.clone(), None]);
        assert!(prefs.remember_microphones(&["Headset".into(), "USB".into()]));
        assert_eq!(
            prefs.ordered_microphones(),
            [usb.clone(), None, headset.clone()]
        );
        prefs.set_microphone_order(vec![usb.clone(), headset.clone(), None]);
        let order = prefs.ordered_microphones();
        assert_eq!(preferred_input(&order, |id| id == "Headset", true), Some(1));
        assert_eq!(preferred_input(&order, |_| false, true), Some(2));
        assert_eq!(preferred_input(&order, |_| false, false), None);
        assert!(!prefs.remember_microphones(&["Headset".into()]));
        let restored: DictationPreferences =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(restored.ordered_microphones(), order);
    }

    #[test]
    fn push_mono_averages_interleaved_channels() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        push_mono(&sink, [1.0, 0.0, 0.5, 0.5, 0.2].into_iter(), 2);
        assert_eq!(*sink.lock().unwrap(), vec![0.5, 0.5]);
    }
}
