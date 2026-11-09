use super::*;
use bruhs_models::{KnowledgeCitation, SourceClip};

pub(super) struct SourceGuard {
    added: BTreeSet<String>,
    deleted: BTreeSet<String>,
}

impl SourceGuard {
    pub fn new(writes: &[Write<'_>]) -> Self {
        let mut added = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        for write in writes {
            if write.kind == Kind::SourceClip && write.draft.is_none() {
                deleted.insert(write.id.to_owned());
            }
            if let Some(Record::Document(document)) = write.draft {
                let existing = match write.expected {
                    Some(Record::Document(original)) => original
                        .citations
                        .iter()
                        .map(|citation| citation.clip_id.as_str())
                        .collect::<std::collections::HashSet<_>>(),
                    _ => std::collections::HashSet::new(),
                };
                for citation in &document.citations {
                    if !existing.contains(citation.clip_id.as_str()) {
                        added.insert(citation.clip_id.clone());
                    }
                }
            }
        }
        Self { added, deleted }
    }

    pub fn stores(&self) -> impl Iterator<Item = &'static str> {
        (!self.added.is_empty() || !self.deleted.is_empty())
            .then_some(super::super::STORE_SOURCE_CLIPS)
            .into_iter()
            .chain((!self.deleted.is_empty()).then_some(super::super::STORE_KNOWLEDGE_DOCUMENTS))
    }

    /// Validate after the batch's writes, under the same source/document lock.
    /// A stale tab must not add a citation after deletion, nor may deletion
    /// silently miss a new citing document absent from its in-memory snapshot.
    pub async fn validate(&self, tx: &LocalTransaction) -> Result<(), SaveError> {
        let results = futures_util::future::join_all(self.added.iter().map(|id| async {
            let raw = tx
                .object_store(super::super::STORE_SOURCE_CLIPS)?
                .get(JsValue::from_str(id))?
                .await?
                .ok_or(SaveError::Conflict)?;
            let source: SourceClip = from_js(raw).map_err(|_| SaveError::Corrupt)?;
            if source.id != *id {
                return Err(SaveError::Corrupt);
            }
            Ok(())
        }))
        .await;
        for result in results {
            result?;
        }
        if !self.deleted.is_empty() {
            #[derive(serde::Deserialize)]
            struct CitedDocument {
                #[serde(default)]
                citations: Vec<KnowledgeCitation>,
            }
            let documents = tx
                .object_store(super::super::STORE_KNOWLEDGE_DOCUMENTS)?
                .get_all(None, None)?
                .await?;
            for raw in documents {
                let document: CitedDocument = from_js(raw).map_err(|_| SaveError::Corrupt)?;
                if document
                    .citations
                    .iter()
                    .any(|citation| self.deleted.contains(&citation.clip_id))
                {
                    return Err(SaveError::Conflict);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
