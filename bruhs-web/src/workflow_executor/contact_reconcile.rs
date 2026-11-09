//! Missing from a directory response is not deletion evidence. Resolve identities first.
use super::*;
use crate::storage::people::merge;
use bruhs_models::PersonMergeResolution;
use std::collections::HashSet;

impl WorkflowExecutor {
    pub async fn refresh_people(self) {
        if !self.people_hydrated.get_untracked() {
            self.people_refresh_deferred.set(true);
            return;
        }
        if self.people_refreshing.get_untracked() {
            self.people_refresh_again.set(true);
            return;
        }
        self.people_refreshing.set(true);
        loop {
            self.people_refresh_again.set(false);
            match self.refresh_person_pages().await {
                Ok(ids) => {
                    self.people_refresh_error.set(None);
                    let missing = self.people.0.with_untracked(|people| {
                        people
                            .iter()
                            .filter(|person| {
                                !ids.contains(person.id.as_str())
                                    && !self.merge_record.with_untracked(|record| {
                                        record.as_ref().is_some_and(|record| {
                                            record.pending() && record.source.id == person.id
                                        })
                                    })
                            })
                            .map(|person| person.id.clone())
                            .collect::<Vec<_>>()
                    });
                    if let Err(error) = self.reconcile_missing(&missing).await {
                        self.report_people_cache(format!("Contact reconciliation needs attention. {error} Your local contacts and relationship drafts are retained; finish any affected edit and retry loading contacts."));
                    }
                }
                Err(error) => {
                    self.people_refresh_error.set(Some(format!("Contact refresh did not finish. {error} Cached contacts and unsaved drafts are retained; retry loading contacts in People.")));
                }
            }
            if !self.people_refresh_again.get_untracked() {
                break;
            }
        }
        self.people_refreshing.set(false);
    }

    pub fn refreshing_people(self) -> bool {
        self.people_refreshing.get()
    }

    async fn refresh_person_pages(self) -> Result<HashSet<String>, String> {
        let mut known = self.people.0.with_untracked(|people| {
            people
                .iter()
                .map(|p| (p.id.clone(), p.updated_at))
                .collect::<std::collections::HashMap<_, _>>()
        });
        let mut present = HashSet::new();
        let mut after = None;
        loop {
            let page = timed(crate::api::person_versions(after.as_deref())).await?;
            let mut changed = Vec::new();
            for entry in page.entries {
                present.insert(entry.id.clone());
                if known
                    .get(&entry.id)
                    .is_none_or(|version| *version < entry.updated_at)
                {
                    changed.push(entry.id.clone());
                }
                known.insert(entry.id, entry.updated_at);
            }
            // A version page is at most four profile batches. Each cache publishes
            // only after commit, so the first page is usable before the directory ends.
            let results = futures_util::future::join_all(
                changed
                    .chunks(bruhs_models::PERSON_PROFILE_BATCH_SIZE)
                    .map(|ids| async {
                        let profiles = timed(crate::api::get_people(ids)).await?;
                        if profiles.iter().any(|profile| {
                            known
                                .get(&profile.id)
                                .is_some_and(|version| profile.updated_at < *version)
                        }) {
                            return Err(
                                "A contact reply is older than its directory version.".into()
                            );
                        }
                        let returned = profiles
                            .iter()
                            .map(|p| p.id.clone())
                            .collect::<HashSet<_>>();
                        self.cache_people_checked(profiles)
                            .await
                            .map_err(|e| {
                                self.report_people_cache(format!("Contact cache could not save. {e} Retry loading contacts before running a People workflow."));
                                e.to_string()
                            })?;
                        Ok::<_, String>(
                            ids.iter()
                                .filter(|id| !returned.contains(*id))
                                .cloned()
                                .collect::<Vec<_>>(),
                        )
                    }),
            )
            .await;
            for result in results {
                // A profile can be merged between the metadata and body reads.
                for missing in result? {
                    present.remove(&missing);
                }
            }
            match page.next_after {
                Some(next) => after = Some(next),
                None => return Ok(present),
            }
        }
    }

    pub(super) async fn reconcile_missing(self, missing: &[String]) -> Result<(), SaveError> {
        if !missing.is_empty()
            && (!self.people_hydrated.get_untracked()
                || !self.merge_loaded.get_untracked()
                || !self.relationships.hydrated()
                || self.contact_syncing.get_untracked()
                || self.contact_merging.get_untracked())
        {
            // Hydration/sync can finish after the directory request. Resume once
            // ready rather than turning a normal startup race into a user error.
            self.people_refresh_deferred.set(true);
            return Ok(());
        }
        for batch in missing.chunks(64) {
            let resolved = timed(crate::api::resolve_person_merges(batch))
                .await
                .map_err(|_| {
                    SaveError::Rejected(
                        "The server could not confirm whether missing contacts were merged.",
                    )
                })?;
            for redirect in resolved {
                self.reconcile_remote_merge(redirect).await?;
            }
        }
        Ok(())
    }

    async fn reconcile_remote_merge(
        self,
        redirect: PersonMergeResolution,
    ) -> Result<(), SaveError> {
        // An existing local journal has its own exact baselines and explicit retry.
        if !self.merge_loaded.get_untracked()
            || self.contact_merging.get_untracked()
            || self.contact_syncing.get_untracked()
            || self
                .merge_record
                .with_untracked(|record| record.as_ref().is_some_and(merge::Record::pending))
        {
            return Err(SaveError::Rejected(
                "Finish pending contact sync or merge recovery first.",
            ));
        }
        let Some(source) = self.people.0.with_untracked(|people| {
            people
                .iter()
                .find(|person| person.id == redirect.source_id)
                .cloned()
        }) else {
            self.cache_people(vec![redirect.target]).await;
            return Ok(());
        };
        let saved = storage::people::cache(std::slice::from_ref(&redirect.target)).await?;
        let target = saved
            .into_iter()
            .find(|person| person.id == redirect.target.id)
            .ok_or(SaveError::Conflict)?;
        if self.contact_merging.get_untracked() || self.contact_syncing.get_untracked() {
            self.people_refresh_deferred.set(true);
            return Ok(());
        }
        let guard = self
            .relationships
            .begin_merge(&[source.id.clone(), target.id.clone()])?;
        self.contact_merging.set(true);
        let result = merge::prepare_remote(
            source,
            target,
            guard.expected(),
            crate::clock::now_ms(),
            redirect.target,
        )
        .await;
        if let Ok(record) = &result {
            self.publish_merge_record(Some(record.clone()));
        }
        drop(guard);
        let result = match result {
            Ok(record) => self.resume_merge(record).await,
            Err(error) => Err(error),
        };
        self.merge_finished(&result);
        result
    }
}

pub(super) async fn timed<T, E: std::fmt::Display>(
    request: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, String> {
    match futures_util::future::select(
        Box::pin(request),
        Box::pin(gloo_timers::future::TimeoutFuture::new(15_000)),
    )
    .await
    {
        futures_util::future::Either::Left((result, _)) => {
            result.map_err(|error| error.to_string())
        }
        futures_util::future::Either::Right(_) => {
            Err("Contact refresh timed out; the local cache is unchanged.".into())
        }
    }
}
