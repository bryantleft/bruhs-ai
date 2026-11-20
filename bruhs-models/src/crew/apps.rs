//! Apps on a bruh: which app accounts and mailboxes it may use, the
//! choices its Settings offer and protocol 5 (D9, D10, D23); the app and
//! mail tools, their gate and their input (D2, D7, D12); and the copy both
//! apps and the bruh see. The app tokens themselves are vault entries of
//! kind [`VaultKind::AppToken`]; the connectors live in [`crate::apps`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::vault::{entry_order, grant_id_ok, grant_set};
use super::{
    effective_level, gate, service_notice, AccessLevel, BruhAccess, Gate, GatedAction,
    APPS_PROTOCOL,
};
use crate::agent::AgentProvider;
use crate::apps::{
    classify, connector, normalize_path, validate_call, AppAction, AppCall, UNKNOWN_APP,
};
use crate::mail::{MailComposeContent, MailComposeError, MailReplyTarget};
use crate::secrets::review_text;
use crate::vault::{CredentialScope, VaultEntry, VaultKind};

/// The app and mail tools of a bruh run, on the `bruhs_workbench` helper
/// (D1).
pub const APP_REQUEST_TOOL: &str = "mcp__bruhs_workbench__app_request";
pub const LIST_APPS_TOOL: &str = "mcp__bruhs_workbench__list_apps";
pub const SEND_MAIL_TOOL: &str = "mcp__bruhs_workbench__send_mail";

/// Model-facing tool descriptions.
pub const APP_REQUEST_DESCRIPTION: &str = "Call an app the owner connected, as the owner. app is an id from list_apps; account picks one when you have several; method is GET, POST, PUT, PATCH or DELETE; path is the API path after the app's base address, starting with /; query is an object of query parameters; body is a JSON body. Bruhs adds the token; you never see it. Reads run at once; writes and anything sent in the owner's name follow your access. Answers are cut at 32 KB.";
pub const LIST_APPS_DESCRIPTION: &str = "List the apps and mail accounts you may use. With app, say how to call that app: its base address, common calls, paging and what needs approval.";
pub const SEND_MAIL_DESCRIPTION: &str = "Send an email as the owner from a connected Gmail or Outlook account. from names the account (list_apps lists them); to, cc and bcc take comma separated addresses; body is plain text. To reply, pass reply with external_thread_id and message_id from a new mail event and leave subject out. Each send follows your access and Send as me.";

/// The longest plain text body `send_mail` sends.
pub const MAIL_BODY_MAX_BYTES: usize = 8 * 1024;
/// The most people one `send_mail` reaches, To, Cc and Bcc together.
pub const MAIL_MAX_RECIPIENTS: usize = 20;
/// The longest subject, as RFC 5322 caps a line.
pub const MAIL_SUBJECT_MAX_BYTES: usize = 998;

/// App copy too: shown beside a bruh's app switches when it can't use apps.
pub const APPS_NEED_ASK: &str = "Apps need a Claude bruh at Ask first or Full.";

// Model-facing copy.
pub const APP_NOT_GRANTED: &str =
    "That app isn't granted to you. list_apps lists the ones you may use.";
pub const APP_ACCOUNT_NEEDED: &str =
    "You may use more than one account for that app. Name one with account; list_apps lists them.";
pub const APP_ACCOUNT_UNKNOWN: &str =
    "You have no account by that name for that app. list_apps lists them.";
pub const MAIL_NOT_GRANTED: &str =
    "You may not send from that account. list_apps lists the ones you may use.";
pub const MAIL_FROM_NEEDED: &str = "Name the account to send from with from; list_apps lists them.";
pub const MAIL_REPLY_SUBJECT: &str =
    "Leave subject out when replying; the reply keeps the thread's.";
pub const MAIL_SUBJECT_NEEDED: &str = "Give the email a subject.";
/// A subject over [`MAIL_SUBJECT_MAX_BYTES`] or with a line break.
pub const MAIL_SUBJECT_SHAPE: &str = "Keep the subject to one line of at most 998 bytes.";
pub const MAIL_BODY_SIZE: &str = "Keep the body between 1 byte and 8 KB.";
/// A body with control characters other than line breaks and tabs.
pub const MAIL_BODY_TEXT: &str =
    "Write the body as plain text, with no control characters but line breaks and tabs.";
/// To without an address, or a recipient that isn't an email address.
pub const MAIL_RECIPIENTS: &str =
    "Put at least one email address in to, and separate addresses with commas.";
pub const MAIL_TOO_MANY: &str = "Send to at most 20 people in all.";
/// A helper call with no allowance for its tool and input (D15).
pub const CALL_NOT_APPROVED: &str = "This call wasn't approved.";
/// A helper call that came from a page, not the bruh's own tools (F26).
pub const CALL_FROM_A_PAGE: &str = "Only the bruh's own tools can send its calls.";
pub const APP_CALL_SHAPE: &str =
    "Pass app, method and path, and query or body only as the tool describes.";
/// A `send_mail` input that doesn't parse as [`MailSendInput`].
pub const MAIL_CALL_SHAPE: &str =
    "Pass to and body, and from, cc, bcc, subject or reply only as the tool describes.";

// The run preamble's app and mail lines (D19).
/// Follows "Apps you may use: {names}." when apps are offered.
pub const APPS_PREAMBLE: &str = "list_apps names their accounts, and with app it tells you how to call one; app_request calls it. Bruhs adds the token; never ask for one.";
/// The send line when no app or mail tool is offered.
pub const NEVER_SEND_PREAMBLE: &str = "Never send messages in the owner's name.";
/// The send line at Ask with apps or mail.
pub const SEND_ASK_PREAMBLE: &str = "Each write, email or post in the owner's name waits for the owner's approval; if one is denied, continue without it and say so.";
/// The send line at Full without Send as me.
pub const SEND_APPROVAL_PREAMBLE: &str =
    "Writes run without asking; email and posts in the owner's name wait for the owner's approval.";
/// The send line at Full with Send as me.
pub const SEND_AS_ME_PREAMBLE: &str = "You may send email and post in the owner's name without asking. Do it only when your job or the owner asks for it.";
/// The preamble names at most this many apps and mailboxes, each.
pub const PREAMBLE_MAX_NAMES: usize = 20;

/// A bruh holds at most this many app account grants.
pub const MAX_APP_GRANTS: usize = 64;
/// A bruh holds at most this many mailbox grants.
pub const MAX_MAIL_GRANTS: usize = 16;

/// Refused unless the request comes from the owner's own app (D23).
pub const OWNER_ONLY_APPS: &str = "Open Bruhs from its own address to change apps.";
const NOT_AN_APP_ACCOUNT: &str = "That isn't an app account.";
const TOO_MANY_APP_GRANTS: &str = "Grant at most 64 app accounts.";
const TOO_MANY_MAIL_GRANTS: &str = "Grant at most 16 mail accounts.";

/// One grant `SetAppGrant` turns on or off: a Global app token by its
/// vault entry id, or a connected mailbox by its id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppGrant {
    Token { entry_id: String },
    Mailbox { mailbox_id: String },
}

/// Whether a service understands app grants and app sources.
#[must_use]
pub fn apps_ready(protocol: u32) -> bool {
    protocol >= APPS_PROTOCOL
}

/// Why a bruh's app grants are off on this service: an older service's
/// update notice once its version is known, and `None` while grants work or
/// the version is still unknown.
#[must_use]
pub fn apps_reason(protocol: Option<u32>) -> Option<&'static str> {
    let protocol = protocol.filter(|protocol| !apps_ready(*protocol))?;
    service_notice(protocol, None).map(|notice| notice.message())
}

/// `list` with `id` added (`granted`) or taken out: sorted and deduped,
/// `id` a grant id (p4's rule: `[A-Za-z0-9_-]`, 1 to 80 characters), at
/// most `max` ids. A removal always goes through. Pass [`MAX_APP_GRANTS`]
/// for app accounts or [`MAX_MAIL_GRANTS`] for mailboxes; the refusal names
/// the one `max` is.
///
/// # Errors
///
/// The owner-facing reason when `id` isn't a grant id or the list is full.
pub fn toggle_grant(
    list: &[String],
    id: &str,
    granted: bool,
    max: usize,
) -> Result<Vec<String>, &'static str> {
    if !grant_id_ok(id) {
        return Err(NOT_AN_APP_ACCOUNT);
    }
    let mut set = grant_set(list);
    if !granted {
        set.retain(|grant| grant != id);
        return Ok(set);
    }
    if let Err(at) = set.binary_search_by(|grant| grant.as_str().cmp(id)) {
        if set.len() >= max {
            return Err(if max == MAX_MAIL_GRANTS {
                TOO_MANY_MAIL_GRANTS
            } else {
                TOO_MANY_APP_GRANTS
            });
        }
        set.insert(at, id.to_owned());
    }
    Ok(set)
}

/// Whether bruh `agent_id` may call an app with `entry` (D9): an app token
/// that is its own, or a Global one in its `apps` grants. Never another
/// kind, a Project entry or another bruh's token.
#[must_use]
pub fn usable_app_token(entry: &VaultEntry, agent_id: &str, apps: &[String]) -> bool {
    if !matches!(entry.kind, VaultKind::AppToken(_)) {
        return false;
    }
    match &entry.scope {
        CredentialScope::Bruh { agent_id: owner } => owner == agent_id,
        CredentialScope::Global => apps.contains(&entry.id),
        CredentialScope::Project { .. } => false,
    }
}

/// What a bruh's Apps section offers: its own app accounts, always usable,
/// and the Global ones it can be granted. Each sorted by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppChoices {
    pub own: Vec<VaultEntry>,
    pub shared: Vec<VaultEntry>,
}

/// The app account choices for bruh `agent_id`: app tokens only. Project
/// entries and other bruhs' tokens are never offered.
#[must_use]
pub fn app_choices(entries: &[VaultEntry], agent_id: &str) -> AppChoices {
    let mut choices = AppChoices::default();
    for entry in entries
        .iter()
        .filter(|entry| matches!(entry.kind, VaultKind::AppToken(_)))
    {
        match &entry.scope {
            CredentialScope::Bruh { agent_id: owner } if owner == agent_id => {
                choices.own.push(entry.clone());
            }
            CredentialScope::Global => choices.shared.push(entry.clone()),
            CredentialScope::Bruh { .. } | CredentialScope::Project { .. } => {}
        }
    }
    choices.own.sort_by(entry_order);
    choices.shared.sort_by(entry_order);
    choices
}

/// Whether `tool` is one of the bruh's app or mail tools.
#[must_use]
pub fn is_app_tool(tool: &str) -> bool {
    matches!(tool, APP_REQUEST_TOOL | LIST_APPS_TOOL | SEND_MAIL_TOOL)
}

/// The app account a call resolved to: the vault entry whose token it uses,
/// its app id and the account label the card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTarget {
    pub entry_id: String,
    pub app: String,
    pub account: String,
}

/// The mailbox a `send_mail` resolved to and the account the card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailTarget {
    pub mailbox_id: String,
    pub account: String,
}

/// What looking up a call's app account found, checked against the bruh's
/// live grants: the target, or the model-facing reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLookup {
    Found(AppTarget),
    Missing(&'static str),
}

/// What looking up a `send_mail`'s mailbox found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailLookup {
    Found(MailTarget),
    Missing(&'static str),
}

/// Why a bruh can't use apps or mail (D2): only a Claude bruh at an
/// effective Ask first or Full can, since only Claude asks before each
/// call.
#[must_use]
pub fn apps_unavailable(level: AccessLevel, provider: AgentProvider) -> Option<&'static str> {
    let usable = provider == AgentProvider::Claude
        && effective_level(level, provider) != AccessLevel::ReadOnly;
    (!usable).then_some(APPS_NEED_ASK)
}

/// The gate for one `app_request` (D7, ruling A): refused when the bruh
/// can't use apps, the input isn't an [`AppCall`], the call breaks a limit,
/// the app is unknown, the lookup found no account for that app, or the
/// allowlist refuses the endpoint; otherwise p1's gate for the call's class.
#[must_use]
pub fn app_gate(
    access: &BruhAccess,
    provider: AgentProvider,
    input: &Value,
    lookup: &AppLookup,
) -> Gate {
    match app_action(access, provider, input, lookup) {
        Ok(action) => gate(
            access,
            effective_level(access.level, provider),
            gated(action),
        ),
        Err(reason) => Gate::Deny(reason),
    }
}

fn app_action(
    access: &BruhAccess,
    provider: AgentProvider,
    input: &Value,
    lookup: &AppLookup,
) -> Result<AppAction, &'static str> {
    if let Some(reason) = apps_unavailable(access.level, provider) {
        return Err(reason);
    }
    let call: AppCall = serde_json::from_value(input.clone()).map_err(|_| APP_CALL_SHAPE)?;
    validate_call(&call)?;
    let connector = connector(&call.app).ok_or(UNKNOWN_APP)?;
    match lookup {
        AppLookup::Missing(reason) => return Err(reason),
        AppLookup::Found(target) if target.app != call.app => return Err(APP_NOT_GRANTED),
        AppLookup::Found(_) => {}
    }
    classify(connector, &call)
}

fn gated(action: AppAction) -> GatedAction {
    match action {
        AppAction::Read => GatedAction::Read,
        AppAction::Write => GatedAction::Write,
        AppAction::SendAsOwner => GatedAction::SendAsOwner,
        AppAction::Delivery => GatedAction::Delivery,
    }
}

/// The gate for `list_apps`: a read, for a bruh that can use apps.
#[must_use]
pub fn list_apps_gate(access: &BruhAccess, provider: AgentProvider) -> Gate {
    match apps_unavailable(access.level, provider) {
        Some(reason) => Gate::Deny(reason),
        None => gate(
            access,
            effective_level(access.level, provider),
            GatedAction::Read,
        ),
    }
}

/// The gate for one `send_mail` (D7): refused when the bruh can't use mail,
/// the input isn't a valid [`MailSendInput`] or the lookup found no
/// mailbox; otherwise a send as the owner.
#[must_use]
pub fn mail_gate(
    access: &BruhAccess,
    provider: AgentProvider,
    input: &Value,
    lookup: &MailLookup,
) -> Gate {
    if let Some(reason) = apps_unavailable(access.level, provider) {
        return Gate::Deny(reason);
    }
    let checked = serde_json::from_value::<MailSendInput>(input.clone())
        .map_err(|_| MAIL_CALL_SHAPE)
        .and_then(|mail| mail.validate());
    if let Err(reason) = checked {
        return Gate::Deny(reason);
    }
    if let MailLookup::Missing(reason) = lookup {
        return Gate::Deny(reason);
    }
    gate(
        access,
        effective_level(access.level, provider),
        GatedAction::SendAsOwner,
    )
}

/// An app call's approval card input: the call as written plus `"as"`, the
/// account it goes out as (D16). The allowance keeps the input as sent.
#[must_use]
pub fn app_card_input(input: &Value, target: &AppTarget) -> Value {
    with_as(input, &target.account)
}

/// A `send_mail`'s approval card input: the input plus `"as"`, the mailbox
/// it goes out from.
#[must_use]
pub fn mail_card_input(input: &Value, target: &MailTarget) -> Value {
    with_as(input, &target.account)
}

fn with_as(input: &Value, account: &str) -> Value {
    let mut card = input.clone();
    if let Value::Object(fields) = &mut card {
        fields.insert("as".to_owned(), Value::String(account.to_owned()));
    }
    card
}

/// One `send_mail` as the bruh wrote it (D12). A model-written `"as"` is
/// refused by the parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailSendInput {
    /// The mailbox's account label; required when more than one is granted.
    #[serde(default)]
    pub from: Option<String>,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    /// Required unless `reply` is set, and absent when it is.
    #[serde(default)]
    pub subject: Option<String>,
    pub body: String,
    #[serde(default)]
    pub reply: Option<MailReplyTarget>,
}

impl MailSendInput {
    /// Checks the subject, the body and the recipients, parsed by the
    /// compose rules: at least one in To, at most [`MAIL_MAX_RECIPIENTS`]
    /// in all once repeats are dropped.
    ///
    /// # Errors
    ///
    /// The model-facing reason the first broken rule gives.
    pub fn validate(&self) -> Result<(), &'static str> {
        let subject = match (&self.reply, &self.subject) {
            (Some(_), Some(_)) => return Err(MAIL_REPLY_SUBJECT),
            (Some(_), None) => "",
            (None, Some(subject)) if !subject.trim().is_empty() => subject.as_str(),
            (None, _) => return Err(MAIL_SUBJECT_NEEDED),
        };
        if subject.len() > MAIL_SUBJECT_MAX_BYTES {
            return Err(MAIL_SUBJECT_SHAPE);
        }
        if self.body.is_empty() || self.body.len() > MAIL_BODY_MAX_BYTES {
            return Err(MAIL_BODY_SIZE);
        }
        let content = MailComposeContent {
            to: self.to.clone(),
            cc: self.cc.clone(),
            bcc: self.bcc.clone(),
            subject: subject.to_owned(),
            body: self.body.clone(),
            attachments: Vec::new(),
        };
        let recipients = content.validate_send().map_err(|error| match error {
            MailComposeError::InvalidSubject | MailComposeError::SubjectTooLarge => {
                MAIL_SUBJECT_SHAPE
            }
            MailComposeError::InvalidBody | MailComposeError::BodyTooLarge => MAIL_BODY_TEXT,
            MailComposeError::TooManyRecipients => MAIL_TOO_MANY,
            MailComposeError::RecipientsTooLarge
            | MailComposeError::InvalidRecipient { .. }
            | MailComposeError::MissingRecipients => MAIL_RECIPIENTS,
            MailComposeError::TooManyAttachments | MailComposeError::InvalidAttachment => {
                MAIL_CALL_SHAPE
            }
        })?;
        if recipients.to.is_empty() {
            return Err(MAIL_RECIPIENTS);
        }
        let count = recipients.to.len() + recipients.cc.len() + recipients.bcc.len();
        if count > MAIL_MAX_RECIPIENTS {
            return Err(MAIL_TOO_MANY);
        }
        Ok(())
    }
}

// Card text (D16, ruling F12). The card input is the call plus `"as"`, so
// these read its fields directly; it never parses as an `AppCall`.

/// An app call card's summary: "{App}: {METHOD} {path}", or "As you on
/// {App}: …" when the call sends as the owner. The path is the normalized
/// one the call is sent on (F4). Uncut; the caller cuts it.
pub(super) fn app_summary(input: &Value) -> Option<String> {
    let (method, path) = app_method_and_path(input)?;
    let app = input["app"].as_str()?;
    let found = connector(app);
    let name = found.map_or_else(|| review_text(app), |found| found.name.to_owned());
    let mut call = input.clone();
    if let Value::Object(fields) = &mut call {
        fields.remove("as");
    }
    let sends = found.is_some_and(|found| {
        serde_json::from_value::<AppCall>(call)
            .ok()
            .and_then(|call| classify(found, &call).ok())
            == Some(AppAction::SendAsOwner)
    });
    let prefix = if sends { "As you on " } else { "" };
    Some(format!("{prefix}{name}: {method} {path}"))
}

/// An app call card's detail, never cut: the whole call first when the
/// summary was longer than `summary_max` characters, then the account, the
/// query and the body as compact JSON, each line through `review_text`.
pub(super) fn app_detail(input: &Value, summary_max: usize) -> Option<String> {
    let mut lines = Vec::new();
    if let Some((method, path)) = app_method_and_path(input) {
        if app_summary(input).is_some_and(|summary| summary.chars().count() > summary_max) {
            lines.push(format!("Call: {method} {path}"));
        }
    }
    if let Some(account) = input["as"].as_str() {
        lines.push(format!("Account: {}", review_text(account)));
    }
    match &input["query"] {
        Value::Null => {}
        Value::Object(fields) if fields.is_empty() => {}
        query => lines.push(format!("Query: {}", review_text(&query.to_string()))),
    }
    if !input["body"].is_null() {
        lines.push(format!("Body: {}", review_text(&input["body"].to_string())));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn app_method_and_path(input: &Value) -> Option<(String, String)> {
    let method = review_text(&input["method"].as_str()?.to_ascii_uppercase());
    let raw = input["path"].as_str()?;
    let path = normalize_path(raw).unwrap_or_else(|_| review_text(raw));
    Some((method, path))
}

/// A mail card's summary: "Email {to}: {subject}", or "Reply by email to
/// {to}" for a reply. Uncut; the caller cuts it.
pub(super) fn mail_summary(input: &Value) -> Option<String> {
    let to = review_text(input["to"].as_str()?);
    Some(if input["reply"].is_null() {
        let subject = review_text(input["subject"].as_str().unwrap_or_default());
        format!("Email {to}: {subject}")
    } else {
        format!("Reply by email to {to}")
    })
}

/// A mail card's detail, never cut: From, To, Cc and Bcc when set, the
/// subject (the thread's for a reply), a blank line, then the body, each
/// line through `review_text` on its own.
pub(super) fn mail_detail(input: &Value) -> Option<String> {
    let to = input["to"].as_str()?;
    let mut lines = Vec::new();
    if let Some(from) = input["as"].as_str() {
        lines.push(format!("From: {}", review_text(from)));
    }
    lines.push(format!("To: {}", review_text(to)));
    for (label, field) in [("Cc", "cc"), ("Bcc", "bcc")] {
        if let Some(value) = input[field].as_str().filter(|value| !value.is_empty()) {
            lines.push(format!("{label}: {}", review_text(value)));
        }
    }
    if !input["reply"].is_null() {
        lines.push("Subject: the thread's".to_owned());
    } else if let Some(subject) = input["subject"].as_str() {
        lines.push(format!("Subject: {}", review_text(subject)));
    }
    lines.push(String::new());
    lines.extend(
        input["body"]
            .as_str()
            .unwrap_or_default()
            .split('\n')
            .map(review_text),
    );
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{APP_GRAPHQL_SHAPE, APP_PATH};
    use crate::crew::{BRUH_BROWSER_TOOL, DELIVERY_DENIAL};
    use crate::secrets::SavedCredentialState;
    use crate::vault::AppTokenMeta;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn grants_toggle_one_id_and_stay_bounded() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            toggle_grant(&s(&["b"]), "a", true, MAX_APP_GRANTS),
            Ok(s(&["a", "b"]))
        );
        assert_eq!(
            toggle_grant(&s(&["a", "b"]), "a", false, MAX_APP_GRANTS),
            Ok(s(&["b"]))
        );
        assert_eq!(
            toggle_grant(&s(&["a"]), "a", true, MAX_APP_GRANTS),
            Ok(s(&["a"]))
        );
        assert!(toggle_grant(&s(&[]), "../x", true, MAX_APP_GRANTS).is_err());
        let full: Vec<String> = (0..16).map(|i| format!("m{i}")).collect();
        assert_eq!(
            toggle_grant(&full, "m99", true, MAX_MAIL_GRANTS),
            Err("Grant at most 16 mail accounts.")
        );
        assert!(toggle_grant(
            &s(&[]),
            &format!("mailbox-v1-{}", "a".repeat(64)),
            true,
            MAX_MAIL_GRANTS
        )
        .is_ok());
    }

    #[test]
    fn full_lists_refuse_new_ids_but_keep_the_rest() {
        let apps: Vec<String> = (0..MAX_APP_GRANTS).map(|i| format!("v1-{i:02}")).collect();
        assert_eq!(
            toggle_grant(&apps, "v1-zz", true, MAX_APP_GRANTS),
            Err("Grant at most 64 app accounts.")
        );
        let mut sorted = apps.clone();
        sorted.sort();
        assert_eq!(
            toggle_grant(&apps, "v1-00", true, MAX_APP_GRANTS),
            Ok(sorted.clone()),
            "a grant already on is no change"
        );
        assert_eq!(
            toggle_grant(&apps, "v1-00", false, MAX_APP_GRANTS).map(|list| list.len()),
            Ok(MAX_APP_GRANTS - 1)
        );
        assert_eq!(
            toggle_grant(&[], "../x", true, MAX_APP_GRANTS),
            Err("That isn't an app account.")
        );
        assert_eq!(
            toggle_grant(
                &["b".into(), "a".into(), "b".into()],
                "c",
                true,
                MAX_APP_GRANTS
            ),
            Ok(vec!["a".into(), "b".into(), "c".into()])
        );
        let too_long = "a".repeat(81);
        assert!(toggle_grant(&[], &too_long, true, MAX_APP_GRANTS).is_err());
        assert!(toggle_grant(&[], "", false, MAX_APP_GRANTS).is_err());
    }

    #[test]
    fn a_bruh_uses_its_own_app_tokens_and_granted_shared_ones() {
        let token = |id: &str, scope| VaultEntry {
            id: id.into(),
            name: id.into(),
            scope,
            state: SavedCredentialState::Saved,
            attention: None,
            kind: VaultKind::AppToken(AppTokenMeta {
                app: "github".into(),
                account: "me".into(),
                params: BTreeMap::new(),
            }),
        };
        let own = token(
            "v1-own",
            CredentialScope::Bruh {
                agent_id: "a".into(),
            },
        );
        let shared = token("v1-g", CredentialScope::Global);
        assert!(usable_app_token(&own, "a", &[]) && !usable_app_token(&shared, "a", &[]));
        assert!(
            usable_app_token(&shared, "a", &["v1-g".into()])
                && !usable_app_token(&own, "b", &["v1-own".into()])
        );
        assert_eq!(
            app_choices(&[shared.clone(), own.clone()], "a"),
            AppChoices {
                own: vec![own.clone()],
                shared: vec![shared]
            }
        );

        // Never a key, a Project token or another bruh's token.
        let key = VaultEntry {
            kind: VaultKind::Value,
            ..token("v1-k", CredentialScope::Global)
        };
        let project = token(
            "v1-p",
            CredentialScope::Project {
                path: "/work".into(),
            },
        );
        let granted = ["v1-k".to_owned(), "v1-p".to_owned()];
        assert!(
            !usable_app_token(&key, "a", &granted) && !usable_app_token(&project, "a", &granted)
        );
        let zed = VaultEntry {
            name: "Zed".into(),
            ..token("v1-z", CredentialScope::Global)
        };
        let ada = VaultEntry {
            name: "ada".into(),
            ..token("v1-y", CredentialScope::Global)
        };
        assert_eq!(
            app_choices(&[zed.clone(), key, project, own, ada.clone()], "b"),
            AppChoices {
                own: Vec::new(),
                shared: vec![ada, zed]
            }
        );
    }

    #[test]
    fn app_calls_follow_the_access_level_and_send_as_me() {
        let found = |app: &str| {
            AppLookup::Found(AppTarget {
                entry_id: format!("v1-{app}"),
                app: app.into(),
                account: "me".into(),
            })
        };
        let at = |level, send_as_me, can_deliver| BruhAccess {
            level,
            send_as_me,
            can_deliver,
        };
        let claude = AgentProvider::Claude;
        let slack = |method: &str, path: &str| json!({"app": "slack", "method": method, "path": path, "body": {}});
        let read = slack("POST", "/api/conversations.history");
        let post = slack("POST", "/api/chat.postMessage");
        assert_eq!(
            app_gate(
                &at(AccessLevel::Ask, false, false),
                claude,
                &read,
                &found("slack")
            ),
            Gate::Allow
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Ask, true, false),
                claude,
                &post,
                &found("slack")
            ),
            Gate::Approve
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, false, false),
                claude,
                &post,
                &found("slack")
            ),
            Gate::Approve
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, true, false),
                claude,
                &post,
                &found("slack")
            ),
            Gate::Allow
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::ReadOnly, true, true),
                claude,
                &read,
                &found("slack")
            ),
            Gate::Deny(APPS_NEED_ASK)
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, true, true),
                AgentProvider::Codex,
                &read,
                &found("slack")
            ),
            Gate::Deny(APPS_NEED_ASK)
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, true, true),
                claude,
                &read,
                &AppLookup::Missing(APP_NOT_GRANTED)
            ),
            Gate::Deny(APP_NOT_GRANTED)
        );
        let merge = json!({"app": "github", "method": "PUT", "path": "/repos/o/r/pulls/1/merge"});
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, true, false),
                claude,
                &merge,
                &found("github")
            ),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, false, true),
                claude,
                &merge,
                &found("github")
            ),
            Gate::Allow
        );
        let patch = json!({"app": "notion", "method": "PATCH", "path": "/v1/pages/p", "body": {}});
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, false, false),
                claude,
                &patch,
                &found("notion")
            ),
            Gate::Allow
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Ask, false, false),
                claude,
                &patch,
                &found("notion")
            ),
            Gate::Approve
        );
        // A target for another app never serves the call; a bad call is
        // refused with its reason.
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, true, true),
                claude,
                &patch,
                &found("github")
            ),
            Gate::Deny(APP_NOT_GRANTED)
        );
        assert_eq!(
            app_gate(
                &at(AccessLevel::Full, true, true),
                claude,
                &json!({"app": "notion", "method": "GET", "path": "x"}),
                &found("notion")
            ),
            Gate::Deny(APP_PATH)
        );
        assert_eq!(
            list_apps_gate(&at(AccessLevel::Ask, false, false), claude),
            Gate::Allow
        );
    }

    #[test]
    fn the_app_gate_refuses_before_it_looks_and_classifies_the_normalized_call() {
        let full = BruhAccess {
            level: AccessLevel::Full,
            send_as_me: true,
            can_deliver: false,
        };
        let claude = AgentProvider::Claude;
        let github = AppLookup::Found(AppTarget {
            entry_id: "v1-gh".into(),
            app: "github".into(),
            account: "octo".into(),
        });
        let gate_of = |input: Value| app_gate(&full, claude, &input, &github);
        // An escape can't hide a merge (F4), nor a lower-case method.
        assert_eq!(
            gate_of(
                json!({"app": "github", "method": "PUT", "path": "/repos/o/r/pulls/1/%6Derge"})
            ),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            gate_of(json!({"app": "github", "method": "PUT", "path": "/REPOS/o/r/PULLS/1/MERGE"})),
            Gate::Deny(DELIVERY_DENIAL)
        );
        // GraphQL only as a POST with no URL query (ruling A over F5).
        assert_eq!(
            gate_of(json!({"app": "github", "method": "GET", "path": "/graphql",
                "query": {"query": "mutation{x}"}})),
            Gate::Deny(APP_GRAPHQL_SHAPE)
        );
        assert_eq!(
            gate_of(
                json!({"app": "github", "method": "POST", "path": "/graphql",
                "body": {"query": "mutation{mergePullRequest(input:{}){clientMutationId}}"}})
            ),
            Gate::Deny(DELIVERY_DENIAL)
        );
        // An endpoint the allowlist doesn't name is refused with its copy.
        assert_eq!(
            gate_of(json!({"app": "github", "method": "POST", "path": "/user/keys", "body": {}})),
            Gate::Deny("Bruhs can't call that endpoint of GitHub yet.")
        );
        // A model-written "as", a header or a host is refused by the parse.
        for input in [
            json!({"app": "github", "method": "GET", "path": "/user", "as": "work"}),
            json!({"app": "github", "method": "GET", "path": "/user", "headers": {}}),
            json!({"app": "github", "method": "GET"}),
            json!("GET /user"),
        ] {
            assert_eq!(gate_of(input), Gate::Deny(APP_CALL_SHAPE));
        }
        // An unknown app is refused before the lookup, which would refuse
        // it too.
        assert_eq!(
            app_gate(
                &full,
                claude,
                &json!({"app": "myspace", "method": "GET", "path": "/x"}),
                &AppLookup::Missing(APP_NOT_GRANTED)
            ),
            Gate::Deny(UNKNOWN_APP)
        );
        // Unavailable comes first, even for a call that doesn't parse.
        assert_eq!(
            app_gate(
                &BruhAccess::default(),
                claude,
                &json!(null),
                &AppLookup::Missing(APP_NOT_GRANTED)
            ),
            Gate::Deny(APPS_NEED_ASK)
        );
        assert_eq!(
            list_apps_gate(&full, AgentProvider::Codex),
            Gate::Deny(APPS_NEED_ASK)
        );
        assert_eq!(
            apps_unavailable(AccessLevel::Ask, AgentProvider::Codex),
            Some(APPS_NEED_ASK)
        );
        assert_eq!(apps_unavailable(AccessLevel::Ask, claude), None);
        assert_eq!(apps_unavailable(AccessLevel::Full, claude), None);
    }

    #[test]
    fn the_card_input_adds_the_account_it_goes_as() {
        let target = AppTarget {
            entry_id: "v1-gh".into(),
            app: "github".into(),
            account: "octo".into(),
        };
        let input = json!({"app": "github", "method": "GET", "path": "/user"});
        assert_eq!(
            app_card_input(&input, &target),
            json!({"app": "github", "method": "GET", "path": "/user", "as": "octo"})
        );
        let mailbox = MailTarget {
            mailbox_id: "m".into(),
            account: "me@x.test".into(),
        };
        let mail = json!({"to": "ann@x.test", "subject": "Lunch", "body": "Hi"});
        assert_eq!(
            mail_card_input(&mail, &mailbox),
            json!({"to": "ann@x.test", "subject": "Lunch", "body": "Hi", "as": "me@x.test"})
        );
        assert!(is_app_tool(APP_REQUEST_TOOL) && is_app_tool(LIST_APPS_TOOL));
        assert!(is_app_tool(SEND_MAIL_TOOL) && !is_app_tool(BRUH_BROWSER_TOOL));
        assert_eq!(APP_REQUEST_TOOL, "mcp__bruhs_workbench__app_request");
    }

    #[test]
    fn mail_sends_ask_unless_full_with_send_as_me() {
        let found = MailLookup::Found(MailTarget {
            mailbox_id: "m".into(),
            account: "me@x.test".into(),
        });
        let mail = json!({"to": "ann@x.test", "subject": "Lunch", "body": "See you at 12."});
        let at = |level, send_as_me| BruhAccess {
            level,
            send_as_me,
            can_deliver: false,
        };
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Ask, true),
                AgentProvider::Claude,
                &mail,
                &found
            ),
            Gate::Approve
        );
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Full, false),
                AgentProvider::Claude,
                &mail,
                &found
            ),
            Gate::Approve
        );
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Full, true),
                AgentProvider::Claude,
                &mail,
                &found
            ),
            Gate::Allow
        );
        let reply = json!({"to": "ann@x.test", "subject": "x", "body": "y",
            "reply": {"external_thread_id": "t", "message_id": "m"}});
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Full, true),
                AgentProvider::Claude,
                &reply,
                &found
            ),
            Gate::Deny(MAIL_REPLY_SUBJECT)
        );
        let many = json!({"to": (0..21).map(|i| format!("p{i}@x.test")).collect::<Vec<_>>().join(", "),
            "subject": "s", "body": "b"});
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Full, true),
                AgentProvider::Claude,
                &many,
                &found
            ),
            Gate::Deny(MAIL_TOO_MANY)
        );
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Full, true),
                AgentProvider::Claude,
                &mail,
                &MailLookup::Missing(MAIL_FROM_NEEDED)
            ),
            Gate::Deny(MAIL_FROM_NEEDED)
        );
        assert_eq!(
            mail_gate(
                &at(AccessLevel::ReadOnly, true),
                AgentProvider::Claude,
                &mail,
                &found
            ),
            Gate::Deny(APPS_NEED_ASK)
        );
        assert_eq!(
            mail_gate(
                &at(AccessLevel::Full, true),
                AgentProvider::Claude,
                &json!({"to": "ann@x.test", "body": "b", "as": "me@x.test"}),
                &found
            ),
            Gate::Deny(MAIL_CALL_SHAPE)
        );
    }

    #[test]
    fn mail_input_follows_its_limits() {
        let input = |value: Value| serde_json::from_value::<MailSendInput>(value).unwrap();
        let ok = input(
            json!({"to": "Ann <ann@x.test>", "cc": "bo@x.test; cy@x.test",
            "subject": "Lunch", "body": "Hi\nthere\tyou"}),
        );
        assert_eq!(ok.validate(), Ok(()));
        let reply = input(json!({"to": "ann@x.test", "body": "y",
            "reply": {"external_thread_id": "t", "message_id": "m"}}));
        assert_eq!(reply.validate(), Ok(()));
        let cases = [
            (
                json!({"to": "ann@x.test", "body": "b"}),
                MAIL_SUBJECT_NEEDED,
            ),
            (
                json!({"to": "ann@x.test", "subject": "  ", "body": "b"}),
                MAIL_SUBJECT_NEEDED,
            ),
            (
                json!({"to": "ann@x.test", "subject": "s".repeat(999), "body": "b"}),
                MAIL_SUBJECT_SHAPE,
            ),
            (
                json!({"to": "ann@x.test", "subject": "a\nBcc: x@y.test", "body": "b"}),
                MAIL_SUBJECT_SHAPE,
            ),
            (
                json!({"to": "ann@x.test", "subject": "s", "body": ""}),
                MAIL_BODY_SIZE,
            ),
            (
                json!({"to": "ann@x.test", "subject": "s", "body": "x".repeat(MAIL_BODY_MAX_BYTES + 1)}),
                MAIL_BODY_SIZE,
            ),
            (
                json!({"to": "ann@x.test", "subject": "s", "body": "a\u{7}b"}),
                MAIL_BODY_TEXT,
            ),
            (
                json!({"to": "", "cc": "bo@x.test", "subject": "s", "body": "b"}),
                MAIL_RECIPIENTS,
            ),
            (
                json!({"to": "not an address", "subject": "s", "body": "b"}),
                MAIL_RECIPIENTS,
            ),
            (
                json!({"to": "ann@x.test", "subject": "s", "body": "b",
                "bcc": (0..20).map(|i| format!("p{i}@x.test")).collect::<Vec<_>>().join(",")}),
                MAIL_TOO_MANY,
            ),
        ];
        for (value, reason) in cases {
            assert_eq!(input(value.clone()).validate(), Err(reason), "{value}");
        }
        // The body may fill 8 KB; the same address twice counts once.
        let full = input(json!({"to": "ann@x.test, ann@X.TEST", "subject": "s",
            "body": "x".repeat(MAIL_BODY_MAX_BYTES)}));
        assert_eq!(full.validate(), Ok(()));
    }
}
