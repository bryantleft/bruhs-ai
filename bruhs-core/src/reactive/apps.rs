//! A bruh's apps in the runtime (spec section 7; D9, D18, D19): the app
//! accounts and mailboxes it may use, the one a call goes out as, and what
//! `list_apps` tells it.
//!
//! Every use reads the bruh's grants from the store again and the accounts
//! and mailboxes live, so a revoked grant, a deleted token or a
//! disconnected mailbox counts at once (a stale mailbox id grants
//! nothing). Every lookup is bounded by the runtime's vault wait and fails
//! closed. Nothing here reads a token: only the vault's value free listing.

use super::{store, ReactiveError, ReactiveRuntime};
use bruhs_models::apps::{
    base_url, connector, AppConnector, AppMailbox, APPS_NOT_ON_SERVER, CONNECTORS,
};
use bruhs_models::crew::{
    usable_app_token, AppLookup, AppTarget, MailLookup, MailTarget, APPS_UNREADABLE,
    APP_ACCOUNT_NEEDED, APP_ACCOUNT_UNKNOWN, APP_NOT_GRANTED, MAIL_FROM_NEEDED, MAIL_NOT_GRANTED,
    PREAMBLE_MAX_NAMES,
};
use bruhs_models::mail::MailReplyTarget;
use bruhs_models::vault::{VaultEntry, VaultKind};
use serde_json::{json, Value};

/// One app account a bruh may use, with where its calls go.
struct Usable {
    entry_id: String,
    /// The vault entry's name, as the owner named it.
    name: String,
    /// The label its token's test call named (with its Jira site or
    /// Shopify shop).
    label: String,
    connector: &'static AppConnector,
    base_url: String,
}

/// `entry` as an account bruh `agent_id` may use with `granted`: its own
/// app token or a granted Global one, for a known app whose address
/// builds. Anything else is `None`.
fn usable(entry: VaultEntry, agent_id: &str, granted: &[String]) -> Option<Usable> {
    if !usable_app_token(&entry, agent_id, granted) {
        return None;
    }
    let VaultKind::AppToken(meta) = entry.kind else {
        return None;
    };
    let connector = connector(&meta.app)?;
    let base_url = base_url(connector, &meta.params).ok()?;
    Some(Usable {
        entry_id: entry.id,
        name: entry.name,
        label: meta.account,
        connector,
        base_url,
    })
}

/// Catalog order, then the name as the owner sees it, then the id.
fn catalog_order(a: &Usable, b: &Usable) -> std::cmp::Ordering {
    let place = |usable: &Usable| {
        CONNECTORS
            .iter()
            .position(|connector| connector.id == usable.connector.id)
    };
    place(a)
        .cmp(&place(b))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.entry_id.cmp(&b.entry_id))
}

/// How `one` is named to the bruh and on its cards: its label, unique
/// among the bruh's accounts for that app; when another shares it, the
/// label and the entry's name.
fn handle(all: &[Usable], one: &Usable) -> String {
    let shared = all
        .iter()
        .filter(|other| other.connector.id == one.connector.id && other.label == one.label)
        .count()
        > 1;
    if shared {
        format!("{} ({})", one.label, one.name)
    } else {
        one.label.clone()
    }
}

/// The account an `app_request` input goes out as, from the bruh's usable
/// accounts. Without `account`, the app's one account; with it, the
/// account whose entry name is exactly that, else whose name on cards is,
/// else whose label is. More than one match asks the bruh to name one.
fn pick_account(all: &[Usable], input: &Value) -> AppLookup {
    let Some(app) = input.get("app").and_then(Value::as_str) else {
        return AppLookup::Missing(APP_NOT_GRANTED);
    };
    let candidates: Vec<&Usable> = all
        .iter()
        .filter(|usable| usable.connector.id == app)
        .collect();
    if candidates.is_empty() {
        return AppLookup::Missing(APP_NOT_GRANTED);
    }
    let picked: Vec<&Usable> = match input.get("account") {
        None | Some(Value::Null) => candidates,
        Some(Value::String(account)) => {
            let matching = |test: &dyn Fn(&Usable) -> bool| -> Vec<&Usable> {
                candidates
                    .iter()
                    .copied()
                    .filter(|usable| test(usable))
                    .collect()
            };
            let by_name = matching(&|usable| usable.name == *account);
            let by_handle = || matching(&|usable| handle(all, usable) == *account);
            let by_label = || matching(&|usable| usable.label == *account);
            let found = if by_name.is_empty() {
                let handled = by_handle();
                if handled.is_empty() {
                    by_label()
                } else {
                    handled
                }
            } else {
                by_name
            };
            if found.is_empty() {
                return AppLookup::Missing(APP_ACCOUNT_UNKNOWN);
            }
            found
        }
        Some(_) => return AppLookup::Missing(APP_ACCOUNT_UNKNOWN),
    };
    match picked.as_slice() {
        [one] => AppLookup::Found(AppTarget {
            entry_id: one.entry_id.clone(),
            app: one.connector.id.to_owned(),
            account: handle(all, one),
        }),
        _ => AppLookup::Missing(APP_ACCOUNT_NEEDED),
    }
}

/// The mailbox a `send_mail` goes out from, among the bruh's granted,
/// connected ones: the one `from` names, in any case, or the only one
/// when `from` is left out. A label two of them share picks neither.
///
/// # Errors
///
/// [`MAIL_FROM_NEEDED`] when `from` is left out and more than one is
/// granted, [`MAIL_NOT_GRANTED`] otherwise.
pub(super) fn pick_mailbox<'a>(
    granted: &'a [AppMailbox],
    from: Option<&str>,
) -> Result<&'a AppMailbox, &'static str> {
    let picked: Vec<&AppMailbox> = match from {
        None => granted.iter().collect(),
        Some(from) => granted
            .iter()
            .filter(|mailbox| mailbox.account.eq_ignore_ascii_case(from))
            .collect(),
    };
    match picked.as_slice() {
        [one] => Ok(one),
        [_, _, ..] if from.is_none() => Err(MAIL_FROM_NEEDED),
        _ => Err(MAIL_NOT_GRANTED),
    }
}

/// One account as `list_apps` names it: no token, header or vault id.
fn account_json(all: &[Usable], one: &Usable) -> Value {
    json!({"name": one.name, "account": handle(all, one), "base_url": one.base_url})
}

impl ReactiveRuntime {
    /// Bruh `agent_id`'s app and mailbox grants, read live, or `None` when
    /// no agent has that id.
    async fn app_grants(
        &self,
        agent_id: &str,
    ) -> Result<Option<(Vec<String>, Vec<String>)>, ReactiveError> {
        let id = agent_id.to_owned();
        self.call(move |connection| {
            if store::vault_grants(connection, &id)?.is_none() {
                return Ok(None);
            }
            let profile = store::crew::profile(connection, &id)?;
            Ok(Some((profile.apps, profile.mail)))
        })
        .await
    }

    /// The app accounts bruh `agent_id` may use, in catalog order, and the
    /// connected mailboxes it is granted. Empty without apps on this
    /// server or for an id no agent has. Not bounded: callers bound it.
    async fn usable_now(
        &self,
        agent_id: &str,
    ) -> Result<(Vec<Usable>, Vec<AppMailbox>), ReactiveError> {
        let unreadable = || ReactiveError::Conflict(APPS_UNREADABLE.into());
        let Some(apps) = self.apps() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let Some((granted, mail)) = self.app_grants(agent_id).await? else {
            return Ok((Vec::new(), Vec::new()));
        };
        let mut accounts: Vec<Usable> = apps
            .vault()
            .app_tokens()
            .await
            .map_err(|_| unreadable())?
            .into_iter()
            .filter_map(|entry| usable(entry, agent_id, &granted))
            .collect();
        accounts.sort_by(catalog_order);
        let mailboxes = if mail.is_empty() {
            Vec::new()
        } else {
            apps.mailboxes()
                .await
                .map_err(|_| unreadable())?
                .into_iter()
                .filter(|mailbox| mail.contains(&mailbox.mailbox_id))
                .collect()
        };
        Ok((accounts, mailboxes))
    }

    /// [`Self::usable_now`] within the vault wait.
    async fn usable_within(
        &self,
        agent_id: &str,
    ) -> Result<(Vec<Usable>, Vec<AppMailbox>), ReactiveError> {
        tokio::time::timeout(self.vault_wait, self.usable_now(agent_id))
            .await
            .unwrap_or_else(|_| Err(ReactiveError::Conflict(APPS_UNREADABLE.into())))
    }

    /// The account an `app_request` with `input` goes out as for bruh
    /// `agent_id`, by its live grants (D9, ruling F24). A lookup that
    /// fails or doesn't answer within the vault wait is
    /// `Missing(APP_NOT_GRANTED)`.
    pub async fn app_lookup(&self, agent_id: &str, input: &Value) -> AppLookup {
        match self.usable_within(agent_id).await {
            Ok((accounts, _)) => pick_account(&accounts, input),
            Err(_) => AppLookup::Missing(APP_NOT_GRANTED),
        }
    }

    /// The mailbox a `send_mail` with `input` goes out from for bruh
    /// `agent_id`: one it is granted that is connected now (D18). For a
    /// reply, the target carries the subject of the message it answers,
    /// as the mail store holds it. A lookup that fails or doesn't answer
    /// within the vault wait is `Missing(MAIL_NOT_GRANTED)`.
    pub async fn mail_lookup(&self, agent_id: &str, input: &Value) -> MailLookup {
        let lookup = async {
            let (_, mailboxes) = self.usable_now(agent_id).await.ok()?;
            let from = match input.get("from") {
                None | Some(Value::Null) => None,
                Some(Value::String(from)) => Some(from.as_str()),
                Some(_) => return Some(MailLookup::Missing(MAIL_NOT_GRANTED)),
            };
            let mailbox = match pick_mailbox(&mailboxes, from) {
                Ok(mailbox) => mailbox,
                Err(reason) => return Some(MailLookup::Missing(reason)),
            };
            let reply = input
                .get("reply")
                .and_then(|reply| serde_json::from_value::<MailReplyTarget>(reply.clone()).ok());
            let reply_subject = match (&reply, self.apps()) {
                (Some(reply), Some(apps)) => apps.reply_subject(&mailbox.mailbox_id, reply).await,
                _ => None,
            };
            Some(MailLookup::Found(MailTarget {
                mailbox_id: mailbox.mailbox_id.clone(),
                account: mailbox.account.clone(),
                reply_subject,
            }))
        };
        tokio::time::timeout(self.vault_wait, lookup)
            .await
            .ok()
            .flatten()
            .unwrap_or(MailLookup::Missing(MAIL_NOT_GRANTED))
    }

    /// What `list_apps` answers bruh `agent_id` (D1): each app it may use
    /// with its accounts (name, account and base address), and the
    /// mailboxes it may send from. With `app`, that app's guide and
    /// accounts. Never a token, header or vault id.
    ///
    /// # Errors
    ///
    /// [`APP_NOT_GRANTED`] for an app the bruh may not use or no such app;
    /// [`APPS_NOT_ON_SERVER`] without apps here; [`APPS_UNREADABLE`] when
    /// the accounts couldn't be read within the vault wait.
    pub async fn list_apps(
        &self,
        agent_id: &str,
        app: Option<&str>,
    ) -> Result<Value, ReactiveError> {
        if self.apps().is_none() {
            return Err(ReactiveError::Conflict(APPS_NOT_ON_SERVER.into()));
        }
        let (accounts, mailboxes) = self.usable_within(agent_id).await?;
        if let Some(app) = app {
            let mine: Vec<Value> = accounts
                .iter()
                .filter(|usable| usable.connector.id == app)
                .map(|usable| account_json(&accounts, usable))
                .collect();
            let Some(connector) = connector(app).filter(|_| !mine.is_empty()) else {
                return Err(ReactiveError::Invalid(APP_NOT_GRANTED.into()));
            };
            return Ok(json!({
                "app": connector.id,
                "name": connector.name,
                "guide": connector.guide,
                "accounts": mine,
            }));
        }
        let mut apps: Vec<Value> = Vec::new();
        for connector in &CONNECTORS {
            let mine: Vec<Value> = accounts
                .iter()
                .filter(|usable| usable.connector.id == connector.id)
                .map(|usable| account_json(&accounts, usable))
                .collect();
            if !mine.is_empty() {
                apps.push(json!({"app": connector.id, "name": connector.name, "accounts": mine}));
            }
        }
        let mail: Vec<&str> = mailboxes
            .iter()
            .map(|mailbox| mailbox.account.as_str())
            .collect();
        Ok(json!({"apps": apps, "mail": mail}))
    }

    /// The names of the apps bruh `agent_id` may call and the labels of
    /// the mailboxes it may send from, for its run's preamble: each sorted,
    /// deduped and at most 20. Empty when they can't be read in time.
    pub(super) async fn usable_apps(&self, agent_id: &str) -> (Vec<String>, Vec<String>) {
        let Ok((accounts, mailboxes)) = self.usable_within(agent_id).await else {
            return (Vec::new(), Vec::new());
        };
        let listed = |mut names: Vec<String>| {
            names.sort();
            names.dedup();
            names.truncate(PREAMBLE_MAX_NAMES);
            names
        };
        (
            listed(
                accounts
                    .iter()
                    .map(|usable| usable.connector.name.to_owned())
                    .collect(),
            ),
            listed(
                mailboxes
                    .into_iter()
                    .map(|mailbox| mailbox.account)
                    .collect(),
            ),
        )
    }
}
