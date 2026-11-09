//! Strict hydration and compare-and-swap saves for private relationship data.

use bruhs_models::RelationshipProfile;
use idb::{Transaction, TransactionMode};
use wasm_bindgen::JsValue;

use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, to_js, STORE_METADATA, STORE_RELATIONSHIPS};

pub async fn read_in(
    tx: &Transaction,
    person_id: &str,
) -> Result<Option<RelationshipProfile>, SaveError> {
    tx.object_store(STORE_RELATIONSHIPS)?
        .get(JsValue::from_str(person_id))?
        .await?
        .map(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))
        .transpose()
}

pub async fn load() -> Result<Vec<RelationshipProfile>, SaveError> {
    let db = open_db().await?;
    let tx =
        LocalTransaction::new(db.transaction(&[STORE_RELATIONSHIPS], TransactionMode::ReadOnly)?);
    let result = async {
        let raw = tx
            .object_store(STORE_RELATIONSHIPS)?
            .get_all(None, None)?
            .await?;
        let mut profiles = raw
            .into_iter()
            .map(|raw| from_js::<RelationshipProfile>(raw).map_err(|_| SaveError::Corrupt))
            .collect::<Result<Vec<_>, _>>()?;
        profiles.sort_unstable_by_key(|profile| std::cmp::Reverse(profile.updated_at_ms));
        Ok(profiles)
    }
    .await;
    finish(tx, result).await
}

pub async fn save(
    expected: Option<&RelationshipProfile>,
    draft: &RelationshipProfile,
) -> Result<(), SaveError> {
    if draft.person_id.is_empty()
        || expected.is_some_and(|profile| profile.person_id != draft.person_id)
    {
        return Err(SaveError::Rejected(
            "Relationship identity changed. No replacement was saved.",
        ));
    }
    // Encode before opening the transaction; only the edited person's record
    // is copied, never the entire People collection.
    let value = to_js(draft)?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_RELATIONSHIPS, STORE_METADATA],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        super::people::merge::ensure_editable(&tx, &draft.person_id).await?;
        if read_in(&tx, &draft.person_id).await?.as_ref() != expected {
            return Err(SaveError::Conflict);
        }
        tx.object_store(STORE_RELATIONSHIPS)?
            .put(&value, None)?
            .await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}
