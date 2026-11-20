//! App polls (D20): new GitHub issues and pull requests, and new mail, as
//! routine events.
//!
//! A poll reads with the owner's own account, chosen by the owner, so it
//! takes no bruh grant. Its cursor keeps a digest of the source's
//! definition: a new or changed source, or a cursor this build can't read,
//! records where things stand and emits nothing (no backfill). After that a
//! poll emits at most [`APP_POLL_MAX_ITEMS`] new items, oldest first, and the
//! cursor moves to the last one it emitted. Every text field goes through
//! `review_text` before it enters a payload, and no failure carries a token.

use super::host::vault_error;
use super::{AppsError, AppsHost};
use crate::mail::{CacheError, MailError, MailRuntime};
use crate::secrets::SecretError;
use bruhs_models::apps::{
    connector, test_failure_message, AppConnector, AppMailbox, AppQuery, TestFailure,
    APP_POLL_ACCOUNT_GONE, APP_POLL_MAX_ITEMS, MAILBOXES_UNAVAILABLE, POLL_APP_GITHUB,
    POLL_APP_MAIL, POLL_WATCH_MISMATCH,
};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::crew::MAIL_UNAVAILABLE;
use bruhs_models::secrets::review_text;
use bruhs_models::vault::{CredentialScope, VaultKind};
use bruhs_models::EpochMs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// How many items a GitHub list asks for, newest first.
const GITHUB_PAGE: &str = "50";
/// The most characters of an issue or pull request body an event keeps.
const EXCERPT_CHARS: usize = 500;
/// The most characters of any other text field an event keeps, so one
/// event always fits the routine store's payload budget.
const FIELD_CHARS: usize = 500;
/// The most labels an event names.
const MAX_LABELS: usize = 20;
/// The most characters of a new-mail preview.
const PREVIEW_CHARS: usize = 300;

/// One poll's result: its events as `(key, payload)`, oldest first, and the
/// cursor to save with them.
pub(crate) struct PollOutcome {
    pub events: Vec<(String, Value)>,
    pub cursor: Value,
    /// The source's status after the events are saved: the mail sync's copy
    /// when it failed, though the poll still read what the cache holds.
    pub status: Option<String>,
}

/// Where a GitHub poll stands: the newest issue or pull request number it
/// has seen or emitted.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GithubCursor {
    definition: String,
    newest: u64,
}

/// Where a mail poll stands: the newest `message_at_ms` it has emitted, or
/// its baseline.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MailCursor {
    definition: String,
    after_ms: i64,
}

/// Which GitHub list a poll reads.
#[derive(Clone, Copy)]
enum GithubWatch {
    Issues,
    Pulls,
}

impl GithubWatch {
    /// The list's path segment.
    fn segment(self) -> &'static str {
        match self {
            Self::Issues => "issues",
            Self::Pulls => "pulls",
        }
    }

    /// The key's kind segment.
    fn key(self) -> &'static str {
        match self {
            Self::Issues => "issue",
            Self::Pulls => "pull",
        }
    }

    fn event(self) -> &'static str {
        match self {
            Self::Issues => "github_issue_opened",
            Self::Pulls => "github_pull_opened",
        }
    }

    /// The issues list holds pull requests too; they carry `pull_request`.
    fn wants(self, item: &Value) -> bool {
        match self {
            Self::Issues => item.get("pull_request").is_none(),
            Self::Pulls => true,
        }
    }
}

impl AppsHost {
    /// Polls one app source. `definition` names the source's app, account
    /// and query; a cursor saved for another one starts a new baseline.
    ///
    /// # Errors
    ///
    /// The source's status copy, and no cursor to save: a missing account
    /// ([`APP_POLL_ACCOUNT_GONE`]), a refused token (the test-failure copy),
    /// or an app or cache that couldn't be read.
    pub(crate) async fn poll(
        &self,
        app: &str,
        account: &str,
        query: &AppQuery,
        cursor: Option<Value>,
        definition: &str,
    ) -> Result<PollOutcome, AppsError> {
        let definition = digest(definition);
        match (app, query) {
            (POLL_APP_GITHUB, AppQuery::GithubIssues { repo }) => {
                self.poll_github(account, repo, GithubWatch::Issues, cursor, definition)
                    .await
            }
            (POLL_APP_GITHUB, AppQuery::GithubPulls { repo }) => {
                self.poll_github(account, repo, GithubWatch::Pulls, cursor, definition)
                    .await
            }
            (POLL_APP_MAIL, AppQuery::NewMail) => self.poll_mail(account, cursor, definition).await,
            _ => Err(AppsError::Invalid(POLL_WATCH_MISMATCH)),
        }
    }

    async fn poll_github(
        &self,
        account: &str,
        repo: &str,
        watch: GithubWatch,
        cursor: Option<Value>,
        definition: String,
    ) -> Result<PollOutcome, AppsError> {
        let (connector, params) = self.github_account(account).await?;
        let secret =
            self.vault()
                .resolve_app_token(account)
                .await
                .map_err(|error| match error {
                    SecretError::NotSaved => AppsError::Refused(APP_POLL_ACCOUNT_GONE),
                    error => vault_error(error),
                })?;
        let query: BTreeMap<String, Value> = [
            ("state", "all"),
            ("sort", "created"),
            ("direction", "desc"),
            ("per_page", GITHUB_PAGE),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
        .collect();
        let path = format!("/repos/{repo}/{}", watch.segment());
        let reply = self
            .http()
            .read_whole(connector, &params, &secret, &path, &query)
            .await?;
        drop(secret);
        let failed = |failure| AppsError::TestFailed(test_failure_message(connector.name, failure));
        match reply.status {
            200..=299 => {}
            401 | 403 => return Err(failed(TestFailure::Refused)),
            _ => return Err(failed(TestFailure::Unexpected)),
        }
        let Value::Array(items) = reply.body else {
            return Err(failed(TestFailure::Unexpected));
        };
        let mut found: Vec<(u64, &Value)> = items
            .iter()
            .filter(|item| watch.wants(item))
            .filter_map(|item| Some((item.get("number")?.as_u64()?, item)))
            .collect();
        let Some(mut cursor) = read_cursor::<GithubCursor>(cursor, &definition) else {
            let newest = found.iter().map(|(number, _)| *number).max().unwrap_or(0);
            return outcome(Vec::new(), &GithubCursor { definition, newest }, None);
        };
        found.retain(|(number, _)| *number > cursor.newest);
        found.sort_by_key(|(number, _)| *number);
        found.dedup_by_key(|(number, _)| *number);
        found.truncate(APP_POLL_MAX_ITEMS);
        if let Some((number, _)) = found.last() {
            cursor.newest = *number;
        }
        let events = found
            .into_iter()
            .map(|(number, item)| {
                (
                    format!("github:{repo}:{}:{number}", watch.key()),
                    github_payload(watch, repo, number, item),
                )
            })
            .collect();
        outcome(events, &cursor, None)
    }

    /// A Global GitHub token's connector and params; anything else, or no
    /// entry, is [`APP_POLL_ACCOUNT_GONE`].
    async fn github_account(
        &self,
        account: &str,
    ) -> Result<(&'static AppConnector, BTreeMap<String, String>), AppsError> {
        let gone = AppsError::Refused(APP_POLL_ACCOUNT_GONE);
        let Some(entry) = self
            .vault()
            .vault_entry(account)
            .await
            .map_err(vault_error)?
        else {
            return Err(gone);
        };
        match (entry.kind, entry.scope) {
            (VaultKind::AppToken(meta), CredentialScope::Global) if meta.app == POLL_APP_GITHUB => {
                let connector = connector(&meta.app).ok_or(gone)?;
                Ok((connector, meta.params))
            }
            _ => Err(gone),
        }
    }

    async fn poll_mail(
        &self,
        mailbox_id: &str,
        cursor: Option<Value>,
        definition: String,
    ) -> Result<PollOutcome, AppsError> {
        let Some(mail) = &self.mail else {
            return Err(AppsError::Unavailable(MAIL_UNAVAILABLE));
        };
        // One mail poll at a time: two never sync a mailbox at once. The
        // Inbox's own sync can still run meanwhile; the cache refuses
        // whichever saves second (a revision conflict), so neither moves
        // the other's sync position.
        let _polling = self.mail_polls.lock().await;
        let (mailbox, connection_id) = self
            .connected_mailboxes()
            .await?
            .into_iter()
            .find(|(mailbox, _)| mailbox.mailbox_id == mailbox_id)
            .ok_or(AppsError::Refused(APP_POLL_ACCOUNT_GONE))?;
        let status = self.sync_mail(mail, &mailbox, &connection_id).await;
        let unreadable = |_| AppsError::Unavailable(MAILBOXES_UNAVAILABLE);
        let Some(mut cursor) = read_cursor::<MailCursor>(cursor, &definition) else {
            let after_ms = mail
                .newest_unread_ms(mailbox_id)
                .await
                .map_err(unreadable)?
                .unwrap_or_else(|| EpochMs::now().as_millis());
            return outcome(
                Vec::new(),
                &MailCursor {
                    definition,
                    after_ms,
                },
                status,
            );
        };
        let mut threads = mail
            .unread_since(mailbox_id, cursor.after_ms, APP_POLL_MAX_ITEMS)
            .await
            .map_err(unreadable)?;
        // A full page may stop partway through threads that share its last
        // time; leave those for the next poll, which reads past the cursor.
        if threads.len() == APP_POLL_MAX_ITEMS {
            let last = threads.last().map(|thread| thread.message_at_ms);
            if let Some(keep) = threads
                .iter()
                .rposition(|thread| Some(thread.message_at_ms) != last)
            {
                threads.truncate(keep + 1);
            }
        }
        if let Some(thread) = threads.last() {
            cursor.after_ms = thread.message_at_ms;
        }
        let events = threads
            .into_iter()
            .map(|thread| {
                let key = format!(
                    "mail:{}:{}:{}",
                    mailbox.mailbox_id, thread.external_thread_id, thread.message_at_ms
                );
                let payload = json!({
                    "event": "new_mail",
                    "mailbox": reviewed(&mailbox.mailbox_id, FIELD_CHARS),
                    "account": reviewed(&mailbox.account, FIELD_CHARS),
                    "external_thread_id": reviewed(&thread.external_thread_id, FIELD_CHARS),
                    "message_id": reviewed(&thread.message_id, FIELD_CHARS),
                    "sender": reviewed(&thread.sender, FIELD_CHARS),
                    "subject": reviewed(&thread.subject, FIELD_CHARS),
                    "preview": reviewed(&thread.preview, PREVIEW_CHARS),
                    "message_at_ms": thread.message_at_ms,
                });
                (key, payload)
            })
            .collect();
        outcome(events, &cursor, status)
    }

    /// Runs one provider sync page for `mailbox` in a session for its
    /// connection. `None` when it worked or another sync got there first;
    /// otherwise the sync's own copy, for the source's status.
    async fn sync_mail(
        &self,
        mail: &MailRuntime,
        mailbox: &AppMailbox,
        connection_id: &str,
    ) -> Option<String> {
        let session = match mail
            .session(self.vault().connections(), mailbox.provider, connection_id)
            .await
        {
            Ok(session) => session,
            Err(error) => return sync_problem(error),
        };
        match mailbox.provider {
            ServiceProvider::Gmail => match self.gmail.sync_page(&session).await {
                Ok(_) => None,
                Err(crate::mail::gmail::GmailError::Mail(error)) => sync_problem(error),
                Err(error) => Some(error.to_string()),
            },
            ServiceProvider::Outlook => match self.outlook.sync_page(&session).await {
                Ok(_) => None,
                Err(crate::mail::outlook::OutlookError::Mail(error)) => sync_problem(error),
                Err(error) => Some(error.to_string()),
            },
            _ => None,
        }
    }
}

/// A failed sync's status copy. A revision conflict means another sync
/// saved first, which is no problem for the poll.
fn sync_problem(error: MailError) -> Option<String> {
    match error {
        MailError::Cache(CacheError::RevisionConflict) => None,
        error => Some(error.to_string()),
    }
}

/// The saved cursor when it is this definition's and readable; `None`
/// starts a new baseline.
fn read_cursor<T: serde::de::DeserializeOwned + HasDefinition>(
    cursor: Option<Value>,
    definition: &str,
) -> Option<T> {
    cursor
        .and_then(|cursor| serde_json::from_value::<T>(cursor).ok())
        .filter(|cursor| cursor.definition() == definition)
}

trait HasDefinition {
    fn definition(&self) -> &str;
}

impl HasDefinition for GithubCursor {
    fn definition(&self) -> &str {
        &self.definition
    }
}

impl HasDefinition for MailCursor {
    fn definition(&self) -> &str {
        &self.definition
    }
}

fn outcome(
    events: Vec<(String, Value)>,
    cursor: &impl Serialize,
    status: Option<String>,
) -> Result<PollOutcome, AppsError> {
    Ok(PollOutcome {
        events,
        // Plain strings and numbers: serializing can't fail.
        cursor: json!(cursor),
        status,
    })
}

/// A new issue's or pull request's event.
fn github_payload(watch: GithubWatch, repo: &str, number: u64, item: &Value) -> Value {
    let text = |pointer: &str, max: usize| {
        reviewed(
            item.pointer(pointer).and_then(Value::as_str).unwrap_or(""),
            max,
        )
    };
    let labels: Vec<String> = item
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|label| label.get("name").and_then(Value::as_str))
        .take(MAX_LABELS)
        .map(|name| reviewed(name, FIELD_CHARS))
        .collect();
    json!({
        "event": watch.event(),
        "repo": reviewed(repo, FIELD_CHARS),
        "number": number,
        "title": text("/title", FIELD_CHARS),
        "url": text("/html_url", FIELD_CHARS),
        "author": text("/user/login", FIELD_CHARS),
        "created_at": text("/created_at", FIELD_CHARS),
        "labels": labels,
        "excerpt": text("/body", EXCERPT_CHARS),
    })
}

/// `text` with its controls made visible, at most `max` characters.
fn reviewed(text: &str, max: usize) -> String {
    let start: String = text.chars().take(max).collect();
    review_text(&start).chars().take(max).collect()
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
#[path = "poll_tests.rs"]
mod tests;
