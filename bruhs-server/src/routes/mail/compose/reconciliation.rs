use super::*;
use bruhs_core::mail::compose::reconciliation::MailSentLookup;

pub(super) async fn reconcile(
    State(state): State<Arc<AppState>>,
    Extension(gmail): Extension<Arc<GmailClient>>,
    Extension(outlook): Extension<Arc<OutlookClient>>,
    Path((mailbox, operation)): Path<(String, String)>,
    body: Result<Json<MailSentReconcileRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<MailSendStatus>, Error> {
    let Json(body) = body.map_err(|_| invalid())?;
    let runtime = runtime(&state)?;
    // This pure local read also validates the host and operation's mailbox.
    let existing = runtime
        .compose_send_status(body.host_id.clone(), mailbox.clone(), operation.clone())
        .await?
        .ok_or_else(|| AppError::NotFound("No saved status is available for this send. Check Sent mail before starting another send.".into()))?;
    if existing.state != MailDispatchState::Uncertain {
        // Do not refresh broker credentials or contact a provider for a state
        // that cannot be reconciled. The normal GET remains pure as well.
        return Ok(Json(existing));
    }
    let selected = runtime
        .mailboxes()
        .await?
        .mailboxes
        .into_iter()
        .find(|row| row.mailbox_id == mailbox)
        .ok_or(MailError::Cache(CacheError::UnknownMailbox))?;
    let lookup: Arc<dyn MailSentLookup> = match selected.provider {
        ServiceProvider::Gmail => gmail,
        ServiceProvider::Outlook => outlook,
        _ => {
            return Err(
                AppError::BadRequest("Choose a Gmail or Outlook mail account.".into()).into(),
            )
        }
    };
    let session = runtime
        .session(
            state.secrets().connections(),
            lookup.provider(),
            &body.connection_id,
        )
        .await?;
    if session.authorization().mailbox().as_str() != mailbox {
        return Err(AppError::Conflict("Choose the account that owns this send.".into()).into());
    }
    let result = session
        .reconcile_uncertain_send(body.host_id, operation, lookup.as_ref())
        .await;
    drop(session);
    Ok(Json(result?))
}
