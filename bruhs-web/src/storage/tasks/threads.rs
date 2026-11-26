//! Thread, transcript and draft writes share their parent guard.
use wasm_bindgen::JsValue;

use super::{Kind, Write};
use crate::storage::{
    from_js,
    metadata::{LocalTransaction, SaveError},
    STORE_COMPOSER_DRAFTS, STORE_MESSAGES, STORE_THREADS,
};

pub(super) async fn validate_and_cascade(
    tx: &LocalTransaction,
    writes: &[Write<'_>],
) -> Result<bool, SaveError> {
    let mut retirement = false;
    for write in writes {
        if matches!(write.kind, Kind::Conversation | Kind::ComposerDraft) && write.draft.is_some() {
            let raw = tx
                .object_store(STORE_THREADS)?
                .get(JsValue::from_str(write.id))?
                .await?
                .ok_or(SaveError::Conflict)?;
            let parent: crate::state::Thread = from_js(raw).map_err(|_| SaveError::Corrupt)?;
            if parent.id != write.id {
                return Err(SaveError::Conflict);
            }
        }
        if write.kind == Kind::Thread && write.draft.is_none() {
            let store = tx.object_store(STORE_MESSAGES)?;
            let key = JsValue::from_str(write.id);
            if let Some(raw) = store.get(key.clone())?.await? {
                let messages = crate::storage::messages::decode(raw)?;
                if messages.thread_id() != write.id {
                    return Err(SaveError::Conflict);
                }
                retirement |=
                    crate::storage::question_file_retirement::stage(tx, &messages, None).await?;
            }
            // Visited/unsaved history was compared explicitly in this group.
            // Unopened history is validated and deleted here, under the same
            // thread+messages lock; no orphan can survive a successful delete.
            store.delete(key.clone())?.await?;
            tx.object_store(STORE_COMPOSER_DRAFTS)?.delete(key)?.await?;
        }
    }
    Ok(retirement)
}
