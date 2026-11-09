//! Stage speaker identities without mutating storage. Both stores publish the
//! complete plan atomically, including profiles, voice fills, and the meeting.
use super::{
    person_changes::{next_version, validate_person},
    MeetingStoreError, MeetingStoreResult,
};
use bruhs_models::{CentroidsFile, Meeting, PersonProfile, Speaker};
use std::collections::{BTreeMap, HashSet};

pub(super) struct LinkUpdate {
    pub meeting: Meeting,
    pub people: BTreeMap<String, PersonProfile>,
}

pub(super) fn needs_linking(meeting: &Meeting, file: &CentroidsFile) -> bool {
    file.centroids.iter().any(|centroid| {
        !meeting
            .speakers
            .iter()
            .any(|speaker| speaker.id == centroid.cluster_label)
    })
}

fn profiles<'a>(
    saved: &'a BTreeMap<String, PersonProfile>,
    changed: &'a BTreeMap<String, PersonProfile>,
) -> impl Iterator<Item = &'a PersonProfile> {
    saved
        .values()
        .filter(|person| !changed.contains_key(&person.id))
        .chain(changed.values())
}

pub(super) fn plan(
    mut meeting: Meeting,
    saved: &BTreeMap<String, PersonProfile>,
    file: &CentroidsFile,
) -> MeetingStoreResult<Option<LinkUpdate>> {
    if !needs_linking(&meeting, file) {
        return Ok(None);
    }
    if !file.linking_threshold.is_finite() {
        return Err(MeetingStoreError::InvalidInput(
            "Non-finite speaker threshold".into(),
        ));
    }
    let threshold = f64::from(file.linking_threshold.clamp(0.0, 1.0));
    let mut known: HashSet<_> = meeting
        .speakers
        .iter()
        .map(|speaker| speaker.id.clone())
        .collect();
    let mut changed = BTreeMap::<String, PersonProfile>::new();
    let mut last_number = None;
    for centroid in &file.centroids {
        if known.contains(&centroid.cluster_label) {
            continue;
        }
        if centroid.cluster_label.trim().is_empty()
            || centroid.embedding.iter().any(|value| !value.is_finite())
        {
            return Err(MeetingStoreError::InvalidInput(
                "Invalid speaker centroid".into(),
            ));
        }
        let named = centroid.cluster_label.strip_prefix("name:");
        let matched = if let Some(name) = named {
            if name.trim().is_empty() {
                return Err(MeetingStoreError::InvalidInput("Empty speaker name".into()));
            }
            let normalized = name.to_lowercase();
            let mut matches = profiles(saved, &changed)
                .filter(|person| person.display_name.to_lowercase() == normalized);
            let first = matches.next().cloned();
            if matches.next().is_some() {
                return Err(MeetingStoreError::Conflict(
                    "Speaker name matches multiple contacts; link manually.".into(),
                ));
            }
            first
        } else if file.linking_auto {
            let norm = norm(&centroid.embedding);
            if norm == 0.0 {
                return Err(MeetingStoreError::InvalidInput(
                    "Voice matching requires a nonzero vector.".into(),
                ));
            }
            let mut best: Option<(&PersonProfile, f64)> = None;
            for person in profiles(saved, &changed) {
                let Some(voice) = &person.voice_embedding else {
                    continue;
                };
                if voice.len() != centroid.embedding.len() {
                    continue;
                }
                let denominator = self::norm(voice) * norm;
                if denominator == 0.0 || !denominator.is_finite() {
                    continue;
                }
                let score = voice
                    .iter()
                    .zip(&centroid.embedding)
                    .map(|(a, b)| f64::from(*a) * f64::from(*b))
                    .sum::<f64>()
                    / denominator;
                if score >= threshold
                    && best.is_none_or(|(prior, prior_score)| {
                        score > prior_score || (score == prior_score && person.id < prior.id)
                    })
                {
                    best = Some((person, score));
                }
            }
            best.map(|(person, _)| person.clone())
        } else {
            None
        };
        let person = match matched {
            Some(mut person) => {
                if named.is_some()
                    && person.voice_embedding.is_none()
                    && !centroid.embedding.is_empty()
                {
                    person.voice_embedding = Some(centroid.embedding.clone());
                    person.updated_at = next_version(person.updated_at)?;
                    validate_person(&person)?;
                    changed.insert(person.id.clone(), person.clone());
                }
                person
            }
            None => {
                let name = match named {
                    Some(name) => name.to_owned(),
                    None => {
                        let prior = *last_number.get_or_insert_with(|| {
                            profiles(saved, &changed)
                                .filter_map(|person| {
                                    person
                                        .display_name
                                        .strip_prefix("Speaker ")?
                                        .parse::<u32>()
                                        .ok()
                                })
                                .max()
                                .unwrap_or(0)
                        });
                        let next = prior.checked_add(1).ok_or_else(|| {
                            MeetingStoreError::InvalidInput("Speaker numbering exhausted".into())
                        })?;
                        last_number = Some(next);
                        format!("Speaker {next}")
                    }
                };
                let mut person = PersonProfile::new(name);
                // An OCR name can itself be "Speaker N"; don't reuse that
                // number later in this batch after lazy numbering has started.
                if let Some(prior) = last_number.as_mut() {
                    if let Some(number) = person
                        .display_name
                        .strip_prefix("Speaker ")
                        .and_then(|suffix| suffix.parse::<u32>().ok())
                    {
                        *prior = (*prior).max(number);
                    }
                }
                person.created_at = chrono::Utc::now().timestamp_millis();
                person.updated_at = person.created_at;
                if !centroid.embedding.is_empty() {
                    person.voice_embedding = Some(centroid.embedding.clone());
                }
                validate_person(&person)?;
                changed.insert(person.id.clone(), person.clone());
                person
            }
        };
        meeting.speakers.push(Speaker {
            id: centroid.cluster_label.clone(),
            label: person.display_name,
            person_id: Some(person.id),
            color: None,
        });
        known.insert(centroid.cluster_label.clone());
    }
    meeting.updated_at = next_version(meeting.updated_at)?;
    Ok(Some(LinkUpdate {
        meeting,
        people: changed,
    }))
}

fn norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests;
