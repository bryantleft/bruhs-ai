use super::*;

pub async fn complete(expected: &Record) -> Result<Record, SaveError> {
    let Phase::Confirmed { reply } = &expected.phase else {
        return Err(SaveError::Unavailable);
    };
    let mut completed = expected.clone();
    completed.phase = Phase::Completed {
        reply: reply.clone(),
    };
    let mut archive = completed.clone();
    archive.key = format!("{KEY}.archive.{}", archive.id);
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_METADATA, STORE_PEOPLE, STORE_RELATIONSHIPS],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        if read_in(&tx).await?.as_ref() != Some(expected) {
            return Err(SaveError::Conflict);
        }
        if read_lock(&tx).await? != Some(expected.lock()) {
            return Err(SaveError::Conflict);
        }
        check_baselines(&tx, expected).await?;
        // Drain every request, then wait for terminal commit before publishing any UI state.
        let (contacts, context, journal, outbox) = futures_util::join!(
            async {
                let store = tx.object_store(STORE_PEOPLE)?;
                store.put(&to_js(reply.as_ref())?, None)?.await?;
                store
                    .delete(JsValue::from_str(&expected.source.id))?
                    .await?;
                Ok::<_, SaveError>(())
            },
            async {
                let store = tx.object_store(STORE_RELATIONSHIPS)?;
                if let Some(profile) = &expected.after_relationship {
                    store.put(&to_js(profile)?, None)?.await?;
                }
                store
                    .delete(JsValue::from_str(&expected.source.id))?
                    .await?;
                Ok::<_, SaveError>(())
            },
            async {
                let store = tx.object_store(STORE_METADATA)?;
                store.put(&to_js(&archive)?, None)?.await?;
                store.put(&to_js(&completed)?, None)?.await?;
                store.delete(JsValue::from_str(LOCK_KEY))?.await?;
                Ok::<_, SaveError>(())
            },
            outbox::mark_merged_in(&tx, &expected.source.id)
        );
        contacts?;
        context?;
        journal?;
        outbox?;
        Ok(completed)
    }
    .await;
    finish(tx, result).await
}
