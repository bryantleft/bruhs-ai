//! Local halves of deleting a contact and undoing that delete. Each runs in
//! one transaction with the private links it rewrites.

use bruhs_models::{PersonLinkSet, RelationshipProfile};

use super::*;
use crate::storage::{relationships, STORE_RELATIONSHIPS};

/// What a local delete removed.
pub struct Deleted {
    /// The private links that were unlinked.
    pub links: PersonLinkSet,
    /// The contact never reached the server.
    pub was_pending: bool,
}

fn stores() -> [&'static str; 5] {
    [
        STORE_METADATA,
        STORE_PEOPLE,
        STORE_RELATIONSHIPS,
        crate::storage::STORE_INBOX_CONVERSATIONS,
        crate::storage::STORE_KNOWLEDGE_DOCUMENTS,
    ]
}

/// Delete cascade: the contact (with its observations and aliases), its
/// relationship (details and reminders) and its sync entry go; Inbox
/// participants and knowledge links stay, unlinked. Refuses with nothing
/// changed when either record changed since the caller read it, or when a
/// merge holds the contact.
pub async fn delete(
    person: &PersonProfile,
    relationship: Option<&RelationshipProfile>,
) -> Result<Deleted, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&stores(), TransactionMode::ReadWrite)?);
    let result = async {
        if merge::read_lock(&tx).await?.is_some_and(|lock| lock.locks(&person.id)) {
            return Err(SaveError::Rejected(
                "This contact has a pending merge. Retry that merge before deleting it. No records were changed.",
            ));
        }
        let stored = tx
            .object_store(STORE_PEOPLE)?
            .get(JsValue::from_str(&person.id))?
            .await?
            .map(|raw| from_js::<PersonProfile>(raw).map_err(|_| SaveError::Corrupt))
            .transpose()?;
        // A contact the server sent but this browser never cached is still deletable.
        if stored.as_ref().is_some_and(|stored| stored != person) {
            return Err(SaveError::Conflict);
        }
        if relationships::read_in(&tx, &person.id).await?.as_ref() != relationship {
            return Err(SaveError::Conflict);
        }
        let was_pending = outbox::forget_in(&tx, &person.id).await?;
        let (mut conversations, mut documents) = super::links::load_in(&tx).await?;
        let links = bruhs_models::unlink_person(&person.id, &mut conversations, &mut documents);
        super::links::save_in(&tx, &links, &conversations, &documents).await?;
        tx.object_store(STORE_PEOPLE)?
            .delete(JsValue::from_str(&person.id))?
            .await?;
        tx.object_store(STORE_RELATIONSHIPS)?
            .delete(JsValue::from_str(&person.id))?
            .await?;
        Ok(Deleted { links, was_pending })
    }
    .await;
    finish(tx, result).await
}

/// Undo of [`delete`]: the contact and its relationship come back and the
/// recorded links that are still unlinked are relinked. `person` is the
/// server's restored record, or the deleted one for a local-only delete,
/// which waits for its first sync again. Returns the links put back.
pub async fn restore(
    person: &PersonProfile,
    relationship: Option<&RelationshipProfile>,
    links: &PersonLinkSet,
    local_only: bool,
) -> Result<PersonLinkSet, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&stores(), TransactionMode::ReadWrite)?);
    let result = async {
        if outbox::merged_ids(&tx).await?.contains(&person.id) {
            return Err(SaveError::Rejected(
                "This contact identity was merged. No records were changed.",
            ));
        }
        let people = tx.object_store(STORE_PEOPLE)?;
        if people.get(JsValue::from_str(&person.id))?.await?.is_some()
            || relationships::read_in(&tx, &person.id).await?.is_some()
        {
            return Err(SaveError::Rejected(
                "This contact already exists again. No records were changed.",
            ));
        }
        people.put(&to_js(person)?, None)?.await?;
        if let Some(profile) = relationship {
            tx.object_store(STORE_RELATIONSHIPS)?
                .put(&to_js(profile)?, None)?
                .await?;
        }
        if local_only {
            outbox::restage_in(&tx, &person.id).await?;
        }
        let (mut conversations, mut documents) = super::links::load_in(&tx).await?;
        let restored =
            bruhs_models::relink_person(&person.id, links, &mut conversations, &mut documents);
        super::links::save_in(&tx, &restored, &conversations, &documents).await?;
        Ok(restored)
    }
    .await;
    finish(tx, result).await
}
