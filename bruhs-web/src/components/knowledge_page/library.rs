//! Search complete local text, but clone/render only a bounded page of cards.
use bruhs_models::{KnowledgeDocument, SourceClip};

pub const PAGE_SIZE: usize = 24;

#[derive(Clone, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub detail: String,
    pub tags: Vec<String>,
}

#[derive(PartialEq, Eq)]
struct Entry {
    card: Card,
    fields: Vec<String>,
    updated_at: i64,
}

#[derive(PartialEq, Eq)]
pub struct Index(Vec<Entry>);

impl Index {
    pub fn documents(documents: &[KnowledgeDocument]) -> Self {
        Self::sorted(documents.iter().map(|document| {
            Entry {
                card: Card {
                    id: document.id.clone(),
                    title: document.title.clone(),
                    preview: super::document_preview(&document.body_markdown),
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
                    detail: super::source_host(&source.source_url),
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

    pub fn page(&self, matches: &[usize], page: usize) -> Vec<Card> {
        matches
            .iter()
            .skip(page.min(last_page(matches.len())) * PAGE_SIZE)
            .take(PAGE_SIZE)
            .filter_map(|index| self.0.get(*index))
            .map(|entry| entry.card.clone())
            .collect()
    }
}

pub fn last_page(count: usize) -> usize {
    count.saturating_sub(1) / PAGE_SIZE
}

#[cfg(test)]
mod tests;
