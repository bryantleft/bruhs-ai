use crate::ui::DropdownOption;
use bruhs_models::logic::record_picker::{self, RecordIndex};

pub const PAGE_SIZE: usize = record_picker::PAGE_SIZE;

/// The picker's options with the shared search index over their labels and
/// groups (`bruhs_models::logic::record_picker`, also used by desktop).
#[derive(PartialEq)]
pub(super) struct Index {
    options: Vec<DropdownOption>,
    search: RecordIndex,
}

impl Index {
    pub fn new(options: &[DropdownOption]) -> Self {
        Self {
            options: options.to_vec(),
            search: RecordIndex::new(
                options
                    .iter()
                    .map(|option| (option.label.as_str(), option.group.as_deref())),
            ),
        }
    }

    pub fn matching(&self, query: &str) -> Vec<usize> {
        self.search.matching(query)
    }

    pub fn page(&self, matches: &[usize], highlighted: usize) -> Vec<(usize, DropdownOption)> {
        let start = record_picker::page_start(matches.len(), highlighted);
        matches
            .iter()
            .enumerate()
            .skip(start)
            .take(PAGE_SIZE)
            .filter_map(|(position, index)| {
                self.options
                    .get(*index)
                    .map(|option| (position, option.clone()))
            })
            .collect()
    }

    pub fn value(&self, matches: &[usize], position: usize) -> Option<String> {
        self.options
            .get(*matches.get(position)?)
            .map(|option| option.value.clone())
    }
}

pub(super) fn navigate(current: usize, count: usize, key: &str) -> Option<usize> {
    record_picker::navigate(current, count, key)
}

#[cfg(test)]
mod tests;
