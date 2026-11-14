//! Row identity and position change less often than streaming row contents.
//! Keep the keyed list/index independent so a token neither clones the whole
//! list for `For` nor makes every row scan all of its siblings.

use super::TimelineRow;
use leptos::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct RowDirectory {
    rows: Memo<Vec<TimelineRow>>,
    pub keys: Memo<Vec<String>>,
    positions: Memo<HashMap<String, usize>>,
}

impl RowDirectory {
    pub fn new(rows: Memo<Vec<TimelineRow>>) -> Self {
        let keys = Memo::new(move |_| {
            rows.with(|rows| {
                rows.iter()
                    .map(|row| row.key().to_string())
                    .collect::<Vec<_>>()
            })
        });
        let positions = Memo::new(move |_| {
            keys.with(|keys| {
                keys.iter()
                    .enumerate()
                    .map(|(index, key)| (key.clone(), index))
                    .collect()
            })
        });
        Self {
            rows,
            keys,
            positions,
        }
    }

    pub fn lookup(self, key: &str) -> Option<TimelineRow> {
        let index = self.positions.with(|index| index.get(key).copied());
        self.rows
            .with(|rows| index.and_then(|index| rows.get(index)).cloned())
    }

    pub fn row(self, key: String) -> Memo<Option<TimelineRow>> {
        Memo::new(move |_| self.lookup(&key))
    }
}

#[cfg(test)]
mod tests;
