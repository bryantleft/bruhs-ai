use super::batch::{MergeCandidate, UniqueRecords};
use super::*;

impl Workspace {
    pub(in crate::views::workspace) fn repair_retired_person_references(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let hub = self.meetings_hub.read(cx);
        if !hub.has_retired_people() {
            return;
        }
        let mut referenced_ids = self.private_referenced_person_ids();
        referenced_ids.extend(
            hub.people
                .iter()
                .filter(|p| hub.person_is_retired(&p.id))
                .map(|p| p.id.clone()),
        );
        self.meetings_hub.update(cx, |hub, _| {
            hub.retain_needed_person_recoveries(&referenced_ids)
        });
        let hub = self.meetings_hub.read(cx);
        let mut ids: Vec<_> = referenced_ids
            .into_iter()
            .filter(|id| hub.person_is_retired(id))
            .collect();
        if ids.is_empty() {
            return;
        }
        ids.sort();
        let people = UniqueRecords::new(
            &hub.people,
            |p| p.id.as_str(),
            "Duplicate contact identities prevent safe merge. Originals were retained.",
        );
        let candidates = ids.into_iter().map(|source_id| {
            let target = hub.canonical_person_id(&source_id).and_then(|id| people.get(id)).and_then(|p| p.cloned().ok_or("The surviving contact is not cached yet. Retry contact sync; local references are retained."));
            MergeCandidate { source_id, target }
        }).collect();
        self.apply_person_merge_candidates(candidates, cx);
    }
}
