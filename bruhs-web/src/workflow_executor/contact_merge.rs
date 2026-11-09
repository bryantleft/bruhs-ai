use super::*;
use crate::storage::people::merge::{self, Phase, Record};

impl WorkflowExecutor {
    pub(super) fn publish_merge_record(self, record: Option<Record>) {
        let ids = record
            .as_ref()
            .filter(|record| record.pending())
            .map(|record| vec![record.source.id.clone(), record.target.id.clone()])
            .unwrap_or_default();
        self.relationships.set_merge_locks(ids);
        self.merge_record.set(record);
    }

    pub fn load_merge(self) {
        if self.contact_merging.get_untracked() {
            return;
        }
        self.merge_loaded.set(false);
        wasm_bindgen_futures::spawn_local(async move {
            match merge::load().await {
                Ok(record) => {
                    self.publish_merge_record(record);
                    self.merge_error.set(None);
                    self.merge_loaded.set(true);
                }
                Err(error) => self.merge_error.set(Some(format!(
                    "Contact merge recovery could not load. {error}"
                ))),
            }
        });
    }

    pub fn merge_people(self, source_id: String, target_id: String) {
        let prepared = (|| {
            if !self.merge_loaded.get_untracked()
                || !self.people_hydrated.get_untracked()
                || self.contact_syncing.get_untracked()
                || self.contact_merging.get_untracked()
                || self.people_cache_error.get_untracked().is_some()
                || self
                    .merge_record
                    .with_untracked(|record| record.as_ref().is_some_and(Record::pending))
            {
                return Err(SaveError::Rejected("Finish contact loading, sync and pending merge recovery before starting another merge."));
            }
            let people = self.people.0.with_untracked(|people| {
                let source = people.iter().find(|person| person.id == source_id).cloned();
                let target = people.iter().find(|person| person.id == target_id).cloned();
                source.zip(target).ok_or(SaveError::Unavailable)
            })?;
            let guard = self.relationships.begin_merge(&[source_id, target_id])?;
            Ok((people, guard))
        })();
        let ((source, target), guard) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.merge_error.set(Some(error.to_string()));
                return;
            }
        };
        self.contact_merging.set(true);
        self.merge_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result =
                merge::prepare(source, target, guard.expected(), crate::clock::now_ms()).await;
            if let Ok(record) = &result {
                self.publish_merge_record(Some(record.clone()));
            }
            drop(guard);
            let result = match result {
                Ok(record) => self.resume_merge(record).await,
                Err(error) => Err(error),
            };
            self.merge_finished(&result);
        });
    }

    pub fn retry_contact_merge(self) {
        if self.contact_merging.get_untracked() {
            return;
        }
        let Some(record) = self.merge_record.get_untracked().filter(Record::pending) else {
            self.load_merge();
            return;
        };
        self.contact_merging.set(true);
        self.merge_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = self.resume_merge(record).await;
            self.merge_finished(&result);
        });
    }

    pub(super) async fn resume_merge(self, mut record: Record) -> Result<(), SaveError> {
        if matches!(record.phase, Phase::Prepared) {
            // A timeout is uncertain, never proof of failure. The durable pair is safe to retry.
            let request = Box::pin(crate::api::merge_people(
                &record.source.id,
                &record.target.id,
            ));
            let timeout = Box::pin(gloo_timers::future::TimeoutFuture::new(15_000));
            let reply = match futures_util::future::select(request, timeout).await {
                futures_util::future::Either::Left((result, _)) => result.map_err(|_| SaveError::Rejected("The server merge could not be confirmed. The saved intent is retained; retry contact merge when connected."))?,
                futures_util::future::Either::Right(_) => return Err(SaveError::Rejected("The server merge timed out. Its outcome is uncertain; the saved intent is retained for retry.")),
            };
            record = merge::confirm(&record, reply).await?;
            self.publish_merge_record(Some(record.clone()));
        }
        let guard = self
            .relationships
            .begin_merge(&[record.source.id.clone(), record.target.id.clone()])?;
        let completed = merge::complete(&record).await?;
        let Phase::Completed { reply } = &completed.phase else {
            return Err(SaveError::Unavailable);
        };
        guard.merged(&completed.source.id, completed.after_relationship.clone());
        self.set_people.0.update(|people| {
            people.retain(|person| person.id != completed.source.id);
            if let Some(target) = people.iter_mut().find(|person| person.id == reply.id) {
                *target = reply.as_ref().clone();
            } else {
                people.push(reply.as_ref().clone());
            }
        });
        self.publish_merge_record(Some(completed));
        Ok(())
    }

    pub(super) fn merge_finished(self, result: &Result<(), SaveError>) {
        if let Err(error) = result {
            self.merge_error
                .set(Some(format!("Contact merge needs attention. {error}")));
        }
        self.contact_merging.set(false);
        if self.contact_sync_again.get_untracked() {
            self.contact_sync_again.set(false);
            self.retry_contact_sync();
        }
    }

    pub fn merge_busy(self) -> bool {
        self.contact_merging.get()
    }
}
