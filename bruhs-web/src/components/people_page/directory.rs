use std::collections::HashMap;

use bruhs_models::{names_match_query, PersonProfile};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub name: String,
    pub aliases: Vec<String>,
    pub tracked: bool,
    pub observations: usize,
}

#[derive(Default, PartialEq, Eq)]
pub struct Directory {
    pub order: Vec<String>,
    pub summaries: HashMap<String, Summary>,
}

impl Directory {
    pub fn new(people: &[PersonProfile], previous: Option<&Self>) -> Self {
        let mut remaining = people
            .iter()
            .map(|person| (person.id.as_str(), person))
            .collect::<HashMap<_, _>>();
        let mut sorted = Vec::with_capacity(people.len());
        if let Some(previous) = previous {
            // Updates must not move an open editor across a page boundary.
            for id in &previous.order {
                if let Some(person) = remaining.remove(id.as_str()) {
                    sorted.push(person);
                }
            }
        }
        let mut added = remaining.into_values().collect::<Vec<_>>();
        added.sort_unstable_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        sorted.extend(added);
        let mut directory = Self::default();
        for person in sorted {
            directory.order.push(person.id.clone());
            directory.summaries.insert(
                person.id.clone(),
                Summary {
                    name: person.display_name.clone(),
                    aliases: person.aliases.clone(),
                    tracked: person.tracked,
                    observations: person.observations.len(),
                },
            );
        }
        directory
    }

    #[cfg(test)]
    pub fn page(&self, page: usize) -> Vec<String> {
        bruhs_models::people_page(&self.order, page).to_vec()
    }

    /// Ids whose name or alias matches `query`, in directory order. A blank
    /// query keeps everyone.
    pub fn matching(&self, query: &str) -> Vec<String> {
        if query.trim().is_empty() {
            return self.order.clone();
        }
        self.order
            .iter()
            .filter(|id| {
                self.summaries.get(id.as_str()).is_some_and(|summary| {
                    names_match_query(
                        std::iter::once(summary.name.as_str())
                            .chain(summary.aliases.iter().map(String::as_str)),
                        query,
                    )
                })
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_pages_every_identity_without_copying_private_histories() {
        let people = (0..205)
            .map(|n| {
                let mut person = PersonProfile::new(format!("Person {n}"));
                person.id = format!("p-{n:03}");
                person.updated_at = n;
                person.voice_embedding = Some(vec![0.5; 1000]);
                person
            })
            .collect::<Vec<_>>();
        let directory = Directory::new(&people, None);
        assert_eq!(directory.page(0).len(), 25);
        assert_eq!(directory.page(1).len(), 25);
        assert_eq!(directory.page(8).len(), 5);
        assert_eq!(
            (0..9)
                .flat_map(|page| directory.page(page))
                .collect::<Vec<_>>(),
            directory.order
        );
        assert!(directory.page(usize::MAX).is_empty());
        assert_eq!(directory.order.first().unwrap(), "p-204");
        assert_eq!(directory.summaries["p-204"].name, "Person 204");
    }

    #[test]
    fn search_reaches_aliases_beyond_the_first_page_without_reordering() {
        let mut people = (0..60)
            .map(|n| {
                let mut person = PersonProfile::new(format!("Person {n}"));
                person.id = format!("p-{n:03}");
                person.updated_at = 100 + n;
                person
            })
            .collect::<Vec<_>>();
        let mut late = PersonProfile::new("Zoë Ångström");
        late.id = "late".into();
        late.aliases = vec!["The Captain".into()];
        late.updated_at = 0;
        people.push(late);
        let directory = Directory::new(&people, None);
        assert!(!directory.page(0).contains(&"late".to_string()));
        assert_eq!(directory.matching("CAPTAIN"), vec!["late".to_string()]);
        assert_eq!(directory.matching("zoË"), vec!["late".to_string()]);
        assert_eq!(directory.matching("  "), directory.order);
        assert!(directory.matching("nobody").is_empty());
    }

    #[test]
    fn edits_and_new_contacts_do_not_reorder_existing_editors() {
        let mut first = PersonProfile::new("First");
        first.updated_at = 1;
        let mut second = PersonProfile::new("Second");
        second.updated_at = 2;
        let before = Directory::new(&[first.clone(), second.clone()], None);
        first.display_name = "Renamed".into();
        first.updated_at = 100;
        let added = PersonProfile::new("New contact");
        let after = Directory::new(&[first.clone(), second, added.clone()], Some(&before));
        assert_eq!(&after.order[..2], before.order);
        assert_eq!(after.order[2], added.id);
        assert_eq!(after.summaries[&first.id].name, "Renamed");
    }
}
