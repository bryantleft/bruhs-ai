//! The vault routes (D22) for Settings, Vault. Values leave only in the
//! write-only create and update bodies and never come back: every answer is
//! metadata. The browser keeps none of it beyond the page.

use std::future::Future;

use super::http::{decode, send_request, url};
use super::{segment, ApiError};
use bruhs_models::vault::{
    vault_failure_message, VaultCheck, VaultCreate, VaultEntry, VaultFailure, VaultListing,
    VaultUpdate, VAULT_CALL_TIMEOUT_MS,
};
use futures_util::future::{select, Either};
use gloo_net::http::{Method, RequestBuilder, Response};
use serde::{de::DeserializeOwned, Serialize};

const VAULT_PATH: &str = "/api/vault";

fn vault_entry_path(id: &str) -> String {
    format!("{VAULT_PATH}/{}", segment(id))
}

fn vault_check_path(id: &str) -> String {
    format!("{}/check", vault_entry_path(id))
}

/// What a vault page shows for a failed call, by the desktop's rule
/// (`bruhs_models::vault::vault_failure_message`): a refusal's or an
/// unavailable store's own sentence, else fixed copy. A call that ran out
/// of time says the service couldn't be reached.
pub fn vault_message(error: &ApiError) -> String {
    let failure = match error {
        ApiError::Network(_) | ApiError::Timeout { .. } => VaultFailure::NoAnswer,
        ApiError::Http { status, detail } => VaultFailure::Status {
            code: *status,
            error: detail.as_deref(),
        },
        ApiError::Decode(_) | ApiError::Aborted => VaultFailure::Other,
    };
    vault_failure_message(failure).to_owned()
}

/// `call`, unless `deadline` comes first: then `abort` drops the request
/// and the call fails as timed out, so the page's buttons come back.
async fn bounded<T>(
    call: impl Future<Output = Result<T, ApiError>>,
    deadline: impl Future<Output = ()>,
    abort: impl FnOnce(),
) -> Result<T, ApiError> {
    futures_util::pin_mut!(call);
    futures_util::pin_mut!(deadline);
    match select(call, deadline).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => {
            abort();
            Err(ApiError::Timeout {
                after_ms: VAULT_CALL_TIMEOUT_MS,
            })
        }
    }
}

/// One vault call within [`VAULT_CALL_TIMEOUT_MS`], as the desktop waits:
/// a keychain or 1Password prompt can hold the server, and the page must
/// not wait on it for good. `read` takes the successful response.
async fn vault_call<B, T, R>(
    method: Method,
    route: &str,
    body: Option<&B>,
    read: impl FnOnce(Response) -> R,
) -> Result<T, ApiError>
where
    B: Serialize + ?Sized,
    R: Future<Output = Result<T, ApiError>>,
{
    let abort = web_sys::AbortController::new()
        .map_err(|_| ApiError::Network("The browser cannot cancel vault requests.".into()))?;
    let call = async {
        let builder = RequestBuilder::new(&url(route))
            .method(method)
            .abort_signal(Some(&abort.signal()));
        let request = match body {
            Some(body) => builder.json(body)?,
            None => builder.build()?,
        };
        read(send_request(request).await?).await
    };
    let deadline = gloo_timers::future::TimeoutFuture::new(VAULT_CALL_TIMEOUT_MS);
    bounded(call, deadline, || abort.abort()).await
}

async fn vault_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    method: Method,
    route: &str,
    body: Option<&B>,
) -> Result<T, ApiError> {
    vault_call(method, route, body, decode).await
}

/// Every entry's metadata, the bruhs' names and whether `op` was found.
/// An older server answers 404.
pub async fn vault_listing() -> Result<VaultListing, ApiError> {
    vault_json::<(), _>(Method::GET, VAULT_PATH, None).await
}

pub async fn create_vault_entry(create: &VaultCreate) -> Result<VaultEntry, ApiError> {
    vault_json(Method::POST, VAULT_PATH, Some(create)).await
}

pub async fn update_vault_entry(id: &str, update: &VaultUpdate) -> Result<VaultEntry, ApiError> {
    vault_json(Method::PUT, &vault_entry_path(id), Some(update)).await
}

pub async fn delete_vault_entry(id: &str) -> Result<(), ApiError> {
    vault_call::<(), _, _>(Method::DELETE, &vault_entry_path(id), None, |_| async {
        Ok(())
    })
    .await
}

/// Whether one read of the entry works. Never the value.
pub async fn check_vault_entry(id: &str) -> Result<VaultCheck, ApiError> {
    vault_json::<(), _>(Method::POST, &vault_check_path(id), None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use bruhs_models::vault::{DUPLICATE_NAME, VAULT_FAILED, VAULT_UNREACHABLE};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    /// BC M2: a call still waiting at the deadline is dropped and fails as
    /// timed out, which the page shows as fixed copy; one that answered
    /// first is kept.
    #[test]
    fn a_vault_call_gives_up_at_its_deadline() {
        let mut context = Context::from_waker(Waker::noop());
        let aborted = std::cell::Cell::new(false);
        let late = pin!(bounded(
            std::future::pending::<Result<(), ApiError>>(),
            std::future::ready(()),
            || aborted.set(true),
        ));
        let Poll::Ready(result) = late.poll(&mut context) else {
            panic!("the deadline ends the call");
        };
        let error = result.unwrap_err();
        assert_eq!(error, ApiError::Timeout { after_ms: 75_000 });
        assert!(aborted.get(), "the request is cancelled");
        assert_eq!(vault_message(&error), VAULT_UNREACHABLE);

        let aborted = std::cell::Cell::new(false);
        let answered = pin!(bounded(
            std::future::ready(Ok(7)),
            std::future::pending::<()>(),
            || aborted.set(true),
        ));
        assert_eq!(answered.poll(&mut context), Poll::Ready(Ok(7)));
        assert!(!aborted.get());
    }

    /// V Task 14 minor: the desktop's rule, not any status's detail.
    #[test]
    fn only_the_vault_routes_fixed_refusals_are_shown() {
        let refused = ApiError::http(
            409,
            &serde_json::json!({ "error": DUPLICATE_NAME }).to_string(),
        );
        assert_eq!(vault_message(&refused), DUPLICATE_NAME);
        let internal = ApiError::http(500, r#"{"error":"snapshot failed: /private/path"}"#);
        assert_eq!(vault_message(&internal), VAULT_FAILED);
        assert_eq!(vault_message(&ApiError::http(503, "")), VAULT_FAILED);
        assert_eq!(
            vault_message(&ApiError::Network("TypeError".into())),
            VAULT_UNREACHABLE
        );
        assert_eq!(vault_message(&ApiError::Decode("x".into())), VAULT_FAILED);
    }

    #[test]
    fn vault_urls_escape_ids() {
        assert_eq!(vault_entry_path("v1 a"), "/api/vault/v1%20a");
        assert_eq!(vault_entry_path("a/b?c"), "/api/vault/a%2Fb%3Fc");
        assert_eq!(vault_check_path("x"), "/api/vault/x/check");
    }
}
