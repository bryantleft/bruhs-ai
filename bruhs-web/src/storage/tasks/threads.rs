//! Thread and transcript writes share one native transaction and parent guard.
use wasm_bindgen::JsValue;

use super::{Kind, Write};
use crate::storage::{
    from_js,
    metadata::{LocalTransaction, SaveError},
    MessagesRecord, STORE_MESSAGES, STORE_THREADS,
};

pub(super) async fn validate_and_cascade(
    tx: &LocalTransaction,
    writes: &[Write<'_>],
) -> Result<(), SaveError> {
    for write in writes {
        if write.kind == Kind::Conversation && write.draft.is_some() {
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
                let messages: MessagesRecord = from_js(raw).map_err(|_| SaveError::Corrupt)?;
                if messages.thread_id != write.id {
                    return Err(SaveError::Conflict);
                }
            }
            // Visited/unsaved history was compared explicitly in this group.
            // Unopened history is validated and deleted here, under the same
            // thread+messages lock; no orphan can survive a successful delete.
            store.delete(key)?.await?;
        }
    }
    Ok(())
}
