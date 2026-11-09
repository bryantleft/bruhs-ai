use super::{MeetingStoreError, MeetingStoreResult};
use bruhs_models::PersonProfile;

pub(super) fn next_version(current: i64) -> MeetingStoreResult<i64> {
    current
        .checked_add(1)
        .map(|next| next.max(chrono::Utc::now().timestamp_millis()))
        .ok_or_else(|| {
            MeetingStoreError::Conflict("Record version is exhausted; no change was saved.".into())
        })
}

pub(super) fn merged_profile(
    source: &PersonProfile,
    target: &PersonProfile,
) -> MeetingStoreResult<PersonProfile> {
    let version = next_version(target.updated_at.max(source.updated_at))?;
    let mut merged = target.clone();
    merged
        .observations
        .extend(source.observations.iter().cloned());
    if merged.voice_embedding.is_none() {
        merged.voice_embedding.clone_from(&source.voice_embedding);
    }
    let mut names = merged
        .aliases
        .iter()
        .map(|name| name.to_lowercase())
        .collect::<std::collections::HashSet<_>>();
    names.insert(merged.display_name.to_lowercase());
    for alias in std::iter::once(&source.display_name).chain(&source.aliases) {
        if names.insert(alias.to_lowercase()) {
            merged.aliases.push(alias.clone());
        }
    }
    merged.tracked &= source.tracked;
    merged.updated_at = version;
    Ok(merged)
}

pub(super) fn validate_person(person: &PersonProfile) -> MeetingStoreResult<()> {
    if person.id.trim().is_empty()
        || person.display_name.trim().is_empty()
        || person
            .voice_embedding
            .as_ref()
            .is_some_and(|values| values.iter().any(|v| !v.is_finite()))
    {
        return Err(MeetingStoreError::InvalidInput(
            "Contact identity, name and voice values must be valid.".into(),
        ));
    }
    Ok(())
}
pub(super) fn validate_resolution_ids(ids: &[String]) -> MeetingStoreResult<()> {
    let mut unique = std::collections::HashSet::new();
    if ids.len() > 64
        || ids
            .iter()
            .any(|id| id.trim().is_empty() || id.len() > 1024 || !unique.insert(id))
    {
        return Err(MeetingStoreError::InvalidInput(
            "Contact lookup requires at most 64 unique, nonempty contact IDs.".into(),
        ));
    }
    Ok(())
}
