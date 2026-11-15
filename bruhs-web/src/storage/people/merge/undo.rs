use super::*;

/// The local half of undoing a completed merge, after the server confirmed
/// its unmerge. In one transaction: both contacts as the server returned
/// them, both relationships as they were before the merge, the source
/// identity live again, and the links the merge moved pointed back. Refuses
/// with nothing changed when this merge is no longer the latest completed
/// one or the survivor's relationship changed since.
pub async fn unmerge(
    completed: &Record,
    reply: &bruhs_models::PersonUnmerge,
) -> Result<bruhs_models::PersonLinkSet, SaveError> {
    if !matches!(completed.phase, Phase::Completed { .. })
        || reply.source.id != completed.source.id
        || reply.target.id != completed.target.id
    {
        return Err(SaveError::Rejected(
            "The undo reply names other contacts. No records were changed.",
        ));
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[
            STORE_METADATA,
            STORE_PEOPLE,
            STORE_RELATIONSHIPS,
            crate::storage::STORE_INBOX_CONVERSATIONS,
            crate::storage::STORE_KNOWLEDGE_DOCUMENTS,
        ],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        if read_in(&tx).await?.as_ref() != Some(completed) || read_lock(&tx).await?.is_some() {
            return Err(SaveError::Conflict);
        }
        let (source_now, target_now) = futures_util::join!(
            relationships::read_in(&tx, &completed.source.id),
            relationships::read_in(&tx, &completed.target.id)
        );
        if source_now?.is_some() || target_now? != completed.after_relationship {
            return Err(SaveError::Rejected(
                "The relationship details changed after this merge, so this browser kept them. Reload People to see both contacts.",
            ));
        }
        let (mut conversations, mut documents) = super::super::links::load_in(&tx).await?;
        let restored = bruhs_models::restore_person_links(
            &completed.source.id,
            &completed.target.id,
            &completed.relinked,
            &mut conversations,
            &mut documents,
        );
        super::super::links::save_in(&tx, &restored, &conversations, &documents).await?;
        let people = tx.object_store(STORE_PEOPLE)?;
        people.put(&to_js(&reply.source)?, None)?.await?;
        people.put(&to_js(&reply.target)?, None)?.await?;
        let context = tx.object_store(STORE_RELATIONSHIPS)?;
        for (id, before) in [
            (&completed.source.id, &completed.before_source),
            (&completed.target.id, &completed.before_target),
        ] {
            match before {
                Some(profile) => {
                    context.put(&to_js(profile)?, None)?.await?;
                }
                None => {
                    context.delete(JsValue::from_str(id))?.await?;
                }
            }
        }
        outbox::unmark_merged_in(&tx, &completed.source.id).await?;
        // The merge is over, so recovery no longer shows it.
        tx.object_store(STORE_METADATA)?
            .delete(JsValue::from_str(KEY))?
            .await?;
        Ok(restored)
    }
    .await;
    finish(tx, result).await
}
