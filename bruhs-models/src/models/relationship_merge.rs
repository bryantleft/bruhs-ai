//! Lossless private-context transfer; callers atomically archive originals and remove the source.

use std::collections::{HashMap, HashSet};

use super::RelationshipProfile;

/// Why two relationship profiles cannot be merged safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum RelationshipMergeError {
    IdentityMismatch,
    MissingOrDuplicateItemIds,
    /// Both sides hold the same item id with different contents.
    ConflictingItem,
}

impl RelationshipMergeError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::IdentityMismatch => {
                "Merge relationship identities do not match. No records were changed."
            }
            Self::MissingOrDuplicateItemIds => {
                "Duplicate or missing relationship item IDs prevent safe merge."
            }
            Self::ConflictingItem => {
                "A relationship item ID has conflicting contents. Resolve it before merging."
            }
        }
    }
}

pub fn merge_relationship_context(
    source_id: &str,
    target_id: &str,
    source: Option<&RelationshipProfile>,
    target: Option<&RelationshipProfile>,
    now_ms: i64,
) -> Result<Option<RelationshipProfile>, RelationshipMergeError> {
    if source_id.is_empty()
        || target_id.is_empty()
        || source_id == target_id
        || source.is_some_and(|profile| profile.person_id != source_id)
        || target.is_some_and(|profile| profile.person_id != target_id)
    {
        return Err(RelationshipMergeError::IdentityMismatch);
    }
    let Some(source) = source else {
        if let Some(target) = target {
            let mut known = HashSet::new();
            if target
                .items
                .iter()
                .any(|item| item.id.is_empty() || !known.insert(&item.id))
            {
                return Err(RelationshipMergeError::MissingOrDuplicateItemIds);
            }
        }
        return Ok(target.cloned());
    };
    let mut merged = target
        .cloned()
        .unwrap_or_else(|| RelationshipProfile::new(target_id, now_ms));
    merge_items(source, &mut merged)?;
    let mut preserved = Vec::new();
    for (label, from, into) in [
        ("Company", &source.company, &mut merged.company),
        ("Role", &source.role, &mut merged.role),
        ("Email", &source.email, &mut merged.email),
        ("Phone", &source.phone, &mut merged.phone),
        ("Location", &source.location, &mut merged.location),
    ] {
        if into.trim().is_empty() {
            *into = from.clone();
        } else if !from.trim().is_empty() && into != from {
            preserved.push(format!("{label}: {from}"));
        }
    }
    if !source.notes.is_empty() && source.notes != merged.notes {
        preserved.push(source.notes.clone());
    }
    if !preserved.is_empty() {
        if !merged.notes.is_empty() {
            merged.notes.push_str("\n\n");
        }
        merged.notes.push_str(&format!(
            "Merged relationship ({source_id})\n{}",
            preserved.join("\n\n")
        ));
    }
    merged.last_contacted_at_ms = merged.last_contacted_at_ms.max(source.last_contacted_at_ms);
    merged.updated_at_ms = now_ms.max(source.updated_at_ms).max(merged.updated_at_ms);
    Ok(Some(merged))
}

fn merge_items(
    source: &RelationshipProfile,
    target: &mut RelationshipProfile,
) -> Result<(), RelationshipMergeError> {
    let mut known = HashMap::new();
    for item in &target.items {
        if item.id.is_empty() || known.insert(item.id.as_str(), item).is_some() {
            return Err(RelationshipMergeError::MissingOrDuplicateItemIds);
        }
    }
    let mut source_ids = HashSet::new();
    let mut additions = Vec::new();
    for item in &source.items {
        if item.id.is_empty() || !source_ids.insert(item.id.as_str()) {
            return Err(RelationshipMergeError::MissingOrDuplicateItemIds);
        }
        match known.get(item.id.as_str()) {
            Some(existing) if **existing != *item => {
                return Err(RelationshipMergeError::ConflictingItem)
            }
            Some(_) => {}
            None => additions.push(item.clone()),
        }
    }
    target.items.extend(additions);
    Ok(())
}

#[cfg(test)]
mod tests;
