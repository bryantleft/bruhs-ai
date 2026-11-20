//! Settings, Apps (D21): the wire types both apps and the server share, the
//! rules that read a token's test call, and the page's copy.
//!
//! The write-only bodies ([`AppConnect`], [`AppReplace`]) hold a token, so
//! they derive neither `Debug` nor `Clone`. Every other type here is value
//! free.

use super::{AccountTenant, AppConnector, REDACTED};
use crate::connections::ServiceProvider;
use crate::secrets::{review_text, SavedCredentialState};
use crate::vault::{
    server_port, CredentialScope, VaultAttention, VaultBusy, MAX_APP_ACCOUNT_CHARS, MAX_NAME_CHARS,
    WAIT_FOR_CHANGE, WAIT_FOR_CHECK,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

mod form;
pub use form::*;

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
/// Under a connect form's Name.
pub const APP_NAME_HINT: &str = "Leave it empty to name it after the app and account.";
/// Under a Replace token form's API key.
pub const KEEP_KEY_HINT: &str = "Leave the API key empty to keep it.";
/// A replacement token whose test call names another account (ruling F24).
pub const ANOTHER_ACCOUNT: &str =
    "That token belongs to another account. Connect it as a new account.";
/// The same app, account and scope connected twice (ruling F24).
pub const ACCOUNT_CONNECTED: &str = "That account is already connected. Replace its token instead.";
/// The mail cache couldn't list its mailboxes.
pub const MAILBOXES_UNAVAILABLE: &str = "Bruhs couldn't read your mail accounts. Try again.";
/// A connect or replace body that isn't one: never serde's text, which can
/// quote the token.
pub const INVALID_APP_ACCOUNT: &str = "Invalid app account.";

/// The listing refused to a page on another address (D23).
/// [`open_apps_from_own_address`] names the address when it knows the port.
pub const OPEN_APPS_FROM_OWN_ADDRESS: &str =
    "Open Bruhs from its own address to see and change apps.";

/// [`OPEN_APPS_FROM_OWN_ADDRESS`] naming the exact address that may see
/// and change apps, as the vault's notice does: the server binds only
/// `127.0.0.1`. `server` is the client's own server URL.
#[must_use]
pub fn open_apps_from_own_address(server: &str) -> String {
    server_port(server).map_or_else(
        || OPEN_APPS_FROM_OWN_ADDRESS.to_owned(),
        |port| format!("Open Bruhs at http://127.0.0.1:{port} to see and change apps."),
    )
}

/// Why the page's buttons are disabled right now, if they are: the
/// listing loading, a test call, or a change.
#[must_use]
pub fn apps_wait_reason(busy: VaultBusy, loading: bool) -> Option<&'static str> {
    match busy {
        VaultBusy::Idle => None,
        _ if loading => Some(APPS_LOADING),
        VaultBusy::Checking => Some(WAIT_FOR_CHECK),
        VaultBusy::Changing => Some(WAIT_FOR_CHANGE),
    }
}

/// An account's Used by line: [`SCOPE_SHARED`], or "{bruh}'s own" by the
/// bruh's name when the listing has it.
#[must_use]
pub fn app_scope_line(scope: &CredentialScope, bruhs: &[AppsBruh]) -> String {
    match scope {
        CredentialScope::Global => SCOPE_SHARED.into(),
        CredentialScope::Bruh { agent_id } => {
            let name = bruhs
                .iter()
                .find(|bruh| &bruh.agent_id == agent_id)
                .map_or(agent_id.as_str(), |bruh| bruh.name.as_str());
            format!("{}'s own", review_text(name))
        }
        CredentialScope::Project { path } => format!("Project: {}", review_text(path)),
    }
}

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
    /// The crew protocol the server speaks (ruling F19): grant switches
    /// work only when `apps_ready(protocol)`. 0 from an older server.
    #[serde(default)]
    pub protocol: u32,
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

/// The order a bruh's Settings and a poll's choices list app accounts in:
/// by name ignoring case, then by name, then by id.
#[must_use]
pub fn account_order(a: &AppAccount, b: &AppAccount) -> std::cmp::Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.id.cmp(&b.id))
}

/// An app account's name as both apps show it, escaped.
#[must_use]
pub fn app_account_name(account: &AppAccount) -> String {
    review_text(&account.name)
}

/// Beside an account a write left unfinished: the listing keeps it, but
/// its token may not read until it is replaced.
pub const ACCOUNT_NEEDS_ATTENTION: &str = "needs attention";

/// `label` with the account's state when it isn't usable as saved:
/// "{label} (needs attention)".
#[must_use]
pub fn with_account_state(label: String, state: SavedCredentialState) -> String {
    match state {
        SavedCredentialState::Saved => label,
        SavedCredentialState::NeedsAttention => format!("{label} ({ACCOUNT_NEEDS_ATTENTION})"),
    }
}

/// How a shared app account reads beside its switch in a bruh's Settings:
/// "{name}, {App}", or the app id for an app this build doesn't know, with
/// its state when it needs attention.
#[must_use]
pub fn app_account_choice_label(account: &AppAccount) -> String {
    let app = super::connector(&account.app)
        .map_or_else(|| review_text(&account.app), |app| app.name.to_owned());
    with_account_state(
        format!("{}, {app}", app_account_name(account)),
        account.state,
    )
}

/// How a mailbox reads beside its switch or in a poll's account choice:
/// "{account}, {provider}", or [`mailbox_account`]'s "{provider} account"
/// alone when the cache has no label.
#[must_use]
pub fn mailbox_choice_label(mailbox: &AppMailbox) -> String {
    let account = mailbox_account(mailbox.provider, &mailbox.account);
    if review_text(mailbox.account.trim()).is_empty() {
        return account;
    }
    format!("{account}, {}", mailbox.provider.name())
}

/// An account label as stored: shown as [`review_text`] shows it, cut to
/// [`MAX_APP_ACCOUNT_CHARS`] and trimmed (ruling F24). `None` when nothing
/// is left.
#[must_use]
pub fn account_label(raw: &str) -> Option<String> {
    let shown = shown(raw);
    let cut = cut(&shown, MAX_APP_ACCOUNT_CHARS);
    (!cut.is_empty()).then_some(cut)
}

/// The account a test call named, as stored: its label with the tenant the
/// connector names (ruling A Task 8 I2). Jira is "{label} on {site}",
/// Shopify the shop, and Slack, Linear and Notion "{label} in {workspace}"
/// when the reply names one. A long label is cut to fit
/// [`MAX_APP_ACCOUNT_CHARS`], never the tenant; only a tenant too long to
/// leave room for any label stands alone, cut. `None` when the label is
/// empty, or when it or the tenant holds a scrubbed secret (an app that
/// echoes the token as the account).
#[must_use]
pub fn account_for(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
    label: &str,
    reply: &Value,
) -> Option<String> {
    let label = shown(label);
    if label.is_empty() || label.contains(REDACTED) {
        return None;
    }
    let (separator, tenant) = match connector.test.tenant {
        AccountTenant::None => return account_label(&label),
        AccountTenant::Param(id) => return params.get(id).and_then(|shop| account_label(shop)),
        AccountTenant::Site(id) => (" on ", params.get(id).map(|site| shown(site))?),
        AccountTenant::Workspace(pointer) => match reply.pointer(pointer) {
            Some(Value::String(name)) if !shown(name).is_empty() => (" in ", shown(name)),
            _ => return account_label(&label),
        },
    };
    if tenant.contains(REDACTED) {
        return None;
    }
    let room = MAX_APP_ACCOUNT_CHARS
        .saturating_sub(separator.chars().count())
        .saturating_sub(tenant.chars().count());
    let label = cut(&label, room);
    if label.is_empty() {
        return account_label(&tenant);
    }
    Some(format!("{label}{separator}{tenant}"))
}

/// Reads a test call's answer: 401 and 403 are `Refused`; any other non-2xx
/// is `Unexpected`; an `ok` pointer holding `false` is `Refused` (Slack
/// answers 200 on a bad token) and one holding anything but `true` is
/// `Unexpected`; then the first account pointer holding a string or number
/// names the account, stored as [`account_for`] makes it, or it is
/// `Unexpected`.
pub fn test_account(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
    status: u16,
    body: &Value,
) -> Result<String, TestFailure> {
    let test = &connector.test;
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
    let label = test
        .account
        .iter()
        .find_map(|pointer| match body.pointer(pointer)? {
            Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .ok_or(TestFailure::Unexpected)?;
    account_for(connector, params, &label, body).ok_or(TestFailure::Unexpected)
}

/// Text as [`review_text`] shows it, trimmed.
fn shown(raw: &str) -> String {
    review_text(raw.trim())
}

/// The first `chars` characters, trimmed at the end.
fn cut(text: &str, chars: usize) -> String {
    let cut: String = text.chars().take(chars).collect();
    cut.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::super::connector;
    use super::*;
    use serde_json::json;

    fn none() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn named(
        app: &str,
        params: &BTreeMap<String, String>,
        body: Value,
    ) -> Result<String, TestFailure> {
        test_account(connector(app).unwrap(), params, 200, &body)
    }

    #[test]
    fn test_calls_name_the_account_or_say_why_not() {
        let github = connector("github").unwrap();
        let none = none();
        assert_eq!(
            named("github", &none, json!({"login": "octo"})),
            Ok("octo".into())
        );
        assert_eq!(
            test_account(github, &none, 401, &json!({})),
            Err(TestFailure::Refused)
        );
        assert_eq!(
            test_account(github, &none, 403, &json!({"login": "octo"})),
            Err(TestFailure::Refused)
        );
        assert_eq!(
            test_account(github, &none, 500, &json!({"login": "octo"})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            named("github", &none, json!({"login": ""})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            named("github", &none, json!("not json")),
            Err(TestFailure::Unexpected)
        );
        // An app that echoes the token names a scrubbed account: unexpected.
        assert_eq!(
            named("github", &none, json!({"login": "[redacted]"})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            named("github", &none, json!({"login": "x [redacted] y"})),
            Err(TestFailure::Unexpected)
        );
        // Slack answers 200 with ok false on a bad token.
        assert_eq!(
            named("slack", &none, json!({"ok": false, "user": "me"})),
            Err(TestFailure::Refused)
        );
        assert_eq!(
            named("slack", &none, json!({"user": "me"})),
            Err(TestFailure::Unexpected)
        );
        assert_eq!(
            named("slack", &none, json!({"ok": true, "user": "me"})),
            Ok("me".into())
        );
        // The first pointer that holds a value wins; a number counts.
        assert_eq!(
            named("airtable", &none, json!({"email": " ", "id": "usr1"})),
            Ok("usr1".into())
        );
        assert_eq!(
            named("hubspot", &none, json!({"portalId": 42})),
            Ok("42".into())
        );
    }

    #[test]
    fn labels_name_their_site_shop_or_workspace() {
        let none = none();
        // Jira: one Atlassian email on two sites is two accounts.
        let acme = params(&[("site", "acme"), ("email", "me@x.test")]);
        let beta = params(&[("site", "beta"), ("email", "me@x.test")]);
        let me = json!({"emailAddress": "me@x.test"});
        assert_eq!(
            named("jira", &acme, me.clone()),
            Ok("me@x.test on acme".into())
        );
        assert_eq!(named("jira", &beta, me), Ok("me@x.test on beta".into()));
        // Shopify: the shop's DNS label, not its display name.
        assert_eq!(
            named(
                "shopify",
                &params(&[("shop", "acme-store")]),
                json!({"shop": {"name": "My Store"}})
            ),
            Ok("acme-store".into())
        );
        // Workspaces, where the reply names one.
        assert_eq!(
            named(
                "slack",
                &none,
                json!({"ok": true, "user": "ann", "team": "Acme"})
            ),
            Ok("ann in Acme".into())
        );
        assert_eq!(
            named(
                "linear",
                &none,
                json!({"data": {"viewer": {"email": "a@x.test", "organization": {"urlKey": "acme"}}}})
            ),
            Ok("a@x.test in acme".into())
        );
        assert_eq!(
            named(
                "notion",
                &none,
                json!({"name": "Bruhs", "bot": {"workspace_name": "Ann's Notion"}})
            ),
            Ok("Bruhs in Ann's Notion".into())
        );
        assert_eq!(
            named("notion", &none, json!({"name": "Bruhs"})),
            Ok("Bruhs".into())
        );
        // A workspace is shown as stored, and a scrubbed one is refused.
        assert_eq!(
            named(
                "slack",
                &none,
                json!({"ok": true, "user": "ann", "team": "A\u{202e}cme"})
            ),
            Ok("ann in A\\u{202e}cme".into())
        );
        assert_eq!(
            named(
                "slack",
                &none,
                json!({"ok": true, "user": "ann", "team": "A\u{202e}"})
            ),
            Ok("ann in A\\u{202e}".into())
        );
        assert_eq!(
            named(
                "slack",
                &none,
                json!({"ok": true, "user": "ann", "team": "[redacted]"})
            ),
            Err(TestFailure::Unexpected)
        );
    }

    #[test]
    fn a_long_label_is_cut_and_its_tenant_kept() {
        let site = "s".repeat(40);
        let jira = params(&[("site", &site), ("email", "me@x.test")]);
        let long = "e".repeat(100);
        let account = named("jira", &jira, json!({"emailAddress": long})).unwrap();
        assert_eq!(account.chars().count(), MAX_APP_ACCOUNT_CHARS);
        assert!(account.ends_with(&format!(" on {site}")), "{account}");
        assert!(account.starts_with("eee"));
        // A tenant that leaves no room for any label stands alone.
        let site = "s".repeat(63);
        let jira = params(&[("site", &site), ("email", "me@x.test")]);
        assert_eq!(
            named("jira", &jira, json!({"emailAddress": "me@x.test"})),
            Ok(site)
        );
        let team = "t".repeat(100);
        assert_eq!(
            named(
                "slack",
                &none(),
                json!({"ok": true, "user": "ann", "team": team})
            ),
            Ok("t".repeat(MAX_APP_ACCOUNT_CHARS))
        );
        // Every account fits the stored cap and shows as stored.
        for body in [
            json!({"emailAddress": "e".repeat(300)}),
            json!({"emailAddress": "a\u{7}\u{2028}b"}),
        ] {
            let account = named("jira", &jira, body).unwrap();
            assert!(account.chars().count() <= MAX_APP_ACCOUNT_CHARS);
            assert_eq!(review_text(&account), account);
        }
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
            APP_NAME_HINT,
            KEEP_KEY_HINT,
            OPEN_APPS_FROM_OWN_ADDRESS,
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

    #[test]
    fn the_page_says_who_uses_an_account_and_why_it_waits() {
        use crate::vault::{VaultBusy, WAIT_FOR_CHANGE, WAIT_FOR_CHECK};
        let bruhs = [AppsBruh {
            agent_id: "a".into(),
            name: "A\u{202e}da".into(),
            apps: Vec::new(),
            mail: Vec::new(),
            unavailable: None,
        }];
        assert_eq!(
            app_scope_line(&CredentialScope::Global, &bruhs),
            SCOPE_SHARED
        );
        assert_eq!(
            app_scope_line(
                &CredentialScope::Bruh {
                    agent_id: "a".into()
                },
                &bruhs
            ),
            "A\\u{202e}da's own"
        );
        assert_eq!(
            app_scope_line(
                &CredentialScope::Bruh {
                    agent_id: "gone".into()
                },
                &bruhs
            ),
            "gone's own"
        );
        assert_eq!(apps_wait_reason(VaultBusy::Idle, true), None);
        assert_eq!(
            apps_wait_reason(VaultBusy::Changing, true),
            Some(APPS_LOADING)
        );
        assert_eq!(
            apps_wait_reason(VaultBusy::Changing, false),
            Some(WAIT_FOR_CHANGE)
        );
        assert_eq!(
            apps_wait_reason(VaultBusy::Checking, false),
            Some(WAIT_FOR_CHECK)
        );
        // Owner-only pages name the exact address (rereview I1 of p4).
        assert_eq!(
            open_apps_from_own_address("http://127.0.0.1:3001"),
            "Open Bruhs at http://127.0.0.1:3001 to see and change apps."
        );
        assert_eq!(
            open_apps_from_own_address("https://bruhs.localhost"),
            OPEN_APPS_FROM_OWN_ADDRESS
        );
    }
}
