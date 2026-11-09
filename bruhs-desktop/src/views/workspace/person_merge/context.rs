use super::*;

pub(super) struct PreparedMerge {
    pub archive: PeopleMergeArchive,
    pub canonical: PersonProfile,
    pub relationship: Option<RelationshipProfile>,
    pub recovered_local_name: bool,
}

pub(super) fn prepare_context(
    redirect: &PersonMergeResolution,
    source: Option<&PersonProfile>,
    target: Option<&PersonProfile>,
    source_relationship: Option<&RelationshipProfile>,
    target_relationship: Option<&RelationshipProfile>,
    source_was_pending: bool,
) -> Result<PreparedMerge, &'static str> {
    let source_id = &redirect.source_id;
    let target_id = &redirect.target.id;
    let mut relationship = merge_relationship_context(
        source_id,
        target_id,
        source_relationship,
        target_relationship,
        EpochMs::now().as_millis(),
    )
    .map_err(|error| error.message())?;
    let recovered = source
        .filter(|source| source_was_pending && source.display_name != redirect.target.display_name);
    if let Some(source) = recovered {
        let profile = relationship
            .get_or_insert_with(|| RelationshipProfile::new(target_id, EpochMs::now().as_millis()));
        if !profile.notes.is_empty() {
            profile.notes.push_str("\n\n");
        }
        profile.notes.push_str(&format!(
            "Recovered local contact label ({source_id}): {}",
            source.display_name
        ));
    }
    Ok(PreparedMerge {
        archive: PeopleMergeArchive {
            source: source.cloned(),
            target: target.cloned(),
            source_was_pending,
            source_relationship: source_relationship.cloned(),
            target_relationship: target_relationship.cloned(),
            target_id: target_id.clone(),
        },
        canonical: target
            .filter(|p| p.updated_at > redirect.target.updated_at)
            .unwrap_or(&redirect.target)
            .clone(),
        relationship,
        recovered_local_name: recovered.is_some(),
    })
}
