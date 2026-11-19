//! The vault routes (D22) for Settings, Vault. Values leave only in the
//! write-only create and update bodies and never come back: every answer is
//! metadata. The browser keeps none of it beyond the page.

use super::{get_json, segment, send, send_json, ApiError, NO_BODY};
use bruhs_models::vault::{VaultCheck, VaultCreate, VaultEntry, VaultListing, VaultUpdate};
use gloo_net::http::Method;

const VAULT_PATH: &str = "/api/vault";

fn vault_entry_path(id: &str) -> String {
    format!("{VAULT_PATH}/{}", segment(id))
}

fn vault_check_path(id: &str) -> String {
    format!("{}/check", vault_entry_path(id))
}

/// Every entry's metadata, the bruhs' names and whether `op` was found.
/// An older server answers 404.
pub async fn vault_listing() -> Result<VaultListing, ApiError> {
    get_json(VAULT_PATH).await
}

pub async fn create_vault_entry(create: &VaultCreate) -> Result<VaultEntry, ApiError> {
    send_json(Method::POST, VAULT_PATH, Some(create)).await
}

pub async fn update_vault_entry(id: &str, update: &VaultUpdate) -> Result<VaultEntry, ApiError> {
    send_json(Method::PUT, &vault_entry_path(id), Some(update)).await
}

pub async fn delete_vault_entry(id: &str) -> Result<(), ApiError> {
    send(Method::DELETE, &vault_entry_path(id), NO_BODY)
        .await
        .map(drop)
}

/// Whether one read of the entry works. Never the value.
pub async fn check_vault_entry(id: &str) -> Result<VaultCheck, ApiError> {
    send_json(Method::POST, &vault_check_path(id), NO_BODY).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_urls_escape_ids() {
        assert_eq!(vault_entry_path("v1 a"), "/api/vault/v1%20a");
        assert_eq!(vault_entry_path("a/b?c"), "/api/vault/a%2Fb%3Fc");
        assert_eq!(vault_check_path("x"), "/api/vault/x/check");
    }
}
