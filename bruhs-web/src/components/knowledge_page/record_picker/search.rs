use crate::ui::DropdownOption;

pub const PAGE_SIZE: usize = 24;

#[derive(PartialEq)]
pub(super) struct Index(Vec<(DropdownOption, String, String)>);

impl Index {
    pub fn new(options: &[DropdownOption]) -> Self {
        Self(
            options
                .iter()
                .map(|option| {
                    (
                        option.clone(),
                        option.label.to_lowercase(),
                        option.group.as_deref().unwrap_or_default().to_lowercase(),
                    )
                })
                .collect(),
        )
    }

    pub fn matching(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.0
            .iter()
            .enumerate()
            .filter(|(_, (_, label, group))| {
                query.is_empty() || label.contains(&query) || group.contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn page(&self, matches: &[usize], highlighted: usize) -> Vec<(usize, DropdownOption)> {
        let start = highlighted.min(matches.len().saturating_sub(1)) / PAGE_SIZE * PAGE_SIZE;
        matches
            .iter()
            .enumerate()
            .skip(start)
            .take(PAGE_SIZE)
            .filter_map(|(position, index)| {
                self.0
                    .get(*index)
                    .map(|(option, _, _)| (position, option.clone()))
            })
            .collect()
    }

    pub fn value(&self, matches: &[usize], position: usize) -> Option<String> {
        self.0
            .get(*matches.get(position)?)
            .map(|(option, _, _)| option.value.clone())
    }
}

pub(super) fn navigate(current: usize, count: usize, key: &str) -> Option<usize> {
    let last = count.saturating_sub(1);
    let current = current.min(last);
    Some(match key {
        "ArrowDown" => current.saturating_add(1).min(last),
        "ArrowUp" => current.saturating_sub(1),
        "PageDown" => current.saturating_add(PAGE_SIZE).min(last),
        "PageUp" => current.saturating_sub(PAGE_SIZE),
        "Home" => 0,
        "End" => last,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
