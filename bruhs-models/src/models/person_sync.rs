//! Bounded contact synchronization without retransmitting unchanged private histories.
use serde::{Deserialize, Serialize};

pub const PERSON_VERSION_PAGE_SIZE: usize = 256;
pub const PERSON_PROFILE_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonVersion {
    pub id: String,
    pub updated_at: i64,
}

/// Why a contact version page is not trustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum PersonVersionPageError {
    TooLarge,
    OutOfOrder,
    CursorDoesNotAdvance,
}

impl PersonVersionPageError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::TooLarge => "Contact version page exceeds its limit.",
            Self::OutOfOrder => "Contact version page has duplicate or out-of-order identities.",
            Self::CursorDoesNotAdvance => "Contact version cursor does not advance from its page.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonVersionPage {
    pub entries: Vec<PersonVersion>,
    pub next_after: Option<String>,
}

impl PersonVersionPage {
    /// Stores supply one extra entry to determine whether another page exists.
    pub fn from_sorted(mut entries: Vec<PersonVersion>) -> Self {
        let more = entries.len() > PERSON_VERSION_PAGE_SIZE;
        entries.truncate(PERSON_VERSION_PAGE_SIZE);
        let next_after = if more {
            entries.last().map(|entry| entry.id.clone())
        } else {
            None
        };
        Self {
            entries,
            next_after,
        }
    }

    pub fn validate(&self, after: Option<&str>) -> Result<(), PersonVersionPageError> {
        let mut previous = after.unwrap_or_default();
        if self.entries.len() > PERSON_VERSION_PAGE_SIZE {
            return Err(PersonVersionPageError::TooLarge);
        }
        for entry in &self.entries {
            if entry.id.trim().is_empty() || entry.id.as_str() <= previous {
                return Err(PersonVersionPageError::OutOfOrder);
            }
            previous = &entry.id;
        }
        if self
            .next_after
            .as_deref()
            .is_some_and(|next| self.entries.len() != PERSON_VERSION_PAGE_SIZE || next != previous)
        {
            return Err(PersonVersionPageError::CursorDoesNotAdvance);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookahead_is_not_returned_or_skipped_by_the_cursor() {
        let page = PersonVersionPage::from_sorted(
            (0..257)
                .map(|n| PersonVersion {
                    id: format!("p{n:03}"),
                    updated_at: n,
                })
                .collect(),
        );
        assert_eq!(page.entries.len(), 256);
        assert_eq!(page.next_after.as_deref(), Some("p255"));
        assert!(page.validate(None).is_ok());
    }

    #[test]
    fn repeated_identity_or_cursor_is_rejected() {
        let mut page = PersonVersionPage::from_sorted(vec![PersonVersion {
            id: "same".into(),
            updated_at: 1,
        }]);
        assert!(page.validate(Some("same")).is_err());
        page.entries.push(page.entries[0].clone());
        assert!(page.validate(None).is_err());
        page.entries.pop();
        page.next_after = Some("same".into());
        assert!(page.validate(None).is_err());
    }
}
