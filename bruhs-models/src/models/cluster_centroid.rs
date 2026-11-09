//! Per-cluster centroid produced by sherpa-onnx sub-speaker
//! clustering, used for cross-meeting identity matching against
//! PersonProfile.voice_embedding. `CentroidsFile` wraps them so the
//! daemon can embed linking settings alongside the vectors.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClusterCentroid {
    /// "them_1", "them_2", ... — matches segment.speaker_id values.
    pub cluster_label: String,
    /// L2-normalized embedding (typically 512 floats).
    pub embedding: Vec<f32>,
    /// Earliest segment.start_ms in this cluster (used for tie-breaking).
    pub first_seen_ms: u32,
}

/// Wrapper written to `centroids.json` by the capture daemon (Phase 3d).
/// Bundles the cluster vectors together with the linking settings that
/// were active when the recording was made, so the server-side hook can
/// honour per-session thresholds without a separate IPC round-trip.
///
/// `#[serde(default)]` on all fields ensures that centroids.json files
/// from Phase 3c (which were a bare `Vec<ClusterCentroid>`) are NOT
/// readable as `CentroidsFile` — the new format is a different shape.
/// Back-compat is intentionally not required because centroids.json is
/// ephemeral daemon state (not precious user data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CentroidsFile {
    /// Cosine similarity threshold used by the server linking hook.
    #[serde(default = "default_linking_threshold")]
    pub linking_threshold: f32,
    /// When false the server never auto-links — every cluster becomes a
    /// fresh "Speaker N" profile. User links manually via the chip menu.
    #[serde(default = "default_linking_auto")]
    pub linking_auto: bool,
    pub centroids: Vec<ClusterCentroid>,
}

fn default_linking_threshold() -> f32 {
    0.75
}

fn default_linking_auto() -> bool {
    true
}

impl Default for CentroidsFile {
    fn default() -> Self {
        Self {
            linking_threshold: 0.75,
            linking_auto: true,
            centroids: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let c = ClusterCentroid {
            cluster_label: "them_1".into(),
            embedding: vec![0.1, 0.2, 0.3],
            first_seen_ms: 1234,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: ClusterCentroid = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn centroids_file_roundtrip() {
        let f = CentroidsFile {
            linking_threshold: 0.85,
            linking_auto: false,
            centroids: vec![ClusterCentroid {
                cluster_label: "them_1".into(),
                embedding: vec![0.5, 0.5],
                first_seen_ms: 0,
            }],
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: CentroidsFile = serde_json::from_str(&json).unwrap();
        assert!((back.linking_threshold - 0.85).abs() < f32::EPSILON);
        assert!(!back.linking_auto);
        assert_eq!(back.centroids.len(), 1);
    }

    #[test]
    fn centroids_file_defaults() {
        let f: CentroidsFile = serde_json::from_str(r#"{"centroids":[]}"#).unwrap();
        assert!((f.linking_threshold - 0.75).abs() < f32::EPSILON);
        assert!(f.linking_auto);
    }
}
