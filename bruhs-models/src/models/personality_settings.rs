//! User-controlled privacy toggle for per-person observation extraction.
//! Phase 6 has a single global on/off; Phase 6c will add per-meeting
//! and per-person opt-outs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalitySettings {
    #[serde(default = "default_extract_observations")]
    pub extract_observations: bool,
}

fn default_extract_observations() -> bool {
    true
}

impl Default for PersonalitySettings {
    fn default() -> Self {
        Self {
            extract_observations: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_on() {
        assert!(PersonalitySettings::default().extract_observations);
    }
    #[test]
    fn legacy_deserialize_without_field() {
        let s: PersonalitySettings = serde_json::from_str("{}").unwrap();
        assert!(s.extract_observations);
    }
}
