//! The browser owns conversation history. Record discarded files in the
//! same IndexedDB transaction so a conflict or closed tab can't lose cleanup.

use std::collections::{BTreeMap, BTreeSet};

use bruhs_models::logic::attachments::{
    web_question_upload_key, web_question_upload_prefix, RetireQuestionFiles, MAX_FILES_PER_MESSAGE,
};
use idb::{Transaction, TransactionMode};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

use super::{
    from_js,
    metadata::{finish, LocalTransaction, SaveError},
    open_db, to_js, MessagesRecord, STORE_MESSAGES, STORE_METADATA,
};

const KEY: &str = "discarded-question-files";

#[derive(Serialize, Deserialize)]
struct Outbox {
    key: String,
    files: BTreeMap<String, BTreeSet<String>>,
}

fn keys(record: &MessagesRecord) -> BTreeSet<&str> {
    let prefix = web_question_upload_prefix(record.thread_id());
    record
        .messages()
        .iter()
        .flat_map(|message| &message.files)
        .filter_map(|file| web_question_upload_key(&file.path, &prefix))
        .collect()
}

async fn read(tx: &Transaction) -> Result<Outbox, SaveError> {
    let raw = tx
        .object_store(STORE_METADATA)?
        .get(JsValue::from_str(KEY))?
        .await?;
    raw.map(|value| from_js(value).map_err(|_| SaveError::Corrupt))
        .transpose()
        .map(|outbox| {
            outbox.unwrap_or_else(|| Outbox {
                key: KEY.into(),
                files: BTreeMap::new(),
            })
        })
}

pub(super) async fn stage(
    tx: &Transaction,
    before: &MessagesRecord,
    after: Option<&MessagesRecord>,
) -> Result<bool, SaveError> {
    let old = keys(before);
    if old.is_empty() {
        return Ok(false);
    }
    let retained = after.map(keys).unwrap_or_default();
    let removed = old.difference(&retained);
    if removed.clone().next().is_none() {
        return Ok(false);
    }
    let mut outbox = read(tx).await?;
    outbox
        .files
        .entry(before.thread_id().to_owned())
        .or_default()
        .extend(removed.map(|name| (*name).to_owned()));
    tx.object_store(STORE_METADATA)?
        .put(&to_js(&outbox)?, None)?
        .await?;
    Ok(true)
}

async fn pending() -> Result<Option<(RetireQuestionFiles, Vec<String>)>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_METADATA, STORE_MESSAGES], TransactionMode::ReadOnly)?,
    );
    let result = async {
        let Some((id, names)) = read(&tx).await?.files.into_iter().next() else {
            return Ok(None);
        };
        let thread_id = id.as_str().try_into().map_err(|_| SaveError::Corrupt)?;
        let names = names
            .into_iter()
            .take(MAX_FILES_PER_MESSAGE)
            .collect::<Vec<_>>();
        let transcript = tx
            .object_store(STORE_MESSAGES)?
            .get(JsValue::from_str(&id))?
            .await?
            .map(super::messages::decode)
            .transpose()?;
        let retained = transcript.as_ref().map(keys).unwrap_or_default();
        // A local restore may have brought the file back before reconnecting.
        let files = names
            .iter()
            .filter(|name| !retained.contains(name.as_str()))
            .cloned()
            .collect();
        Ok(Some((RetireQuestionFiles { thread_id, files }, names)))
    }
    .await;
    finish(tx, result).await
}

async fn acknowledge(id: &str, names: &[String]) -> Result<(), SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadWrite)?);
    let result = async {
        let mut outbox = read(&tx).await?;
        if let Some(files) = outbox.files.get_mut(id) {
            for name in names {
                files.remove(name);
            }
            if files.is_empty() {
                outbox.files.remove(id);
            }
            tx.object_store(STORE_METADATA)?
                .put(&to_js(&outbox)?, None)?
                .await?;
        }
        Ok(())
    }
    .await;
    finish(tx, result).await
}

pub(crate) fn retry() {
    wasm_bindgen_futures::spawn_local(async {
        while let Ok(Some((request, names))) = pending().await {
            if !request.files.is_empty()
                && crate::api::retire_question_files(&request).await.is_err()
            {
                return;
            }
            if acknowledge(request.thread_id.as_str(), &names)
                .await
                .is_err()
            {
                return;
            }
        }
    });
}
