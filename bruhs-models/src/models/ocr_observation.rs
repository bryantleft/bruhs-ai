//! OCR observation produced by the Vision-framework OCR pipeline (Phase Vision).
//! Defined in bruhs-models so it can be shared between bruhs-capture (writes
//! the JSON) and bruhs-core / bruhs-server (reads it via the daemon API).

use serde::{Deserialize, Serialize};

/// A single OCR observation extracted from a conferencing-app video frame.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrObservation {
    /// The recognized text string (filtered to candidate participant names).
    pub text: String,
    /// Normalized bounding box. Vision convention: origin = bottom-left,
    /// all values 0..1 relative to the image.
    pub bbox: OcrBoundingBox,
    /// Per-candidate confidence from VNRecognizedText (0..1).
    pub confidence: f32,
    /// Milliseconds since recording start when this frame was captured.
    pub timestamp_ms: u32,
}

/// Normalized bounding box from Vision framework (origin = bottom-left).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrBoundingBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let obs = OcrObservation {
            text: "Alice".to_string(),
            bbox: OcrBoundingBox {
                x: 0.1,
                y: 0.2,
                w: 0.3,
                h: 0.04,
            },
            confidence: 0.95,
            timestamp_ms: 5000,
        };
        let json = serde_json::to_string(&obs).unwrap();
        let back: OcrObservation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, obs);
    }

    #[test]
    fn default_confidence_range() {
        let obs = OcrObservation {
            text: "Bob".to_string(),
            bbox: OcrBoundingBox {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            confidence: 0.8,
            timestamp_ms: 0,
        };
        assert!(obs.confidence >= 0.0 && obs.confidence <= 1.0);
    }
}
