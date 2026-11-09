use bruhs_models::PersonProfile;

use super::*;

impl WorkflowExecutor {
    pub async fn cache_people(self, incoming: Vec<PersonProfile>) {
        if let Err(error) = self.cache_people_checked(incoming).await {
            self.report_people_cache(format!(
                "Contact cache could not save. {error} Server changes are not undone; retry loading contacts before running a People workflow."
            ));
        }
    }

    pub(super) async fn cache_people_checked(
        self,
        incoming: Vec<PersonProfile>,
    ) -> Result<(), SaveError> {
        let saved = storage::people::cache(&incoming).await?;
        self.set_people.0.update(|current| {
            let saved = self.merge_record.with_untracked(|merge| {
                saved
                    .into_iter()
                    .filter(|person| {
                        !merge.as_ref().is_some_and(|merge| {
                            merge.locks(&person.id) || person.id == merge.source.id
                        })
                    })
                    .collect()
            });
            merge_cached(current, saved);
        });
        Ok(())
    }

    pub fn retry_people(self) {
        wasm_bindgen_futures::spawn_local(async move {
            if self.hydrate_people().await {
                self.people_refresh_error.set(None);
                self.refresh_people().await;
            }
        });
    }

    pub async fn hydrate_people(self) -> bool {
        if self.people_loading.get_untracked() {
            return false;
        }
        self.people_loading.set(true);
        self.people_hydrated.set(false);
        let loaded = match storage::people::load_progressively(|people| {
            self.set_people
                .0
                .update(|current| merge_cached(current, people));
        })
        .await
        {
            Ok(()) => {
                self.people_hydrated.set(true);
                self.people_cache_error.set(None);
                self.retry_contact_sync();
                true
            }
            Err(error) => {
                self.people_cache_error
                    .set(Some(format!("Contact cache could not load. {error}")));
                false
            }
        };
        self.people_loading.set(false);
        loaded
    }

    pub fn loading_people(self) -> bool {
        self.people_loading.get()
    }
}

fn merge_cached(current: &mut Vec<PersonProfile>, incoming: Vec<PersonProfile>) {
    let mut positions = current
        .iter()
        .enumerate()
        .map(|(index, person)| (person.id.clone(), index))
        .collect::<std::collections::HashMap<_, _>>();
    for person in incoming {
        if let Some(index) = positions.get(&person.id) {
            let existing = &mut current[*index];
            if person.updated_at > existing.updated_at {
                *existing = person;
            }
        } else {
            positions.insert(person.id.clone(), current.len());
            current.push(person);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_merge_preserves_newer_local_values_and_unique_identity() {
        let mut original = PersonProfile::new("Original");
        original.updated_at = 10;
        let mut newer = original.clone();
        newer.display_name = "Newer name".into();
        newer.updated_at = 11;
        let additional = PersonProfile::new("Additional");
        let mut current = vec![newer.clone()];
        merge_cached(
            &mut current,
            vec![original, additional.clone(), additional.clone()],
        );
        assert_eq!(current, vec![newer, additional]);
    }

    #[test]
    fn contact_merge_updates_in_place_without_reordering_the_directory() {
        let mut original = PersonProfile::new("Original");
        original.updated_at = 10;
        let mut newer = original.clone();
        newer.display_name = "Renamed".into();
        newer.updated_at = 11;
        let other = PersonProfile::new("Other");
        let mut current = vec![original, other.clone()];
        merge_cached(&mut current, vec![newer.clone()]);
        assert_eq!(current, vec![newer, other]);
    }
}
