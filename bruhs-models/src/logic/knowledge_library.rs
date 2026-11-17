//! The Knowledge library's search and pages: the complete local text of every
//! document and source is searched, but only a bounded page of cards is
//! cloned for display. Moved from the web's `knowledge_page/library.rs` so the
//! desktop library searches, orders and pages the same way.

use crate::{KnowledgeDocument, SourceClip};

/// Cards on one page of a library section.
pub const PAGE_SIZE: usize = 24;

/// What a library card shows. Never holds a document's history, links or
/// attachment metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    pub title: String,
    /// A document's first line, or a source's excerpt (160 characters).
    pub preview: String,
    /// A source's host; empty for a document.
    pub detail: String,
    pub tags: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    card: Card,
    /// Lowercased searchable text: title, full body or excerpt, URL, tags.
    fields: Vec<String>,
    updated_at: i64,
}

/// One library section, latest change first, with ties in identity order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Index(Vec<Entry>);

impl Index {
    pub fn documents(documents: &[KnowledgeDocument]) -> Self {
        Self::sorted(documents.iter().map(|document| {
            Entry {
                card: Card {
                    id: document.id.clone(),
                    title: document.title.clone(),
                    preview: document_preview(&document.body_markdown),
                    detail: String::new(),
                    tags: document.tags.clone(),
                },
                fields: [document.title.as_str(), document.body_markdown.as_str()]
                    .into_iter()
                    .chain(document.tags.iter().map(String::as_str))
                    .map(str::to_lowercase)
                    .collect(),
                updated_at: document.updated_at_ms,
            }
        }))
    }

    pub fn sources(sources: &[SourceClip]) -> Self {
        Self::sorted(sources.iter().map(|source| {
            Entry {
                card: Card {
                    id: source.id.clone(),
                    title: source.title.clone(),
                    preview: source.excerpt.chars().take(160).collect(),
                    detail: source_host(&source.source_url),
                    tags: Vec::new(),
                },
                fields: [
                    source.title.as_str(),
                    source.excerpt.as_str(),
                    source.source_url.as_str(),
                ]
                .into_iter()
                .chain(source.tags.iter().map(String::as_str))
                .map(str::to_lowercase)
                .collect(),
                updated_at: source.updated_at_ms,
            }
        }))
    }

    fn sorted(entries: impl Iterator<Item = Entry>) -> Self {
        let mut entries = entries.collect::<Vec<_>>();
        entries.sort_unstable_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.card.id.cmp(&b.card.id))
        });
        Self(entries)
    }

    /// Positions of the entries whose text contains `query`, trimmed and
    /// case folded; a blank query matches everything.
    pub fn matching(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.0
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty() || entry.fields.iter().any(|field| field.contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// The cards on `page` of `matches`; a page past the end shows the last.
    pub fn page(&self, matches: &[usize], page: usize) -> Vec<Card> {
        matches
            .iter()
            .skip(page.min(last_page(matches.len())) * PAGE_SIZE)
            .take(PAGE_SIZE)
            .filter_map(|index| self.0.get(*index))
            .map(|entry| entry.card.clone())
            .collect()
    }

    /// The page of `matches` that holds the record `id`, when it matches.
    pub fn page_of(&self, matches: &[usize], id: &str) -> Option<usize> {
        matches
            .iter()
            .position(|index| self.0.get(*index).is_some_and(|entry| entry.card.id == id))
            .map(|position| position / PAGE_SIZE)
    }
}

/// The last page's index for `count` cards; zero when there are none.
pub fn last_page(count: usize) -> usize {
    count.saturating_sub(1) / PAGE_SIZE
}

/// A document card's preview: its first non-blank line without heading
/// marks, at most 100 characters.
pub fn document_preview(markdown: &str) -> String {
    markdown
        .lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(
            || "Empty document".to_string(),
            |line| {
                line.trim_start_matches('#')
                    .trim()
                    .chars()
                    .take(100)
                    .collect()
            },
        )
}

/// The host a source URL names, without its scheme or path.
pub fn source_host(url: &str) -> String {
    url.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// The document page's save line and empty Backlinks panel, on both apps.
pub mod document_copy {
    pub const SAVED: &str = "Saved locally";
    pub const UNSAVED: &str = "Unsaved changes";
    pub const SAVING: &str = "Saving document and attachments…";
    pub const NOT_SAVED: &str =
        "Document or attachment not saved. Your changes are kept; use the save recovery notice to retry.";
    pub const NO_BACKLINKS: &str = "Nothing links here yet.";
}

#[cfg(test)]
mod tests;
