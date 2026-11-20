//! The Apps routes (D21) for Settings, Apps. A token leaves only in the
//! write-only connect and replace bodies and never comes back: every answer
//! is metadata. Disconnect is the vault's delete, which also takes the
//! account out of every bruh's grants. The browser keeps none of it beyond
//! the page.

use super::vault::vault_json;
use super::{segment, ApiError};
use bruhs_models::apps::{AppAccount, AppConnect, AppReplace, AppTestResult, AppsListing};
use gloo_net::http::Method;

const APPS_PATH: &str = "/api/apps";
const ACCOUNTS_PATH: &str = "/api/apps/accounts";

fn app_account_path(id: &str) -> String {
    format!("{ACCOUNTS_PATH}/{}", segment(id))
}

fn app_test_path(id: &str) -> String {
    format!("{}/test", app_account_path(id))
}

// Each call is bounded as the vault's are: a test call or a keychain prompt
// can hold the server, and the page's buttons must come back.

/// Every app account and mailbox, value free, with each bruh's grants and
/// the server's protocol. An older server answers 404.
pub async fn apps_listing() -> Result<AppsListing, ApiError> {
    vault_json::<(), _>(Method::GET, APPS_PATH, None).await
}

/// Tests the token, then saves the account. A failed test saves nothing
/// and answers 422 with its copy.
pub async fn connect_app(connect: &AppConnect) -> Result<AppAccount, ApiError> {
    vault_json(Method::POST, ACCOUNTS_PATH, Some(connect)).await
}

/// A new token for the same account, once it passes the test call.
pub async fn replace_app_token(id: &str, replace: &AppReplace) -> Result<AppAccount, ApiError> {
    vault_json(Method::PUT, &app_account_path(id), Some(replace)).await
}

/// The saved token's test call. A refused token is a 200 with `ok: false`,
/// so read `ok`, not the status.
pub async fn test_app_account(id: &str) -> Result<AppTestResult, ApiError> {
    vault_json::<(), _>(Method::POST, &app_test_path(id), None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_urls_escape_ids() {
        assert_eq!(app_account_path("v1 a"), "/api/apps/accounts/v1%20a");
        assert_eq!(app_account_path("a/b?c"), "/api/apps/accounts/a%2Fb%3Fc");
        assert_eq!(app_test_path("x"), "/api/apps/accounts/x/test");
    }
}
