//! Durable contact cache. Older server replies cannot replace newer versions.

use super::metadata::{finish, LocalTransaction, SaveError};
use super::{from_js, open_db, to_js, PersonProfile, STORE_PEOPLE};
use crate::storage::STORE_METADATA;
use futures_util::future::join_all;
use idb::TransactionMode;
use wasm_bindgen::JsValue;
pub mod merge;
pub mod outbox;

/// Publish bounded, committed read snapshots so one large JS/WASM decode cannot
/// monopolize startup. Cursor keys come from IndexedDB, not untrusted payload IDs.
pub async fn load_progressively(
    mut publish: impl FnMut(Vec<PersonProfile>),
) -> Result<(), SaveError> {
    let db = open_db().await?;
    let mut after = None;
    loop {
        let query = after
            .as_ref()
            .map(|key| idb::KeyRange::lower_bound(key, Some(true)).map(Into::into))
            .transpose()?;
        let tx = LocalTransaction::new(db.transaction(&[STORE_PEOPLE], TransactionMode::ReadOnly)?);
        let result = async {
            let store = tx.object_store(STORE_PEOPLE)?;
            let keys = store.get_all_keys(query.clone(), Some(256))?;
            let values = store.get_all(query, Some(256))?;
            let (keys, values) =
                futures_util::future::join(async { keys.await }, async { values.await }).await;
            let (keys, values) = (keys?, values?);
            if keys.len() != values.len() {
                return Err(SaveError::Corrupt);
            }
            let mut people = Vec::with_capacity(values.len());
            for (key, raw) in keys.iter().zip(values) {
                let person: PersonProfile = from_js(raw).map_err(|_| SaveError::Corrupt)?;
                if key.as_string().as_deref() != Some(person.id.as_str())
                    || person.id.trim().is_empty()
                {
                    return Err(SaveError::Corrupt);
                }
                people.push(person);
            }
            Ok((people, keys.last().cloned()))
        }
        .await;
        let (people, next) = finish(tx, result).await?;
        let done = people.len() < 256;
        publish(people);
        if done {
            return Ok(());
        }
        after = next;
    }
}

fn replacement(current: &PersonProfile, incoming: &PersonProfile) -> Result<bool, SaveError> {
    if current.id != incoming.id {
        return Err(SaveError::Corrupt);
    }
    if incoming.updated_at == current.updated_at && incoming != current {
        return Err(SaveError::Conflict);
    }
    Ok(incoming.updated_at > current.updated_at)
}

pub async fn cache(incoming: &[PersonProfile]) -> Result<Vec<PersonProfile>, SaveError> {
    let mut identities = std::collections::HashSet::new();
    if incoming
        .iter()
        .any(|person| person.id.is_empty() || !identities.insert(person.id.as_str()))
    {
        return Err(SaveError::Rejected(
            "Contact response has missing or duplicate identities. No contacts were cached.",
        ));
    }
    let mut saved = Vec::with_capacity(incoming.len());
    let db = open_db().await?;
    for batch in incoming.chunks(64) {
        let tx = LocalTransaction::new(
            db.transaction(&[STORE_PEOPLE, STORE_METADATA], TransactionMode::ReadWrite)?,
        );
        let result = async {
            let merged = outbox::merged_ids(&tx).await?;
            let pending = merge::read_lock(&tx).await?;
            join_all(
                batch
                    .iter()
                    .filter(|person| {
                        !merged.contains(&person.id)
                            && !pending
                                .as_ref()
                                .is_some_and(|record| record.locks(&person.id))
                    })
                    .map(|person| async {
                        let store = tx.object_store(STORE_PEOPLE)?;
                        if let Some(raw) = store.get(JsValue::from_str(&person.id))?.await? {
                            let current: PersonProfile =
                                from_js(raw).map_err(|_| SaveError::Corrupt)?;
                            if !replacement(&current, person)? {
                                return Ok(current);
                            }
                        }
                        store.put(&to_js(person)?, None)?.await?;
                        Ok::<_, SaveError>(person.clone())
                    }),
            )
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
        }
        .await;
        saved.extend(finish(tx, result).await?);
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_contact_reply_cannot_replace_a_newer_rename() {
        let mut old = PersonProfile::new("Old name");
        old.updated_at = 10;
        let mut newer = old.clone();
        newer.display_name = "New name".into();
        newer.updated_at = 11;
        assert!(replacement(&old, &newer).unwrap());
        assert!(!replacement(&newer, &old).unwrap());
        assert!(!replacement(&newer, &newer).unwrap());
    }
    #[test]
    fn equal_version_disagreement_and_identity_mismatch_are_not_overwritten() {
        let old = PersonProfile::new("Original");
        let mut conflict = old.clone();
        conflict.display_name = "Different".into();
        assert!(matches!(
            replacement(&old, &conflict),
            Err(SaveError::Conflict)
        ));
        conflict.id = "other".into();
        assert!(matches!(
            replacement(&old, &conflict),
            Err(SaveError::Corrupt)
        ));
    }
}
