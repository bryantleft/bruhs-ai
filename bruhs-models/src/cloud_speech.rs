//! Public voice destinations and non-secret metadata shown by native Settings.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Xai,
    OpenRouter,
}
/// Language inventory advertised by the built-in cloud transcription services.
pub const LANGUAGES: &[&str] = &[
    "ar", "bg", "ca", "cs", "da", "de", "el", "en", "es", "fa", "fi", "fr", "hi", "hr", "hu", "id",
    "it", "ja", "ko", "mk", "ms", "nl", "pl", "pt", "ro", "ru", "sk", "sv", "th", "tr", "uk", "ur",
    "vi", "zh",
];

/// Private per-recording vocabulary. Keep it in the authenticated request body.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SpeechHints {
    pub terms: Vec<String>,
    #[serde(default)]
    pub preserve_correction_words: bool,
}

impl Provider {
    pub fn accepts_vocabulary(self) -> bool {
        matches!(self, Self::OpenAi | Self::Xai)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Xai => "xai",
            Self::OpenRouter => "openrouter",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Xai => "xAI",
            Self::OpenRouter => "OpenRouter",
        }
    }
    pub fn fixed_model(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("gpt-transcribe"),
            Self::Xai => Some("grok-voice-transcribe-2.0"),
            Self::OpenRouter => None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechModel {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyStatus {
    pub configured: bool,
}
/// Contains a private credential. Do not log or persist in desktop state.
#[derive(Serialize, Deserialize)]
pub enum KeyChange {
    Save(String),
    Remove,
}
