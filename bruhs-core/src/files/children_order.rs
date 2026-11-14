use super::FileEntry;
use std::{cmp::Ordering, collections::BinaryHeap, io};

pub(super) struct Selection {
    limit: usize,
    next_order: usize,
    entries: Vec<FileEntry>,
    best: Option<BinaryHeap<Ranked>>,
}

struct Ranked {
    key: (bool, String, usize),
    entry: FileEntry,
}

impl Selection {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            next_order: 0,
            entries: Vec::new(),
            best: None,
        }
    }

    pub(super) fn push(&mut self, entry: FileEntry) -> io::Result<()> {
        if self.limit == 0 {
            return Ok(());
        }
        let order = self.next_order;
        self.next_order = order
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Directory entry order is exhausted."))?;
        if self.best.is_none() && self.entries.len() < self.limit {
            self.entries.push(entry);
            return Ok(());
        }
        let best = self.best.get_or_insert_with(|| {
            std::mem::take(&mut self.entries)
                .into_iter()
                .enumerate()
                .map(|(order, entry)| Ranked {
                    key: (!entry.is_directory, entry.name.to_lowercase(), order),
                    entry,
                })
                .collect()
        });
        let is_file = !entry.is_directory;
        if best.peek().is_some_and(|worst| is_file && !worst.key.0) {
            return Ok(());
        }
        // The read ordinal preserves stable ties, including at the cutoff.
        let candidate = Ranked {
            key: (is_file, entry.name.to_lowercase(), order),
            entry,
        };
        match best.peek_mut() {
            Some(mut worst) if candidate.key < worst.key => *worst = candidate,
            _ => {}
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Vec<FileEntry> {
        match self.best {
            Some(best) => best
                .into_sorted_vec()
                .into_iter()
                .map(|ranked| ranked.entry)
                .collect(),
            None => {
                sort(&mut self.entries);
                self.entries
            }
        }
    }
}

fn sort(entries: &mut [FileEntry]) {
    // With at most one comparison, cached keys cannot be reused.
    if entries.len() <= 2 {
        entries.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        return;
    }
    entries.sort_by_cached_key(|entry| (!entry.is_directory, entry.name.to_lowercase()));
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(mut entries: Vec<FileEntry>, limit: usize) -> Vec<FileEntry> {
        entries.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        entries.truncate(limit);
        entries
    }

    fn selected(entries: Vec<FileEntry>, limit: usize) -> Vec<FileEntry> {
        let mut selection = Selection::new(limit);
        for entry in entries {
            selection.push(entry).expect("fixture ordinal fits");
        }
        selection.finish()
    }

    #[test]
    fn groups_unicode_and_stable_ties_match_every_reference_prefix() {
        let names = [
            "I", "i", "İ", "i\u{307}", "K", "K", "k", "ΟΣ", "ος", "οσ", "É", "e\u{301}", "ß", "SS",
            "😀",
        ];
        for reverse in [false, true] {
            let mut entries: Vec<_> = [false, true]
                .into_iter()
                .flat_map(|is_directory| {
                    names
                        .iter()
                        .enumerate()
                        .map(move |(index, name)| FileEntry {
                            path: format!("{is_directory}/{index}"),
                            name: (*name).to_owned(),
                            is_directory,
                        })
                })
                .collect();
            if reverse {
                entries.reverse();
            }
            for limit in 0..=entries.len() + 1 {
                assert_eq!(
                    selected(entries.clone(), limit),
                    reference(entries.clone(), limit)
                );
            }
        }
    }

    #[test]
    fn ties_keep_the_original_membership_at_the_500_entry_cutoff() {
        let entries: Vec<_> = (0..1001)
            .map(|index| FileEntry {
                path: format!("ordinal-{index}"),
                name: if index % 2 == 0 { "K/" } else { "k/" }.to_owned(),
                is_directory: true,
            })
            .collect();
        assert_eq!(selected(entries.clone(), 500), entries[..500]);
    }

    #[test]
    fn late_best_entries_replace_earlier_worse_entries_without_exceeding_the_limit() {
        let entries: Vec<_> = (0..1001)
            .rev()
            .map(|index| FileEntry {
                path: format!("p{index:04}"),
                name: format!("n{index:04}"),
                is_directory: index % 3 == 0,
            })
            .collect();
        let expected = reference(entries.clone(), 500);
        let mut selection = Selection::new(500);
        for entry in entries {
            selection.push(entry).expect("fixture ordinal fits");
            assert!(
                selection.entries.len() + selection.best.as_ref().map_or(0, BinaryHeap::len) <= 500
            );
            assert!(selection.best.is_none() || selection.entries.is_empty());
        }
        assert_eq!(selection.finish(), expected);
    }

    #[test]
    fn exhausted_read_order_fails_without_returning_a_partial_success() {
        let mut selection = Selection::new(500);
        selection.next_order = usize::MAX;
        let error = selection
            .push(FileEntry {
                path: "a".to_owned(),
                name: "a".to_owned(),
                is_directory: false,
            })
            .expect_err("ordinal overflow");
        assert_eq!(error.to_string(), "Directory entry order is exhausted.");
        assert!(selection.best.is_none());
        assert!(selection.entries.is_empty());
    }
}
