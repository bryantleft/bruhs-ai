//! Narrow SSE invalidations: coalesce by identity and bound fetch/write concurrency.
use super::contact_reconcile::timed;
use super::*;

impl WorkflowExecutor {
    fn has_person_version(self, id: &str, version: i64) -> bool {
        self.people.0.with_untracked(|people| {
            people
                .iter()
                .any(|person| person.id == id && person.updated_at >= version)
        })
    }

    pub fn person_changed(self, id: String, version: i64) {
        if !self.people_hydrated.get_untracked() {
            self.people_refresh_deferred.set(true);
            return;
        }
        if self.has_person_version(&id, version) {
            return;
        }
        self.person_refreshes.update(|pending| {
            pending
                .entry(id)
                .and_modify(|current| *current = (*current).max(version))
                .or_insert(version);
        });
        if self.person_refresh_running.get_untracked() {
            return;
        }
        self.person_refresh_running.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let mut batch = Vec::with_capacity(4);
                self.person_refreshes.update(|pending| {
                    for _ in 0..4 {
                        let Some(item) = pending.pop_first() else {
                            break;
                        };
                        batch.push(item);
                    }
                });
                if batch.is_empty() {
                    break;
                }
                futures_util::future::join_all(batch.into_iter().map(|(id, version)| async move {
                    if self.has_person_version(&id, version) { return; }
                    match timed(crate::api::get_person(&id)).await {
                        Ok(Some(person)) if person.updated_at >= version => self.cache_people(vec![person]).await,
                        Ok(None) => self.remote_person_merged(id),
                        _ => self.report_people_cache("A changed contact could not refresh. Your local copy is retained; retry loading contacts in People.".into()),
                    }
                })).await;
            }
            self.person_refresh_running.set(false);
        });
    }

    pub fn remote_person_merged(self, source_id: String) {
        if self.merge_record.with_untracked(|record| {
            record
                .as_ref()
                .is_some_and(|record| record.source.id == source_id)
        }) {
            // The initiating client already owns a durable journal and its HTTP reply.
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = self.reconcile_missing(&[source_id]).await {
                self.report_people_cache(format!("Contact reconciliation needs attention. {error} Finish affected drafts and retry loading contacts in People."));
            }
        });
    }
}
