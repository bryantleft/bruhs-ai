//! Public voice destinations and non-secret metadata shown by native Settings.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Xai,
    OpenRouter,
}
impl Provider {
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
