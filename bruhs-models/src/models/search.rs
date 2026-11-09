//! Provider-neutral records and ranking for universal local search.
//!
//! The index owns normalized text so UI surfaces rebuild it only when their
//! source records change. Queries then scan borrowed normalized strings and
//! clone only the small, ranked result set returned to the renderer.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// A domain represented in universal search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Thread,
    Task,
    Project,
    Event,
    SchedulingLink,
    Meeting,
    MeetingNote,
    Person,
    Transcript,
    Document,
    Clip,
    Conversation,
    Finance,
    Health,
}

impl SearchKind {
    /// Human-readable singular label used by both UI surfaces.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Thread => "Thread",
            Self::Task => "Task",
            Self::Project => "Project",
            Self::Event => "Event",
            Self::SchedulingLink => "Scheduling link",
            Self::Meeting => "Meeting",
            Self::MeetingNote => "Meeting note",
            Self::Person => "Person",
            Self::Transcript => "Transcript",
            Self::Document => "Document",
            Self::Clip => "Clip",
            Self::Conversation => "Conversation",
            Self::Finance => "Finance",
            Self::Health => "Health",
        }
    }
}

/// One searchable projection of a domain record.
///
/// `id` remains the domain record id. `subtitle` carries concise navigation
/// context (for example a project or meeting name), while `body` contains text
/// that should be discoverable without dominating title matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDocument {
    pub id: String,
    pub kind: SearchKind,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub updated_at_ms: i64,
}

impl SearchDocument {
    /// Creates a search projection without imposing a provider-specific type.
    pub fn new(
        id: impl Into<String>,
        kind: SearchKind,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        body: impl Into<String>,
        updated_at_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            subtitle: subtitle.into(),
            body: body.into(),
            updated_at_ms,
        }
    }
}

impl From<&crate::MeetingNote> for SearchDocument {
    fn from(note: &crate::MeetingNote) -> Self {
        Self::new(
            &note.id,
            SearchKind::MeetingNote,
            &note.title,
            "Local text note",
            &note.body,
            note.updated_at,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedDocument {
    document: SearchDocument,
    normalized_title: String,
    normalized_subtitle: String,
    normalized_body: String,
}

impl From<SearchDocument> for IndexedDocument {
    fn from(document: SearchDocument) -> Self {
        Self {
            normalized_title: normalize(&document.title),
            normalized_subtitle: normalize(&document.subtitle),
            normalized_body: normalize(&document.body),
            document,
        }
    }
}

/// A ranked result returned by [`SearchIndex::search`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub document: SearchDocument,
    pub score: u64,
}

/// An immutable, reusable local index for universal search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchIndex {
    documents: Vec<IndexedDocument>,
}

impl SearchIndex {
    /// Builds normalized search state once for a collection of projections.
    pub fn new(documents: impl IntoIterator<Item = SearchDocument>) -> Self {
        Self {
            documents: documents.into_iter().map(IndexedDocument::from).collect(),
        }
    }

    /// Returns the number of indexed domain records.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Returns whether the index contains no records.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Ranks records containing every query token and returns at most `limit`.
    ///
    /// Exact and prefix title matches win, followed by title, subtitle, and
    /// body phrase matches. Individual token placement refines the ordering;
    /// recency breaks equal scores. An empty query or zero limit returns no
    /// results.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let normalized_query = normalize(query.trim());
        if normalized_query.is_empty() || limit == 0 {
            return Vec::new();
        }

        let tokens = normalized_query.split_whitespace().collect::<Vec<_>>();
        let mut ranked = self
            .documents
            .iter()
            .enumerate()
            .filter_map(|(index, document)| {
                score_document(document, &normalized_query, &tokens)
                    .map(|score| (index, score, document.document.updated_at_ms))
            })
            .collect::<Vec<_>>();

        ranked.sort_unstable_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| left.0.cmp(&right.0))
        });
        ranked.truncate(limit);
        ranked
            .into_iter()
            .map(|(index, score, _)| SearchHit {
                document: self.documents[index].document.clone(),
                score,
            })
            .collect()
    }
}

fn normalize(value: &str) -> String {
    value.to_lowercase()
}

fn score_document(document: &IndexedDocument, query: &str, tokens: &[&str]) -> Option<u64> {
    let contains_token = |token: &&str| {
        document.normalized_title.contains(*token)
            || document.normalized_subtitle.contains(*token)
            || document.normalized_body.contains(*token)
    };
    if !tokens.iter().all(contains_token) {
        return None;
    }

    let mut score: u64 = match document.normalized_title.as_str().cmp(query) {
        Ordering::Equal => 1_200,
        _ if document.normalized_title.starts_with(query) => 900,
        _ if document.normalized_title.contains(query) => 700,
        _ if document.normalized_subtitle.contains(query) => 500,
        _ if document.normalized_body.contains(query) => 300,
        _ => 100,
    };

    for token in tokens {
        score = score.saturating_add(if document.normalized_title.starts_with(token) {
            80
        } else if document.normalized_title.contains(token) {
            60
        } else if document.normalized_subtitle.contains(token) {
            30
        } else {
            10
        });
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(
        id: &str,
        kind: SearchKind,
        title: &str,
        subtitle: &str,
        body: &str,
        updated_at_ms: i64,
    ) -> SearchDocument {
        SearchDocument::new(id, kind, title, subtitle, body, updated_at_ms)
    }

    #[test]
    fn search_should_rank_exact_title_before_body_match() {
        let index = SearchIndex::new([
            document(
                "body",
                SearchKind::Meeting,
                "Weekly sync",
                "Yesterday",
                "The launch plan is ready",
                20,
            ),
            document(
                "exact",
                SearchKind::Project,
                "Launch plan",
                "Active project",
                "",
                10,
            ),
        ]);

        let hits = index.search("launch plan", 10);

        assert_eq!(hits[0].document.id, "exact");
    }

    #[test]
    fn search_should_require_every_query_token_across_fields() {
        let index = SearchIndex::new([
            document(
                "match",
                SearchKind::Task,
                "Prepare launch",
                "Website",
                "Review the final copy",
                10,
            ),
            document(
                "partial",
                SearchKind::Task,
                "Prepare launch",
                "Website",
                "",
                20,
            ),
        ]);

        let hits = index.search("launch copy", 10);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.id, "match");
    }

    #[test]
    fn search_should_use_recency_to_break_equal_scores() {
        let index = SearchIndex::new([
            document("old", SearchKind::Task, "Review", "", "", 10),
            document("new", SearchKind::Task, "Review", "", "", 20),
        ]);

        let hits = index.search("review", 10);

        assert_eq!(hits[0].document.id, "new");
    }

    #[test]
    fn search_should_be_case_insensitive_and_unicode_safe() {
        let index = SearchIndex::new([document(
            "unicode",
            SearchKind::Person,
            "Renée",
            "Design",
            "Discuss 東京 launch",
            10,
        )]);

        let hits = index.search("RENÉE 東京", 10);

        assert_eq!(hits[0].document.id, "unicode");
    }

    #[test]
    fn search_should_honor_limit_and_empty_queries() {
        let index = SearchIndex::new([
            document("one", SearchKind::Task, "Plan one", "", "", 1),
            document("two", SearchKind::Task, "Plan two", "", "", 2),
        ]);

        assert_eq!(index.search("plan", 1).len(), 1);
        assert!(index.search("   ", 10).is_empty());
        assert!(index.search("plan", 0).is_empty());
    }

    #[test]
    fn search_should_not_overflow_with_many_repeated_tokens() {
        let index = SearchIndex::new([document("a", SearchKind::Task, "a", "", "", 0)]);
        assert_eq!(index.search(&"a ".repeat(1000), 1)[0].document.id, "a");
    }
}
