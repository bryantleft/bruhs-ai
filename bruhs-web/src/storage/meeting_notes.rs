//! Manual edits compare the exact original in the same transaction as the write.
use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, to_js, STORE_MEETING_NOTES};
use bruhs_models::{MeetingNote, MeetingNoteDraft, MeetingNoteEditError};
use idb::TransactionMode;
use wasm_bindgen::JsValue;

pub async fn save(draft: &MeetingNoteDraft, now: i64) -> Result<MeetingNote, SaveError> {
    let note = draft.prepare(now).map_err(|_| SaveError::Rejected(
        "This note cannot be saved. Check its title, text length and recording link; your draft is retained."
    ))?;
    let db = open_db().await?;
    let tx =
        LocalTransaction::new(db.transaction(&[STORE_MEETING_NOTES], TransactionMode::ReadWrite)?);
    let result = async {
        let store = tx.object_store(STORE_MEETING_NOTES)?;
        let current = store
            .get(JsValue::from_str(&draft.id))?
            .await?
            .map(from_js::<MeetingNote>)
            .transpose()?;
        draft
            .check_current(current.as_ref())
            .map_err(|error| match error {
                MeetingNoteEditError::Removed => SaveError::Rejected(error.message()),
                _ => SaveError::Conflict,
            })?;
        let encoded = to_js(&note)?;
        if current.is_some() {
            store.put(&encoded, None)?.await?;
        } else {
            store.add(&encoded, None)?.await?;
        }
        Ok(note)
    }
    .await;
    finish(tx, result).await
}
