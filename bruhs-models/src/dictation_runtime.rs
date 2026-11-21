//! Host-wide hardware and memory preferences for local composer dictation.
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hardware {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unload {
    Never,
    #[default]
    Immediately,
    Min2,
    Min5,
    Min10,
    Min15,
    Hour1,
}
impl Unload {
    pub const CHOICES: [(Self, &'static str); 7] = [
        (Self::Never, "Never"),
        (Self::Immediately, "Immediately"),
        (Self::Min2, "After 2 minutes"),
        (Self::Min5, "After 5 minutes"),
        (Self::Min10, "After 10 minutes"),
        (Self::Min15, "After 15 minutes"),
        (Self::Hour1, "After 1 hour"),
    ];
    pub fn seconds(self) -> Option<u64> {
        match self {
            Self::Never => None,
            Self::Immediately => Some(0),
            Self::Min2 => Some(120),
            Self::Min5 => Some(300),
            Self::Min10 => Some(600),
            Self::Min15 => Some(900),
            Self::Hour1 => Some(3600),
        }
    }
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hardware: Hardware,
    pub unload: Unload,
}
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Hardware(Hardware),
    Unload(Unload),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub settings: Settings,
    pub gpu: Option<String>,
    pub loaded: Option<crate::WhisperModel>,
    #[serde(default)]
    pub compatible_origin: Option<String>,
}

/// Files managed on the capture host, shared with meeting transcription.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelEntry {
    pub model: crate::WhisperModel,
    pub installed: bool,
    pub downloaded_bytes: Option<u64>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAction {
    Download(crate::WhisperModel),
    Import {
        model: crate::WhisperModel,
        path: String,
    },
    Cancel(crate::WhisperModel),
    Remove(crate::WhisperModel),
}
