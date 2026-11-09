//! Whisper model catalog. Each variant has a stable filename (used as
//! the cache key under `models_dir/`) and a download URL pointing at
//! the canonical ggerganov/whisper.cpp Hugging Face mirror.
//!
//! `sha256` and `size_bytes` are the Git LFS object id and size Hugging Face
//! publishes for each file (`lfs.oid` / `lfs.size` in
//! <https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/main>,
//! read 2026-10-03). The capture daemon refuses a download that does not match.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhisperModel {
    TinyEn,
    #[default]
    BaseEn,
    SmallEn,
    LargeV3Turbo,
}

pub struct ModelDescriptor {
    pub filename: &'static str,
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the file at `url`.
    pub sha256: &'static str,
    /// Exact length of the file at `url`.
    pub size_bytes: u64,
    pub size_mb: u32,
    pub label: &'static str,
    pub multilingual: bool,
}

impl WhisperModel {
    pub fn descriptor(&self) -> ModelDescriptor {
        match self {
            Self::TinyEn => ModelDescriptor {
                filename: "ggml-tiny.en.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
                sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
                size_bytes: 77704715,
                size_mb: 75,
                label: "Tiny (English) — fastest, lowest accuracy",
                multilingual: false,
            },
            Self::BaseEn => ModelDescriptor {
                filename: "ggml-base.en.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
                sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
                size_bytes: 147964211,
                size_mb: 150,
                label: "Base (English) — balanced, default",
                multilingual: false,
            },
            Self::SmallEn => ModelDescriptor {
                filename: "ggml-small.en.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
                sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
                size_bytes: 487614201,
                size_mb: 470,
                label: "Small (English) — better accuracy, slower",
                multilingual: false,
            },
            Self::LargeV3Turbo => ModelDescriptor {
                filename: "ggml-large-v3-turbo.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
                sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
                size_bytes: 1624555275,
                size_mb: 1600,
                label: "Large v3 Turbo (multilingual) — best, big download",
                multilingual: true,
            },
        }
    }

    /// Stable identifier used in REST request bodies + localStorage.
    pub fn key(&self) -> &'static str {
        match self {
            Self::TinyEn => "tiny_en",
            Self::BaseEn => "base_en",
            Self::SmallEn => "small_en",
            Self::LargeV3Turbo => "large_v3_turbo",
        }
    }

    pub fn all() -> &'static [WhisperModel] {
        &[
            Self::TinyEn,
            Self::BaseEn,
            Self::SmallEn,
            Self::LargeV3Turbo,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_base_en() {
        assert_eq!(WhisperModel::default(), WhisperModel::BaseEn);
    }

    #[test]
    fn descriptors_have_nonempty_fields() {
        for m in WhisperModel::all() {
            let d = m.descriptor();
            assert!(!d.filename.is_empty());
            assert_eq!(d.sha256.len(), 64);
            assert!(d
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(d.size_bytes > 0);
            assert!(d.url.starts_with("https://"));
            assert!(d.size_mb > 0);
        }
    }

    #[test]
    fn serde_roundtrip_snake_case() {
        let m = WhisperModel::LargeV3Turbo;
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"large_v3_turbo\"");
        let back: WhisperModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in WhisperModel::all() {
            assert!(seen.insert(m.key()), "duplicate key: {}", m.key());
        }
    }
}
