//! Settings, Apps (D21): the wire types both apps and the server share, the
//! rules that read a token's test call, and the page's copy.
//!
//! The write-only bodies ([`AppConnect`], [`AppReplace`]) hold a token, so
//! they derive neither `Debug` nor `Clone`. Every other type here is value
//! free.

use super::AppTest;
use crate::connections::ServiceProvider;
use crate::secrets::{review_text, SavedCredentialState};
use crate::vault::{CredentialScope, VaultAttention, MAX_APP_ACCOUNT_CHARS, MAX_NAME_CHARS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// Page copy.
pub const APPS_TITLE: &str = "Apps";
pub const APPS_DESCRIPTION: &str = "Connect apps with a token. A bruh you grant an app calls it through Bruhs, which keeps the token in your OS keychain and never shows it to the bruh.";
pub const CONNECT: &str = "Connect";
pub const TEST: &str = "Test";
pub const REPLACE_TOKEN: &str = "Replace token";
pub const DISCONNECT: &str = "Disconnect";
pub const TEST_AND_CONNECT: &str = "Test and connect";
pub const GET_TOKEN: &str = "Get a token";
pub const SCOPE_SHARED: &str = "Bruhs you grant it to";
pub const MAIL_GROUP: &str = "Mail";
pub const MAIL_HINT: &str = "Gmail and Outlook accounts connect in Settings, Vault. Grant one to a bruh so it can send email from it.";
pub const NO_MAILBOXES: &str = "No mail accounts connected.";
pub const NO_BRUHS_FOR_APPS: &str = "Make a bruh to grant it apps.";
pub const GRANTS_LABEL: &str = "Bruhs";
pub const APPS_LOADING: &str = "Loading apps.";
/// A replacement token whose test call names another account (ruling F24).
pub const ANOTHER_ACCOUNT: &str =
    "That token belongs to another account. Connect it as a new account.";
/// The same app, account and scope connected twice (ruling F24).
pub const ACCOUNT_CONNECTED: &str = "That account is already connected. Replace its token instead.";
/// The mail cache couldn't list its mailboxes.
pub const MAILBOXES_UNAVAILABLE: &str = "Bruhs couldn't read your mail accounts. Try again.";

/// One connected app account, value free (`GET /api/apps`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppAccount {
    /// The vault entry's id.
    pub id: String,
    /// The connector id.
    pub app: String,
    pub name: String,
    /// The label the token's test call named.
    pub account: String,
    pub scope: CredentialScope,
    pub state: SavedCredentialState,
    #[serde(default)]
    pub attention: Option<VaultAttention>,
}

/// A connected Gmail or Outlook mailbox a bruh may be granted (D18).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppMailbox {
    pub mailbox_id: String,
    pub provider: ServiceProvider,
    pub account: String,
}

/// A bruh with its app and mail grants, and why it can't use them now.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppsBruh {
    pub agent_id: String,
    pub name: String,
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub mail: Vec<String>,
    #[serde(default)]
    pub unavailable: Option<String>,
}

/// `GET /api/apps`. No `deny_unknown_fields`: a newer server may add more.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct AppsListing {
    #[serde(default)]
    pub accounts: Vec<AppAccount>,
    #[serde(default)]
    pub mailboxes: Vec<AppMailbox>,
    #[serde(default)]
    pub bruhs: Vec<AppsBruh>,
}

/// `POST /api/apps/accounts`. Write-only: deliberately no `Debug` or
/// `Clone`. `params` are the app's non-secret values; `key` is Trello's.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConnect {
    pub app: String,
    #[serde(default)]
    pub name: Option<String>,
    pub scope: CredentialScope,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    pub token: String,
    #[serde(default)]
    pub key: Option<String>,
}

/// `PUT /api/apps/accounts/{id}`. Write-only: deliberately no `Debug` or
/// `Clone`. A missing `key` keeps the stored one.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppReplace {
    pub token: String,
    #[serde(default)]
    pub key: Option<String>,
}

/// What a token's test call showed. Never holds the token.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppTestResult {
    pub ok: bool,
    #[serde(default)]
    pub account: Option<String>,
    pub message: String,
}

/// Why a token's test call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFailure {
    /// The app said no: 401 or 403, or an `ok` of `false`.
    Refused,
    /// The call never got an answer.
    Unreachable,
    /// Any other answer: another status, or no account in it.
    Unexpected,
}

/// The owner copy for a failed test call.
#[must_use]
pub fn test_failure_message(app_name: &str, failure: TestFailure) -> String {
    match failure {
        TestFailure::Refused => {
            format!("{app_name} refused the token. Check it and what it may do.")
        }
        TestFailure::Unreachable => {
            format!("Bruhs couldn't reach {app_name}. Check your connection and try again.")
        }
        TestFailure::Unexpected => format!(
            "{app_name} answered in a way Bruhs didn't expect. Check the token's account and try again."
        ),
    }
}

#[must_use]
pub fn connected_as(account: &str) -> String {
    format!("Connected as {account}.")
}

/// An account's name when the owner gives none: "{App} {account}", cut to
/// [`MAX_NAME_CHARS`] so a long label never fails the save (ruling F24).
#[must_use]
pub fn default_account_name(app_name: &str, account: &str) -> String {
    let name: String = format!("{app_name} {account}")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    name.trim_end().to_owned()
}

#[must_use]
pub fn disconnect_confirm(name: &str) -> String {
    format!("Disconnect {name}? Bruhs lose it, and routines that watch it stop.")
}

/// A bruh's own app account, in its Settings (D22).
#[must_use]
pub fn own_app_line(name: &str) -> String {
    format!("{name}: its own")
}

/// A mailbox's account as the page and `list_apps` show it: its label, or
/// "{provider} account" when the cache has none (ruling F14).
#[must_use]
pub fn mailbox_account(provider: ServiceProvider, label: &str) -> String {
    let label = review_text(label.trim());
    if label.is_empty() {
        format!("{} account", provider.name())
    } else {
        label
    }
}

/// An account label as stored: shown as [`review_text`] shows it, cut to
/// [`MAX_APP_ACCOUNT_CHARS`] and trimmed (ruling F24). `None` when nothing
/// is left.
#[must_use]
pub fn account_label(raw: &str) -> Option<String> {
    let shown: String = review_text(raw.trim())
        .chars()
        .take(MAX_APP_ACCOUNT_CHARS)
        .collect();
    let shown = shown.trim();
    (!shown.is_empty()).then(|| shown.to_owned())
}

/// Reads a test call's answer: 401 and 403 are `Refused`; any other non-2xx
/// is `Unexpected`; an `ok` pointer holding `false` is `Refused` (Slack
/// answers 200 on a bad token) and one holding anything but `true` is
/// `Unexpected`; then the first account pointer holding a string or number
/// names the account, or it is `Unexpected`.
pub fn test_account(test: &AppTest, status: u16, body: &Value) -> Result<String, TestFailure> {
    match status {
        401 | 403 => return Err(TestFailure::Refused),
        200..=299 => {}
        _ => return Err(TestFailure::Unexpected),
    }
    if let Some(pointer) = test.ok {
        match body.pointer(pointer) {
            Some(Value::Bool(true)) => {}
            Some(Value::Bool(false)) => return Err(TestFailure::Refused),
            _ => return Err(TestFailure::Unexpected),
        }
    }
    test.account
        .iter()
        .find_map(|pointer| match body.pointer(pointer)? {
            Value::String(text) => account_label(text),
            Value::Number(number) => account_label(&number.to_string()),
            _ => None,
        })
        .ok_or(TestFailure::Unexpected)
}

#[cfg(test)]
mod tests {
    use super::super::connector;
    use super::*;
    use serde_json::json;

    #[test]
    fn test_calls_name_the_account_or_say_why_not() {
        let github = connector("github").unwrap().test;
        assert_eq!(
            test_account(&github, 200, &json!({"login": "octo"})),
            Ok("octo".into())
        );
        assert_eq!(
            test_account(&github, 401, &json!({})),
            Err(TestFailure::Refused)
        );
        assert_eq!(
            test_account(&github, 403, &json!({"login": "octo"})),
            Err(TestFailure::Refused)
        );
        assert_eq!(
            test_account(&github, 500, &json!({"login": "octo"})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            test_account(&github, 200, &json!({"login": ""})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            test_account(&github, 200, &json!("not json")),
            Err(TestFailure::Unexpected)
        );
        // Slack answers 200 with ok false on a bad token.
        let slack = connector("slack").unwrap().test;
        assert_eq!(
            test_account(&slack, 200, &json!({"ok": false, "user": "me"})),
            Err(TestFailure::Refused)
        );
        assert_eq!(
            test_account(&slack, 200, &json!({"user": "me"})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            test_account(&slack, 200, &json!({"ok": true, "user": "me"})),
            Ok("me".into())
        );
        // The first pointer that holds a value wins; a number counts.
        let airtable = connector("airtable").unwrap().test;
        assert_eq!(
            test_account(&airtable, 200, &json!({"id": "usr1"})),
            Ok("usr1".into())
        );
        let hubspot = connector("hubspot").unwrap().test;
        assert_eq!(
            test_account(&hubspot, 200, &json!({"portalId": 42})),
            Ok("42".into())
        );
    }

    #[test]
    fn account_labels_show_as_stored_and_fit() {
        assert_eq!(account_label("  octo "), Some("octo".into()));
        assert_eq!(account_label("   "), None);
        assert_eq!(account_label("a\u{202e}b"), Some("a\\u{202e}b".into()));
        let long = "x".repeat(MAX_APP_ACCOUNT_CHARS + 10);
        assert_eq!(
            account_label(&long).map(|label| label.chars().count()),
            Some(MAX_APP_ACCOUNT_CHARS)
        );
        let name = default_account_name("GitHub", &"y".repeat(MAX_APP_ACCOUNT_CHARS));
        assert_eq!(name.chars().count(), 7 + MAX_APP_ACCOUNT_CHARS);
        assert!(name.chars().count() <= MAX_NAME_CHARS);
        assert_eq!(
            default_account_name(&"N".repeat(78), "ab cd")
                .chars()
                .count(),
            MAX_NAME_CHARS
        );
        // A cut that lands on a space leaves no trailing space.
        assert_eq!(default_account_name(&"N".repeat(78), " b"), "N".repeat(78));
        assert_eq!(default_account_name("GitHub", "octo"), "GitHub octo");
    }

    #[test]
    fn the_page_copy_reads_as_written() {
        assert_eq!(
            test_failure_message("GitHub", TestFailure::Refused),
            "GitHub refused the token. Check it and what it may do."
        );
        assert_eq!(
            test_failure_message("Slack", TestFailure::Unreachable),
            "Bruhs couldn't reach Slack. Check your connection and try again."
        );
        assert_eq!(
            test_failure_message("Jira", TestFailure::Unexpected),
            "Jira answered in a way Bruhs didn't expect. Check the token's account and try again."
        );
        assert_eq!(connected_as("octo"), "Connected as octo.");
        assert_eq!(
            disconnect_confirm("GitHub octo"),
            "Disconnect GitHub octo? Bruhs lose it, and routines that watch it stop."
        );
        assert_eq!(own_app_line("GitHub octo"), "GitHub octo: its own");
        assert_eq!(mailbox_account(ServiceProvider::Gmail, ""), "Gmail account");
        assert_eq!(
            mailbox_account(ServiceProvider::Outlook, " me@x.test "),
            "me@x.test"
        );
        for copy in [
            APPS_DESCRIPTION,
            MAIL_HINT,
            ANOTHER_ACCOUNT,
            ACCOUNT_CONNECTED,
            MAILBOXES_UNAVAILABLE,
        ] {
            assert!(
                !copy.contains(" - ") && !copy.contains('\u{2014}') && !copy.contains('\u{b7}'),
                "{copy}"
            );
        }
    }

    #[test]
    fn the_wire_reads_old_and_new_and_refuses_strays() {
        let listing: AppsListing = serde_json::from_value(json!({"newer": 1})).unwrap();
        assert_eq!(listing, AppsListing::default());
        let account: AppAccount = serde_json::from_value(json!({
            "id": "v1-a",
            "app": "github",
            "name": "GitHub octo",
            "account": "octo",
            "scope": {"type": "global"},
            "state": "saved",
        }))
        .unwrap();
        assert_eq!(account.attention, None);
        let bruh: AppsBruh =
            serde_json::from_value(json!({"agent_id": "a", "name": "Ann"})).unwrap();
        assert!(bruh.apps.is_empty() && bruh.mail.is_empty() && bruh.unavailable.is_none());
        let connect: AppConnect = serde_json::from_value(json!({
            "app": "jira",
            "scope": {"type": "global"},
            "params": {"site": "acme", "email": "me@acme.test"},
            "token": "tok12345678",
        }))
        .unwrap();
        assert_eq!((connect.name, connect.key), (None, None));
        assert!(serde_json::from_value::<AppConnect>(json!({
            "app": "github",
            "scope": {"type": "global"},
            "token": "tok12345678",
            "host": "evil.test",
        }))
        .is_err());
        assert!(serde_json::from_value::<AppReplace>(json!({
            "token": "tok12345678",
            "account": "other",
        }))
        .is_err());
        let result = AppTestResult {
            ok: false,
            account: None,
            message: "m".into(),
        };
        assert_eq!(
            json!(result),
            json!({"ok": false, "account": null, "message": "m"})
        );
    }
}
