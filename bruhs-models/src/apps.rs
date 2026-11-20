//! Apps: the services a bruh may call with the owner's own token (spec §7).
//!
//! - The catalog ([`CONNECTORS`]): the fourteen connectors, their hosts, auth
//!   forms, per-account values, test calls and guides (D3, D4).
//! - The rules ([`classify`]): the call a bruh writes ([`AppCall`]), its limits,
//!   path normalizing, and the allowlist that refuses every endpoint an app
//!   doesn't name and classifies the rest as a read, write, send or delivery
//!   (D5, D6, D12, F4, ruling A).
//!
//! Nothing here holds a token: tokens live in the vault and core adds them
//! when it sends a call. Model-facing text (refusals, guides) is not app copy.

mod catalog;
mod guides;
mod page;
mod poll;
mod rules;
pub use catalog::*;
pub use page::*;
pub use poll::*;
pub use rules::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// What an app call does, mapped one to one to the crew gate's actions (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAction {
    /// Reads and changes nothing.
    Read,
    /// Changes the owner's data in the app.
    Write,
    /// Others see it as coming from the owner: a message, comment, email,
    /// invite, payment or refund.
    SendAsOwner,
    /// Changes code on a forge: a merge, a pull request, a pushed file.
    Delivery,
}

/// What a bruh gets back from an app call (D14). Any status is returned, so
/// the bruh can read an app's error. Secrets are already scrubbed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppResponse {
    pub status: u16,
    /// Only the [`RESPONSE_HEADERS`], lower case.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON when the body parses, else a string.
    pub body: Value,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// Limits (D12 to D14).
/// The largest body a call may send, as compact JSON.
pub const APP_BODY_MAX_BYTES: usize = 16 * 1024;
/// The most of a reply a bruh gets back.
pub const APP_RESPONSE_MAX_BYTES: usize = 32 * 1024;
/// The most of a reply Bruhs reads; more ends the call.
pub const APP_READ_MAX_BYTES: usize = 1024 * 1024;
/// One call's whole budget, connect to last byte.
pub const APP_CALL_TIMEOUT_MS: u64 = 30_000;
pub const MAX_QUERY_PARAMS: usize = 50;
/// Each query key and value, as text.
pub const MAX_QUERY_ITEM_BYTES: usize = 1024;
/// The whole query as compact JSON.
pub const MAX_QUERY_BYTES: usize = 4 * 1024;
pub const MAX_PATH_BYTES: usize = 1024;
/// The longest account name a call may give.
pub const MAX_ACCOUNT_CHARS: usize = 64;
/// App calls that may run at once.
pub const MAX_APP_CALLS: usize = 4;
/// The reply headers a bruh sees; every other header is dropped.
pub const RESPONSE_HEADERS: [&str; 6] = [
    "content-type",
    "link",
    "location",
    "retry-after",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
];

/// What a scrubbed secret reads as in a reply (D14).
pub const REDACTED: &str = "[redacted]";

// Model-facing copy.
pub const APP_METHOD: &str = "Use GET, POST, PUT, PATCH or DELETE.";
pub const APP_PATH: &str =
    "Use the API path after the app's base address, starting with /, with no host, query or fragment.";
pub const APP_PATH_ESCAPE: &str =
    "That path hides a separator in an escape. Write the path plainly.";
pub const APP_QUERY_TOO_LARGE: &str = "Send at most 50 query parameters, 4 KB in all.";
pub const APP_QUERY_VALUES: &str =
    "Query values are strings, numbers or booleans, each under 1 KB.";
/// A query that tries to change the method the gate saw (F4).
pub const APP_QUERY_METHOD: &str = "Pass the method as method, not in the query.";
pub const APP_GET_BODY: &str = "GET and DELETE calls take no body.";
pub const APP_BODY_TOO_LARGE: &str = "Keep the body under 16 KB.";
/// Stripe reads form fields, not JSON, so its calls carry no body.
pub const APP_QUERY_ONLY: &str =
    "Stripe takes no body. Pass every parameter in query, nested ones as metadata[key].";
pub const UNKNOWN_APP: &str = "There is no such app. list_apps lists the ones you may use.";
/// A GraphQL call with another method or a URL query (ruling A, C2).
pub const APP_GRAPHQL_SHAPE: &str =
    "Send GraphQL as a POST with the document in the body and no URL query.";
/// A GraphQL document that selects or names a field the app's rule refuses:
/// one that makes, reads or changes a token, key, secret or webhook (final
/// review A1 I2).
pub const APP_CREDENTIAL_REFUSED: &str =
    "Bruhs doesn't let a bruh make, read or change app credentials, keys or webhooks. Ask the owner.";
/// A GitHub GraphQL mutation: GitHub writes go by REST, where each one has
/// its own class (final review A1 I1).
pub const APP_GITHUB_GRAPHQL_WRITES: &str =
    "Bruhs sends GitHub GraphQL queries only. Use the REST endpoints for GitHub writes.";
pub const APP_ACCOUNT_SHAPE: &str =
    "Name the account in 1 to 64 characters, with no control characters.";
/// A field an allowlisted endpoint takes but a bruh may not send, such as a
/// Stripe customer's balance.
pub const APP_FIELD_REFUSED: &str =
    "Bruhs doesn't let a bruh send that field to that endpoint. Ask the owner.";
pub const APP_CUT_NOTE: &str = "Cut at 32 KB. Ask for less: fewer fields or a smaller page.";
pub const APP_TOO_LARGE: &str =
    "The app's answer is over 1 MB. Ask for less: fewer fields or a smaller page.";
pub const APP_NOT_TEXT: &str = "The app answered with something that isn't text.";
pub const APP_UNREACHABLE: &str =
    "Bruhs couldn't reach the app. Try again later, and tell the owner if it keeps failing.";
pub const APP_TIMED_OUT: &str =
    "The app didn't answer in 30 seconds. A write may still have landed: read before trying again.";
/// A write whose answer failed after it was sent: a dropped connection, a
/// broken, oversized or non-text reply. The app may have acted on it.
pub const APP_MAY_HAVE_LANDED: &str =
    "The app may have made that change. Read before trying again.";
/// Every call slot stayed busy until the deadline, so nothing was sent.
pub const APP_BUSY: &str =
    "Bruhs sent nothing: too many app calls are running. Try again in a moment.";
pub const APP_HOST_REFUSED: &str =
    "That app's address leads somewhere Bruhs doesn't call. Tell the owner.";

// App copy for an account's values (D4, D21).
pub const PARAM_TOKEN: &str = "Token";
pub const PARAM_KEY: &str = "API key";
pub const PARAM_EMAIL: &str = "Email";
pub const PARAM_SITE: &str = "Site";
pub const PARAM_SHOP: &str = "Shop";
pub const SITE_HINT: &str = "The name before .atlassian.net.";
pub const SHOP_HINT: &str = "The name before .myshopify.com.";
pub const ENTER_TOKEN: &str = "Paste the token.";
pub const TOKEN_SHAPE: &str =
    "Tokens are one line of 8 characters to 4 KB, with no spaces or quotes.";
pub const SITE_SHAPE: &str = "Use one name of letters, digits and hyphens.";
pub const EMAIL_SHAPE: &str = "Enter the account's email address.";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn app_actions_and_replies_keep_their_wire_names() {
        assert_eq!(json!(AppAction::SendAsOwner), json!("send_as_owner"));
        assert_eq!(
            serde_json::from_value::<AppAction>(json!("delivery")).unwrap(),
            AppAction::Delivery
        );
        let reply = AppResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: json!({"ok": true}),
            truncated: false,
            note: None,
        };
        // The quiet fields stay off the wire, and an older reply still reads.
        assert_eq!(
            json!(reply),
            json!({"status": 200, "headers": {}, "body": {"ok": true}})
        );
        let old: AppResponse =
            serde_json::from_value(json!({"status": 404, "body": "gone"})).unwrap();
        assert_eq!((old.status, old.truncated, old.note), (404, false, None));
        assert!(RESPONSE_HEADERS
            .iter()
            .all(|h| *h == h.to_ascii_lowercase()));
        assert!(!RESPONSE_HEADERS.contains(&"set-cookie"));
    }
}
