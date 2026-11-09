use super::batch::{MergeCandidate, UniqueRecords};
use super::*;
use crate::views::meetings_hub::MeetingsHub;
use std::collections::HashMap;

const INVALID: &str =
    "Contact merge reply is stale or invalid. Local records are retained; retry contact sync.";
const CYCLE: &str = "Contact redirects form a cycle. Originals are retained.";
const MISSING: &str =
    "The surviving contact is not cached yet. Retry contact sync; local references are retained.";

/// Resolve the whole read-only reply before preflighting private context. Memoized,
/// iterative walks also handle chains split across concurrent HTTP responses.
fn candidates(incoming: &[PersonMergeResolution], hub: &MeetingsHub) -> Vec<MergeCandidate> {
    let mut edges = HashMap::with_capacity(incoming.len());
    let mut profiles = HashMap::<&str, &PersonProfile>::new();
    for redirect in incoming {
        let target = &redirect.target;
        let valid = !redirect.source_id.trim().is_empty()
            && !target.id.trim().is_empty()
            && !target.display_name.trim().is_empty()
            && redirect.source_id != target.id;
        edges
            .entry(redirect.source_id.as_str())
            .and_modify(|edge| *edge = None)
            .or_insert(valid.then_some(target.id.as_str()));
        if valid {
            profiles
                .entry(target.id.as_str())
                .and_modify(|profile| {
                    if target.updated_at > profile.updated_at {
                        *profile = target;
                    }
                })
                .or_insert(target);
        }
    }
    let people = UniqueRecords::new(
        &hub.people,
        |p| p.id.as_str(),
        "Duplicate contact identities prevent safe merge. Originals were retained.",
    );
    let mut memo: HashMap<&str, Result<&PersonProfile, &'static str>> = HashMap::new();
    let mut sources: Vec<_> = edges
        .keys()
        .copied()
        .filter(|id| !hub.person_is_retired(id))
        .collect();
    sources.sort_unstable();
    sources
        .into_iter()
        .map(|source| {
            let mut current = source;
            let mut path = Vec::new();
            let mut seen = HashSet::new();
            let target = loop {
                if let Some(result) = memo.get(current) {
                    break *result;
                }
                if current.trim().is_empty() {
                    break Err(INVALID);
                }
                if !seen.insert(current) {
                    break Err(CYCLE);
                }
                path.push(current);
                // Saved originals are authoritative for already-retired identities.
                if let Some(next) = hub.saved_person_merge_target(current) {
                    current = next;
                    continue;
                }
                match edges.get(current) {
                    Some(Some(next)) => current = next,
                    Some(None) => break Err(INVALID),
                    None => {
                        break people.get(current).and_then(|cached| {
                            profiles.get(current).copied().or(cached).ok_or(MISSING)
                        });
                    }
                }
            };
            for id in path {
                memo.insert(id, target);
            }
            MergeCandidate {
                source_id: source.into(),
                target: target.cloned(),
            }
        })
        .collect()
}

impl Workspace {
    pub(in crate::views::workspace) fn reconcile_person_merges(
        &mut self,
        incoming: &[PersonMergeResolution],
        cx: &mut Context<'_, Self>,
    ) {
        let candidates = candidates(incoming, self.meetings_hub.read(cx));
        self.apply_person_merge_candidates(candidates, cx);
        self.repair_retired_person_references(cx);
    }
}
