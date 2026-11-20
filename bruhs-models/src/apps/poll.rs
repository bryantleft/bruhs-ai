//! App poll sources (D20): the built-in queries a routine can watch, their
//! limits, the line both apps show for one, and the editor's form rules.
//!
//! A poll uses the owner's own account, so it needs no bruh grant; core
//! polls it from Task 10 and refuses any save of one that isn't the owner's.

use super::page::{
    account_order, app_account_name, mailbox_account, mailbox_choice_label, AppsListing,
};
use crate::reactive::SourceKind;
use crate::secrets::review_text;
use crate::vault::CredentialScope;
use serde::{Deserialize, Serialize};

/// What an app poll watches. GitHub queries go with the `github` app, new
/// mail with `mail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppQuery {
    GithubIssues { repo: String },
    GithubPulls { repo: String },
    NewMail,
}

/// The app id for the GitHub queries.
pub const POLL_APP_GITHUB: &str = "github";
/// The app id for new mail, read from the existing mail store.
pub const POLL_APP_MAIL: &str = "mail";

pub const APP_POLL_MIN_MS: u64 = 60_000;
pub const APP_POLL_DEFAULT_MS: u64 = 300_000;
pub const APP_POLL_MAX_MS: u64 = 86_400_000;
/// The most new items one poll emits, oldest first.
pub const APP_POLL_MAX_ITEMS: usize = 20;
/// Each part of `owner/name`.
const REPO_PART_MAX_CHARS: usize = 100;
/// The editor's interval, in whole minutes.
const MINUTES_MAX: u64 = 1_440;
const MINUTE_MS: u64 = 60_000;
/// The longest account id a poll keeps.
const ACCOUNT_MAX_CHARS: usize = 128;

// App copy.
pub const SOURCE_KIND_APP: &str = "App";
/// The kind label of a source a newer Bruhs wrote.
pub const SOURCE_KIND_OTHER: &str = "Other";
pub const WATCH_LABEL: &str = "Watch";
pub const POLL_ACCOUNT_LABEL: &str = "Account";
pub const REPO_LABEL: &str = "Repository";
pub const REPO_HINT: &str = "owner/name";
pub const MINUTES_LABEL: &str = "Check every (minutes)";
/// A source whose kind a newer Bruhs wrote: shown read only, never saved
/// (ruling F3).
pub const FROM_NEWER_SOURCE: &str = "This source comes from a newer Bruhs. Update to change it.";
pub const REPO_SHAPE: &str = "Use owner/name for the repository.";
pub const POLL_INTERVAL_RANGE: &str = "Check at most once a minute and at least once a day.";
pub const POLL_ACCOUNT_NEEDED: &str = "Choose an account.";
pub const POLL_WATCH_MISMATCH: &str = "That watch doesn't go with that app.";
/// The simple editor's "Starts when" choice for an app poll.
pub const STARTS_WHEN_APP: &str = "GitHub or email has something new";
/// A GitHub watch with no account to poll with: polls use a Global one.
pub const NO_GITHUB_ACCOUNTS: &str =
    "No GitHub account to watch with. Connect one in Settings, Apps, for Bruhs you grant it to.";
/// Asked before an editor changes the type of a newer Bruhs' source, which
/// replaces what that Bruhs saved (ruling A Task 5).
pub const CHANGE_NEWER_SOURCE: &str =
    "This source comes from a newer Bruhs. Changing its type replaces it for good.";
pub const CHANGE_TYPE: &str = "Change type";
const YOUR_MAIL: &str = "your mail";

// Model-facing status copy (D20).
pub const APP_POLL_ACCOUNT_GONE: &str = "That account is gone. Choose another in the routine.";
pub const APPS_NOT_ON_SERVER: &str = "Apps aren't available on this server.";

/// Checks an app poll before it is saved: the watch goes with the app, an
/// account is chosen, the repository is `owner/name` and the interval is
/// between a minute and a day.
pub fn validate_app_poll(
    app: &str,
    account: &str,
    query: &AppQuery,
    poll_ms: u64,
) -> Result<(), &'static str> {
    let fits = match query {
        AppQuery::GithubIssues { .. } | AppQuery::GithubPulls { .. } => app == POLL_APP_GITHUB,
        AppQuery::NewMail => app == POLL_APP_MAIL,
    };
    if !fits {
        return Err(POLL_WATCH_MISMATCH);
    }
    let chars = account.chars().count();
    if account.trim() != account
        || chars == 0
        || chars > ACCOUNT_MAX_CHARS
        || review_text(account) != account
    {
        return Err(POLL_ACCOUNT_NEEDED);
    }
    if let AppQuery::GithubIssues { repo } | AppQuery::GithubPulls { repo } = query {
        if !repo_fits(repo) {
            return Err(REPO_SHAPE);
        }
    }
    if !(APP_POLL_MIN_MS..=APP_POLL_MAX_MS).contains(&poll_ms) {
        return Err(POLL_INTERVAL_RANGE);
    }
    Ok(())
}

fn repo_fits(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [owner, name].into_iter().all(|part| {
        let chars = part.chars().count();
        (1..=REPO_PART_MAX_CHARS).contains(&chars)
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    })
}

/// The line both apps show for an app poll. `account_label` names the
/// mailbox; without one it reads "your mail".
pub fn describe_app_poll(query: &AppQuery, account_label: Option<&str>) -> String {
    match query {
        AppQuery::GithubIssues { repo } => format!("GitHub: new issues in {repo}"),
        AppQuery::GithubPulls { repo } => format!("GitHub: new pull requests in {repo}"),
        AppQuery::NewMail => format!("Email: new mail in {}", account_label.unwrap_or(YOUR_MAIL)),
    }
}

/// The line the advanced source editor shows, read only and with Save off,
/// for a source it can't change: an app poll (edited in a routine's
/// editor) and a newer Bruhs' kind (ruling F18). `None` for every other
/// kind.
pub fn source_read_only(kind: &SourceKind) -> Option<String> {
    match kind {
        SourceKind::AppPoll { query, .. } => Some(describe_app_poll(query, None)),
        SourceKind::Other => Some(FROM_NEWER_SOURCE.into()),
        _ => None,
    }
}

/// The editor's choice of what to watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PollWatch {
    #[default]
    GithubIssues,
    GithubPulls,
    NewMail,
}

pub fn watch_label(watch: PollWatch) -> &'static str {
    match watch {
        PollWatch::GithubIssues => "New GitHub issues",
        PollWatch::GithubPulls => "New GitHub pull requests",
        PollWatch::NewMail => "New email",
    }
}

/// The App source editor's fields as typed. `repo` is ignored for new mail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppPollForm {
    pub watch: PollWatch,
    pub account: String,
    pub repo: String,
    pub minutes: String,
}

/// Builds the source the form describes, or the first thing to fix.
pub fn poll_form_kind(form: &AppPollForm) -> Result<SourceKind, &'static str> {
    let repo = || form.repo.trim().to_owned();
    let (app, query) = match form.watch {
        PollWatch::GithubIssues => (POLL_APP_GITHUB, AppQuery::GithubIssues { repo: repo() }),
        PollWatch::GithubPulls => (POLL_APP_GITHUB, AppQuery::GithubPulls { repo: repo() }),
        PollWatch::NewMail => (POLL_APP_MAIL, AppQuery::NewMail),
    };
    let minutes = form
        .minutes
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|minutes| (1..=MINUTES_MAX).contains(minutes))
        .ok_or(POLL_INTERVAL_RANGE)?;
    let poll_ms = minutes * MINUTE_MS;
    let account = form.account.trim();
    validate_app_poll(app, account, &query, poll_ms)?;
    Ok(SourceKind::AppPoll {
        app: app.into(),
        account: account.into(),
        query,
        poll_ms,
    })
}

/// The form an app poll opens with; `None` for any other kind. An interval
/// that isn't whole minutes shows rounded to the nearest one.
pub fn poll_form_for(kind: &SourceKind) -> Option<AppPollForm> {
    let SourceKind::AppPoll {
        account,
        query,
        poll_ms,
        ..
    } = kind
    else {
        return None;
    };
    let (watch, repo) = match query {
        AppQuery::GithubIssues { repo } => (PollWatch::GithubIssues, repo.clone()),
        AppQuery::GithubPulls { repo } => (PollWatch::GithubPulls, repo.clone()),
        AppQuery::NewMail => (PollWatch::NewMail, String::new()),
    };
    let minutes = poll_ms.saturating_add(MINUTE_MS / 2) / MINUTE_MS;
    Some(AppPollForm {
        watch,
        account: account.clone(),
        repo,
        minutes: minutes.clamp(1, MINUTES_MAX).to_string(),
    })
}

/// A new App source's form: new GitHub issues, checked at the default
/// interval.
#[must_use]
pub fn new_poll_form() -> AppPollForm {
    AppPollForm {
        minutes: (APP_POLL_DEFAULT_MS / MINUTE_MS).to_string(),
        ..AppPollForm::default()
    }
}

/// The accounts a watch can poll with, as `(id, label)`: Global GitHub
/// accounts by name for the GitHub watches, mailboxes for new email. A
/// poll runs as the owner, so a bruh's own account is never offered.
#[must_use]
pub fn poll_account_choices(watch: PollWatch, listing: &AppsListing) -> Vec<(String, String)> {
    if watch == PollWatch::NewMail {
        return listing
            .mailboxes
            .iter()
            .map(|mailbox| (mailbox.mailbox_id.clone(), mailbox_choice_label(mailbox)))
            .collect();
    }
    let mut accounts: Vec<_> = listing
        .accounts
        .iter()
        .filter(|account| {
            account.app == POLL_APP_GITHUB && account.scope == CredentialScope::Global
        })
        .collect();
    accounts.sort_by(|a, b| account_order(a, b));
    accounts
        .into_iter()
        .map(|account| (account.id.clone(), app_account_name(account)))
        .collect()
}

/// The account a new mail poll names in its line ([`describe_app_poll`]):
/// the mailbox's account. `None` for any other source, or a mailbox the
/// listing no longer has.
#[must_use]
pub fn poll_account_label(kind: &SourceKind, listing: &AppsListing) -> Option<String> {
    let SourceKind::AppPoll {
        account,
        query: AppQuery::NewMail,
        ..
    } = kind
    else {
        return None;
    };
    listing
        .mailboxes
        .iter()
        .find(|mailbox| &mailbox.mailbox_id == account)
        .map(|mailbox| mailbox_account(mailbox.provider, &mailbox.account))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{AppAccount, AppMailbox};
    use crate::connections::ServiceProvider;
    use crate::reactive::EventSource;
    use crate::secrets::SavedCredentialState;
    use crate::vault::CredentialScope;
    use serde_json::json;

    fn listing() -> AppsListing {
        let account = |id: &str, app: &str, name: &str, scope: CredentialScope| AppAccount {
            id: id.into(),
            app: app.into(),
            name: name.into(),
            account: "octo".into(),
            scope,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        AppsListing {
            accounts: vec![
                account("v1-w", "github", "Work", CredentialScope::Global),
                account("v1-a", "github", "alt", CredentialScope::Global),
                account(
                    "v1-o",
                    "github",
                    "Own",
                    CredentialScope::Bruh {
                        agent_id: "a".into(),
                    },
                ),
                account("v1-n", "notion", "Notes", CredentialScope::Global),
            ],
            mailboxes: vec![AppMailbox {
                mailbox_id: "m1".into(),
                provider: ServiceProvider::Gmail,
                account: "me@x.test".into(),
            }],
            ..AppsListing::default()
        }
    }

    /// D20: a GitHub watch polls with a Global GitHub account, new mail
    /// with a mailbox; each list reads as the bruh's Settings read it.
    #[test]
    fn poll_accounts_go_with_the_watch() {
        let listing = listing();
        let github = vec![
            ("v1-a".to_owned(), "alt".to_owned()),
            ("v1-w".to_owned(), "Work".to_owned()),
        ];
        assert_eq!(
            poll_account_choices(PollWatch::GithubIssues, &listing),
            github
        );
        assert_eq!(
            poll_account_choices(PollWatch::GithubPulls, &listing),
            github
        );
        assert_eq!(
            poll_account_choices(PollWatch::NewMail, &listing),
            vec![("m1".to_owned(), "me@x.test, Gmail".to_owned())]
        );
        let mail = SourceKind::AppPoll {
            app: POLL_APP_MAIL.into(),
            account: "m1".into(),
            query: AppQuery::NewMail,
            poll_ms: APP_POLL_DEFAULT_MS,
        };
        assert_eq!(
            poll_account_label(&mail, &listing).as_deref(),
            Some("me@x.test")
        );
        assert_eq!(
            describe_app_poll(
                &AppQuery::NewMail,
                poll_account_label(&mail, &listing).as_deref()
            ),
            "Email: new mail in me@x.test"
        );
        let gone = SourceKind::AppPoll {
            app: POLL_APP_MAIL.into(),
            account: "m9".into(),
            query: AppQuery::NewMail,
            poll_ms: APP_POLL_DEFAULT_MS,
        };
        assert_eq!(poll_account_label(&gone, &listing), None);
        assert_eq!(poll_account_label(&SourceKind::Custom, &listing), None);
        let form = new_poll_form();
        assert_eq!(
            (form.watch, form.minutes.as_str()),
            (PollWatch::GithubIssues, "5")
        );
        for copy in [
            STARTS_WHEN_APP,
            NO_GITHUB_ACCOUNTS,
            CHANGE_NEWER_SOURCE,
            CHANGE_TYPE,
        ] {
            assert!(
                !copy.contains(['–', '—', '•', '·']) && !copy.contains(" - "),
                "{copy}"
            );
        }
    }

    #[test]
    fn app_polls_have_stable_wire_shapes_and_limits() {
        let kind = SourceKind::AppPoll {
            app: "github".into(),
            account: "v1-g".into(),
            query: AppQuery::GithubIssues { repo: "o/r".into() },
            poll_ms: 300_000,
        };
        assert_eq!(
            serde_json::to_value(&kind).unwrap(),
            json!({"type": "app_poll", "app": "github", "account": "v1-g",
                "query": {"type": "github_issues", "repo": "o/r"}, "poll_ms": 300_000})
        );
        assert_eq!(
            serde_json::from_value::<SourceKind>(serde_json::to_value(&kind).unwrap()).unwrap(),
            kind
        );
        assert!(validate_app_poll(
            "github",
            "v1-g",
            &AppQuery::GithubPulls {
                repo: "o/r.js".into()
            },
            60_000
        )
        .is_ok());
        assert_eq!(
            validate_app_poll(
                "mail",
                "m",
                &AppQuery::GithubIssues { repo: "o/r".into() },
                60_000
            ),
            Err("That watch doesn't go with that app.")
        );
        assert_eq!(
            validate_app_poll("github", "m", &AppQuery::NewMail, 60_000),
            Err(POLL_WATCH_MISMATCH)
        );
        for repo in ["o", "o/r/x", "../r", "o/..", "o/r r", ""] {
            assert_eq!(
                validate_app_poll(
                    "github",
                    "v1-g",
                    &AppQuery::GithubIssues { repo: repo.into() },
                    60_000
                ),
                Err("Use owner/name for the repository."),
                "{repo}"
            );
        }
        let long = "a".repeat(101);
        assert!(validate_app_poll(
            "github",
            "v1-g",
            &AppQuery::GithubIssues {
                repo: format!("o/{long}")
            },
            60_000
        )
        .is_err());
        assert!(validate_app_poll("mail", "m", &AppQuery::NewMail, 59_999).is_err());
        assert_eq!(
            validate_app_poll("mail", "m", &AppQuery::NewMail, APP_POLL_MAX_MS + 1),
            Err("Check at most once a minute and at least once a day.")
        );
        assert!(validate_app_poll("mail", "m", &AppQuery::NewMail, APP_POLL_MAX_MS).is_ok());
        for account in ["", " m", "m\u{202e}"] {
            assert_eq!(
                validate_app_poll("mail", account, &AppQuery::NewMail, 60_000),
                Err("Choose an account."),
                "{account:?}"
            );
        }
        assert_eq!(
            describe_app_poll(&AppQuery::NewMail, None),
            "Email: new mail in your mail"
        );
        assert_eq!(
            describe_app_poll(&AppQuery::NewMail, Some("me@x.test")),
            "Email: new mail in me@x.test"
        );
        assert_eq!(
            describe_app_poll(&AppQuery::GithubIssues { repo: "o/r".into() }, None),
            "GitHub: new issues in o/r"
        );
        assert_eq!(
            describe_app_poll(&AppQuery::GithubPulls { repo: "o/r".into() }, None),
            "GitHub: new pull requests in o/r"
        );
        let form = AppPollForm {
            watch: PollWatch::NewMail,
            account: "m".into(),
            repo: String::new(),
            minutes: "5".into(),
        };
        assert_eq!(
            poll_form_kind(&form),
            Ok(SourceKind::AppPoll {
                app: "mail".into(),
                account: "m".into(),
                query: AppQuery::NewMail,
                poll_ms: 300_000
            })
        );
        assert_eq!(
            poll_form_for(&poll_form_kind(&form).unwrap())
                .unwrap()
                .minutes,
            "5"
        );
    }

    #[test]
    fn the_poll_form_round_trips_and_names_what_to_fix() {
        let form = AppPollForm {
            watch: PollWatch::GithubPulls,
            account: " v1-g ".into(),
            repo: " o/r ".into(),
            minutes: "1440".into(),
        };
        let kind = poll_form_kind(&form).unwrap();
        assert_eq!(
            kind,
            SourceKind::AppPoll {
                app: "github".into(),
                account: "v1-g".into(),
                query: AppQuery::GithubPulls { repo: "o/r".into() },
                poll_ms: APP_POLL_MAX_MS
            }
        );
        assert_eq!(
            poll_form_for(&kind),
            Some(AppPollForm {
                watch: PollWatch::GithubPulls,
                account: "v1-g".into(),
                repo: "o/r".into(),
                minutes: "1440".into()
            })
        );
        for minutes in ["0", "1441", "1.5", "", "-1", "five"] {
            let form = AppPollForm {
                minutes: minutes.into(),
                ..form.clone()
            };
            assert_eq!(poll_form_kind(&form), Err(POLL_INTERVAL_RANGE), "{minutes}");
        }
        let bad_repo = AppPollForm {
            repo: "o".into(),
            ..form.clone()
        };
        assert_eq!(poll_form_kind(&bad_repo), Err(REPO_SHAPE));
        let no_account = AppPollForm {
            account: "  ".into(),
            ..form
        };
        assert_eq!(poll_form_kind(&no_account), Err(POLL_ACCOUNT_NEEDED));
        assert_eq!(poll_form_for(&SourceKind::Webhook), None);
        assert_eq!(PollWatch::default(), PollWatch::GithubIssues);
        assert_eq!(
            [
                PollWatch::GithubIssues,
                PollWatch::GithubPulls,
                PollWatch::NewMail
            ]
            .map(watch_label),
            ["New GitHub issues", "New GitHub pull requests", "New email"]
        );
    }

    #[test]
    fn unknown_source_and_vault_kinds_read_as_other() {
        let source: EventSource =
            serde_json::from_value(json!({"id": "s", "name": "n", "enabled": true,
            "kind": {"type": "rss_feed", "url": "https://x.test/feed", "poll_ms": 1}}))
            .unwrap();
        assert_eq!(source.kind, SourceKind::Other);
        assert_eq!(poll_form_for(&source.kind), None);
        assert_eq!(
            source_read_only(&source.kind).as_deref(),
            Some("This source comes from a newer Bruhs. Update to change it.")
        );
        let app = SourceKind::AppPoll {
            app: "github".into(),
            account: "v1-g".into(),
            query: AppQuery::GithubPulls { repo: "o/r".into() },
            poll_ms: 60_000,
        };
        assert_eq!(
            source_read_only(&app).as_deref(),
            Some("GitHub: new pull requests in o/r")
        );
        assert_eq!(source_read_only(&SourceKind::Custom), None);
        // A known kind still refuses a field it doesn't have.
        assert!(serde_json::from_value::<SourceKind>(
            json!({"type": "http", "url": "https://x.test", "poll_ms": 1, "x": 1})
        )
        .is_err());
        assert!(serde_json::from_value::<AppQuery>(
            json!({"type": "github_issues", "repo": "o/r", "x": 1})
        )
        .is_err());
        assert!(serde_json::from_value::<AppQuery>(json!({"type": "rss"})).is_err());
        let vault: crate::vault::VaultKind =
            serde_json::from_value(json!({"kind": "passkey", "rp": "x.test"})).unwrap();
        assert_eq!(vault, crate::vault::VaultKind::Other);
    }
}
