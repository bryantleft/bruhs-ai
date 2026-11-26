//! One original-checked record per local thread.

use idb::TransactionMode;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

use crate::components::file_attachments::SavedFileOffer;
use crate::state::Draft;

use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, STORE_COMPOSER_DRAFTS};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DraftRecord {
    pub thread_id: String,
    pub draft: Draft,
    pub files: Vec<SavedFileOffer>,
}

pub(super) fn decode(raw: JsValue, id: &str) -> Result<DraftRecord, SaveError> {
    let draft: DraftRecord = from_js(raw).map_err(|_| SaveError::Corrupt)?;
    if draft.thread_id != id {
        return Err(SaveError::Corrupt);
    }
    Ok(draft)
}

pub(crate) async fn load(id: &str) -> Result<Option<DraftRecord>, SaveError> {
    let db = open_db().await?;
    let tx =
        LocalTransaction::new(db.transaction(&[STORE_COMPOSER_DRAFTS], TransactionMode::ReadOnly)?);
    let result = async {
        tx.object_store(STORE_COMPOSER_DRAFTS)?
            .get(JsValue::from_str(id))?
            .await?
            .map(|raw| decode(raw, id))
            .transpose()
    }
    .await;
    finish(tx, result).await
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use bruhs_models::logic::{attachments::FileKind, composer_text::TextAttachment};

    use crate::components::file_attachments::{offers_block_send, FileOffer, OfferPhase};

    use super::*;

    #[test]
    fn draft_round_trip_keeps_ready_files_and_marks_interrupted_uploads() {
        let mut file = FileOffer {
            id: "attachment".into(),
            name: "Notes.txt".into(),
            kind: FileKind::Text,
            declared_bytes: 4,
            phase: OfferPhase::Ready {
                path: "files/notes.txt".into(),
            },
            bytes: None,
        };
        let draft = DraftRecord {
            thread_id: "thread".into(),
            draft: Draft {
                text: "Review the notes".into(),
                attachments: vec![TextAttachment {
                    name: "Paste".into(),
                    text: "Context".into(),
                }],
                ..Draft::default()
            },
            files: vec![SavedFileOffer::from(&file)],
        };
        let json = serde_json::to_string(&draft).unwrap();
        let restored: DraftRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, draft);
        let mut restored_files = restored
            .files
            .into_iter()
            .map(FileOffer::from)
            .collect::<Vec<_>>();
        assert_eq!(restored_files[0].ready_path(), file.ready_path());
        assert!(!offers_block_send(&restored_files));

        file.phase = OfferPhase::Failed {
            message: "Offline".into(),
            retry: true,
        };
        file.bytes = Some(Rc::new(vec![1, 2, 3, 4]));
        let interrupted = FileOffer::from(SavedFileOffer::from(&file));
        assert!(interrupted.bytes.is_none());
        assert!(matches!(
            interrupted.phase,
            OfferPhase::Failed { retry: false, .. }
        ));
        restored_files.push(interrupted);
        assert!(offers_block_send(&restored_files));
    }
}
