//! Diarization controls: a toggle for sherpa-onnx sub-speaker clustering
//! on the system-audio channel, cross-meeting linking (auto flag +
//! threshold), and max_speakers (2–20, default 6).
//! Phase Vision adds video_ocr_enabled (default false, opt-in).

use serde::{Deserialize, Serialize};

// f32 field (cross_meeting_linking_threshold) means we cannot derive Eq.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiarizeSettings {
    #[serde(default = "default_subspeaker_enabled")]
    pub subspeaker_enabled: bool,
    /// When true (default), Phase 3c auto-links clusters above
    /// `cross_meeting_linking_threshold`. When false, every cluster
    /// becomes a fresh "Speaker N" profile and the user links
    /// manually via the chip menu.
    #[serde(default = "default_cross_meeting_linking_auto")]
    pub cross_meeting_linking_auto: bool,
    /// Cosine similarity threshold for cross-meeting linking.
    /// Range 0.5–0.95; default 0.75. Tighter (higher) reduces
    /// false matches; looser improves recall.
    #[serde(default = "default_cross_meeting_linking_threshold")]
    pub cross_meeting_linking_threshold: f32,
    /// Maximum number of distinct sub-speakers the clustering step can
    /// produce. Range 2–20; default 6. Higher values give better
    /// resolution on multi-person calls; lower values are faster and
    /// produce fewer false splits on quiet meetings.
    #[serde(default = "default_max_speakers")]
    pub max_speakers: u32,
    /// Phase Vision: when true, captures video frames at 2fps from the
    /// first display during recording and runs Apple's Vision framework
    /// OCR locally to read unverified name hints from the meeting UI. These
    /// hints do not identify active speakers or auto-label transcripts. Default false
    /// (opt-in — privacy: video frames are processed in-memory and never
    /// leave the Mac, but the feature requires sustained Screen Recording
    /// access; runtime overhead depends on the device and screen contents).
    #[serde(default)]
    pub video_ocr_enabled: bool,
}

fn default_subspeaker_enabled() -> bool {
    true
}

fn default_cross_meeting_linking_auto() -> bool {
    true
}

fn default_cross_meeting_linking_threshold() -> f32 {
    0.75
}

fn default_max_speakers() -> u32 {
    6
}

impl Default for DiarizeSettings {
    fn default() -> Self {
        Self {
            subspeaker_enabled: true,
            cross_meeting_linking_auto: true,
            cross_meeting_linking_threshold: 0.75,
            max_speakers: 6,
            video_ocr_enabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_on() {
        assert!(DiarizeSettings::default().subspeaker_enabled);
    }

    #[test]
    fn default_auto_link_is_on() {
        assert!(DiarizeSettings::default().cross_meeting_linking_auto);
    }

    #[test]
    fn default_threshold_is_0_75() {
        assert!(
            (DiarizeSettings::default().cross_meeting_linking_threshold - 0.75).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn legacy_deserialize() {
        // JSON with only subspeaker_enabled — back-compat: new fields default.
        let s: DiarizeSettings = serde_json::from_str(r#"{"subspeaker_enabled": true}"#).unwrap();
        assert!(s.subspeaker_enabled);
        assert!(s.cross_meeting_linking_auto);
        assert!((s.cross_meeting_linking_threshold - 0.75).abs() < f32::EPSILON);
        assert_eq!(
            s.max_speakers, 6,
            "max_speakers must default to 6 for legacy JSON"
        );
    }

    #[test]
    fn empty_json_applies_all_defaults() {
        let s: DiarizeSettings = serde_json::from_str("{}").unwrap();
        assert!(s.subspeaker_enabled);
        assert!(s.cross_meeting_linking_auto);
        assert!((s.cross_meeting_linking_threshold - 0.75).abs() < f32::EPSILON);
        assert_eq!(s.max_speakers, 6);
    }

    #[test]
    fn explicit_fields_roundtrip() {
        let orig = DiarizeSettings {
            subspeaker_enabled: false,
            cross_meeting_linking_auto: false,
            cross_meeting_linking_threshold: 0.85,
            max_speakers: 12,
            video_ocr_enabled: true,
        };
        let json = serde_json::to_string(&orig).unwrap();
        let back: DiarizeSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.subspeaker_enabled, orig.subspeaker_enabled);
        assert_eq!(
            back.cross_meeting_linking_auto,
            orig.cross_meeting_linking_auto
        );
        assert!(
            (back.cross_meeting_linking_threshold - orig.cross_meeting_linking_threshold).abs()
                < f32::EPSILON
        );
        assert_eq!(back.max_speakers, 12);
    }

    #[test]
    fn max_speakers_defaults_to_6() {
        let s = DiarizeSettings::default();
        assert_eq!(s.max_speakers, 6);
    }

    #[test]
    fn max_speakers_roundtrip_min_max() {
        for &val in &[2u32, 10, 20] {
            let s = DiarizeSettings {
                max_speakers: val,
                ..DiarizeSettings::default()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: DiarizeSettings = serde_json::from_str(&json).unwrap();
            assert_eq!(back.max_speakers, val);
        }
    }

    #[test]
    fn video_ocr_enabled_defaults_to_false() {
        assert!(!DiarizeSettings::default().video_ocr_enabled);
    }

    #[test]
    fn video_ocr_enabled_legacy_json_defaults_false() {
        // JSON without the field (written before Phase Vision) must default to false.
        let s: DiarizeSettings = serde_json::from_str(r#"{"subspeaker_enabled": true}"#).unwrap();
        assert!(
            !s.video_ocr_enabled,
            "video_ocr_enabled must default to false for legacy JSON"
        );
    }

    #[test]
    fn video_ocr_enabled_roundtrip() {
        let s = DiarizeSettings {
            video_ocr_enabled: true,
            ..DiarizeSettings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: DiarizeSettings = serde_json::from_str(&json).unwrap();
        assert!(back.video_ocr_enabled);
    }
}
