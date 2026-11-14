//! Owned transcript rows with validation invalidated by every mutable borrow.
//! Segments own plain data; immutable access cannot change their validation fields.
use bruhs_models::{TranscriptSegment, TranscriptValidationError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

type Records = BTreeMap<String, Vec<TranscriptSegment>>;
type Validation = Option<(String, Result<(), TranscriptValidationError>)>;

#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TranscriptMap {
    records: Records,
    // One result only; unrelated meeting reads may replace it. Never persisted.
    #[serde(skip)]
    validation: Mutex<Validation>,
}

impl TranscriptMap {
    #[cfg(any(unix, test))]
    pub(crate) fn validate(&self, meeting_id: &str) -> Result<(), TranscriptValidationError> {
        let records = self
            .records
            .get(meeting_id)
            .ok_or(TranscriptValidationError::InvalidSegments)?;
        let mut validation = self
            .validation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((id, result)) = validation.as_ref() {
            if id == meeting_id {
                return *result;
            }
        }
        let result = bruhs_models::validate_transcript_records(meeting_id, records);
        *validation = Some((meeting_id.to_owned(), result));
        result
    }
}

impl Deref for TranscriptMap {
    type Target = Records;
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

impl DerefMut for TranscriptMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // All mutable map/Vec/segment access passes through here. Outstanding
        // mutable borrows prevent validation until they end, even after panic.
        *self
            .validation
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        &mut self.records
    }
}

impl Clone for TranscriptMap {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            validation: Mutex::default(),
        }
    }
}

impl std::fmt::Debug for TranscriptMap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.records.fmt(formatter)
    }
}

impl FromIterator<(String, Vec<TranscriptSegment>)> for TranscriptMap {
    fn from_iter<T: IntoIterator<Item = (String, Vec<TranscriptSegment>)>>(iter: T) -> Self {
        Self {
            records: iter.into_iter().collect(),
            validation: Mutex::default(),
        }
    }
}

impl<'a> IntoIterator for &'a TranscriptMap {
    type Item = (&'a String, &'a Vec<TranscriptSegment>);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Vec<TranscriptSegment>>;
    fn into_iter(self) -> Self::IntoIter {
        self.records.iter()
    }
}

impl<'a> IntoIterator for &'a mut TranscriptMap {
    type Item = (&'a String, &'a mut Vec<TranscriptSegment>);
    type IntoIter = std::collections::btree_map::IterMut<'a, String, Vec<TranscriptSegment>>;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            meeting_id: "m".into(),
            speaker_id: None,
            start_ms: 0,
            end_ms: 1,
            text: "Saved ΟΣ".into(),
            confidence: 0.9,
            partial: false,
            speaker_confidence: None,
        }
    }

    #[test]
    fn mutable_access_invalidates_valid_and_invalid_results_beyond_the_page() {
        let mut map: TranscriptMap = [(
            "m".into(),
            (0..123).map(|i| segment(&format!("s{i}"))).collect(),
        )]
        .into_iter()
        .collect();
        assert_eq!(map.validate("m"), Ok(()));
        assert_eq!(map.validate("m"), Ok(()));
        assert!(map.validation.lock().expect("validation").is_some());
        map.get_mut("m").expect("transcript")[122].id = "s0".into();
        assert!(map.validation.lock().expect("validation").is_none());
        assert_eq!(
            map.validate("m"),
            Err(TranscriptValidationError::InvalidSegments)
        );
        for (_, rows) in &mut map {
            rows[122].id = "fixed".into();
        }
        assert!(map.validation.lock().expect("validation").is_none());
        assert_eq!(map.validate("m"), Ok(()));
        map.entry("m".into()).or_default()[122].confidence = f32::NAN;
        assert_eq!(
            map.validate("m"),
            Err(TranscriptValidationError::InvalidSegments)
        );
    }

    #[test]
    fn identity_changes_replacements_and_serialization_never_reuse_a_foreign_result() {
        let mut map: TranscriptMap = [("m".into(), vec![segment("s")])].into_iter().collect();
        assert_eq!(map.validate("m"), Ok(()));
        let rows = map.remove("m").expect("transcript");
        map.insert("foreign".into(), rows);
        assert_eq!(
            map.validate("foreign"),
            Err(TranscriptValidationError::InvalidSegments)
        );
        let mut cloned = map.clone();
        assert!(cloned.validation.lock().expect("validation").is_none());
        cloned.get_mut("foreign").expect("transcript")[0].meeting_id = "foreign".into();
        assert_eq!(cloned.validate("foreign"), Ok(()));
        let json = serde_json::to_string(&cloned).expect("serialize");
        assert_eq!(
            json,
            serde_json::to_string(&cloned.records).expect("plain records")
        );
        let restored: TranscriptMap = serde_json::from_str(&json).expect("restore");
        assert!(restored.validation.lock().expect("validation").is_none());
        assert_eq!(restored.validate("foreign"), Ok(()));
        assert_eq!(
            map.validate("foreign"),
            Err(TranscriptValidationError::InvalidSegments)
        );
    }

    #[test]
    fn validation_results_remain_bound_to_the_requested_meeting() {
        let map: TranscriptMap = [
            ("m".into(), vec![segment("s")]),
            ("other".into(), vec![segment("foreign")]),
        ]
        .into_iter()
        .collect();
        assert_eq!(map.validate("m"), Ok(()));
        assert_eq!(
            map.validate("other"),
            Err(TranscriptValidationError::InvalidSegments)
        );
        assert_eq!(map.validate("m"), Ok(()));
        assert_eq!(
            map.validation
                .lock()
                .expect("validation")
                .as_ref()
                .expect("cached result")
                .0,
            "m"
        );
    }
}
