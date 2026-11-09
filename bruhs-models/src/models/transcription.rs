//! Final-transcript preferences. Live previews always use on-device Whisper.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionProvider {
    #[default]
    LocalWhisper,
    OpenAiCompatible,
}

impl TranscriptionProvider {
    pub const ALL: [Self; 2] = [Self::LocalWhisper, Self::OpenAiCompatible];

    pub fn key(self) -> &'static str {
        match self {
            Self::LocalWhisper => "local_whisper",
            Self::OpenAiCompatible => "open_ai_compatible",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LocalWhisper => "On-device Whisper",
            Self::OpenAiCompatible => "OpenAI-compatible server",
        }
    }

    pub fn for_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.key() == key)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TranscriptionSettings {
    pub provider: TranscriptionProvider,
    /// Model identifier at the configured native-side endpoint; never a key.
    pub remote_model: String,
}

impl TranscriptionSettings {
    pub fn validate(&self) -> Result<(), TranscriptionSettingsError> {
        if self.provider == TranscriptionProvider::OpenAiCompatible
            && (self.remote_model.trim().is_empty()
                || self.remote_model.chars().any(char::is_control))
        {
            return Err(TranscriptionSettingsError);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "Enter a transcription model supported by your configured server in Settings → Transcription."
)]
pub struct TranscriptionSettingsError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_remote_choice_survives_reload_and_local_needs_no_remote_model() {
        let mut settings: TranscriptionSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.provider, TranscriptionProvider::LocalWhisper);
        assert!(settings.validate().is_ok());
        settings.provider = TranscriptionProvider::OpenAiCompatible;
        assert!(settings.validate().is_err());
        settings.remote_model = "whisper-large-v3-turbo".into();
        let restored: TranscriptionSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert!(restored.validate().is_ok());
    }

    #[test]
    fn unknown_provider_and_control_characters_cannot_become_a_local_fallback() {
        assert!(serde_json::from_str::<TranscriptionSettings>(
            r#"{"provider":"unknown","remote_model":"model"}"#
        )
        .is_err());
        for model in ["", "  ", "model\nother"] {
            assert!(TranscriptionSettings {
                provider: TranscriptionProvider::OpenAiCompatible,
                remote_model: model.into(),
            }
            .validate()
            .is_err());
        }
    }
}
