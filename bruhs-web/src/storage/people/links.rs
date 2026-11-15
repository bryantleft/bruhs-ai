//! Private links to a contact that live outside the contact record: Inbox
//! participants and knowledge links. People changes rewrite them inside the
//! caller's transaction, so a contact and its links never disagree on disk.

use bruhs_models::{InboxConversation, KnowledgeDocument, PersonLinkSet};

use super::*;
use crate::storage::{STORE_INBOX_CONVERSATIONS, STORE_KNOWLEDGE_DOCUMENTS};

/// Every Inbox conversation and knowledge document, read in `tx`.
pub(super) async fn load_in(
    tx: &idb::Transaction,
) -> Result<(Vec<InboxConversation>, Vec<KnowledgeDocument>), SaveError> {
    let (conversations, documents) = futures_util::join!(
        async {
            tx.object_store(STORE_INBOX_CONVERSATIONS)?
                .get_all(None, None)?
                .await?
                .into_iter()
                .map(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))
                .collect::<Result<Vec<InboxConversation>, SaveError>>()
        },
        async {
            tx.object_store(STORE_KNOWLEDGE_DOCUMENTS)?
                .get_all(None, None)?
                .await?
                .into_iter()
                .map(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))
                .collect::<Result<Vec<KnowledgeDocument>, SaveError>>()
        }
    );
    Ok((conversations?, documents?))
}

/// Write back only the records `changed` names.
pub(super) async fn save_in(
    tx: &idb::Transaction,
    changed: &PersonLinkSet,
    conversations: &[InboxConversation],
    documents: &[KnowledgeDocument],
) -> Result<(), SaveError> {
    let conversation_ids = changed.conversation_ids();
    let document_ids = changed.document_ids();
    let store = tx.object_store(STORE_INBOX_CONVERSATIONS)?;
    for conversation in conversations
        .iter()
        .filter(|conversation| conversation_ids.contains(conversation.id.as_str()))
    {
        store.put(&to_js(conversation)?, None)?.await?;
    }
    let store = tx.object_store(STORE_KNOWLEDGE_DOCUMENTS)?;
    for document in documents
        .iter()
        .filter(|document| document_ids.contains(document.id.as_str()))
    {
        store.put(&to_js(document)?, None)?.await?;
    }
    Ok(())
}
