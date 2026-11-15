use super::*;
use bruhs_core::{connections::ConnectionError, mail::MailRuntime};

pub(super) async fn authorization_failed(
    runtime: &MailRuntime,
    mailbox: &str,
    client: &str,
    request: &MailSendRequest,
    error: &MailError,
) {
    let kind = match error {
        MailError::Authorization(ConnectionError::MissingMailPermissions) => {
            MailSendProblemKind::Permission
        }
        MailError::Authorization(
            ConnectionError::AccountMismatch | ConnectionError::RegistrationChanged,
        )
        | MailError::Cache(CacheError::AuthorizationChanged) => MailSendProblemKind::AccountChanged,
        MailError::Authorization(
            ConnectionError::SignIn
            | ConnectionError::Cancelled
            | ConnectionError::IdentityUnavailable
            | ConnectionError::NotConfigured,
        ) => MailSendProblemKind::SignIn,
        _ => MailSendProblemKind::Unavailable,
    };
    if let Err(error) = runtime
        .reject_unqueued_compose(
            mailbox.into(),
            client.into(),
            request.clone(),
            MailSendProblem::new(kind),
        )
        .await
    {
        tracing::warn!(%error, "Could not persist the pre-queue mail rejection");
    }
}
