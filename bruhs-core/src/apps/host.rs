//! The apps host (D8, D18, D21): connects and replaces app tokens only
//! after a passing test call, tests saved ones, sends a gated call with its
//! token, and lists accounts and mailboxes, value free.
//!
//! Tokens arrive in the write-only [`AppConnect`] and [`AppReplace`] bodies
//! and move into `Zeroizing` at once. The store wipes what it keeps or
//! refuses; nothing here logs, returns or formats one.

use super::{AppHttp, AppsError};
use crate::mail::gmail::GmailClient;
use crate::mail::outlook::OutlookClient;
use crate::mail::MailRuntime;
use crate::secrets::{AppSecret, SecretError, SecretManager};
use bruhs_models::apps::{
    classify, connected_as, connector, default_account_name, mailbox_account, test_account,
    test_failure_message, validate_param, AppAccount, AppCall, AppConnect, AppConnector,
    AppMailbox, AppReplace, AppResponse, AppTestResult, TestFailure, ACCOUNT_CONNECTED,
    ANOTHER_ACCOUNT, MAILBOXES_UNAVAILABLE, UNKNOWN_APP,
};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::crew::KEYCHAIN_NEEDS_OWNER;
use bruhs_models::vault::{
    validate_entry_name, AppTokenMeta, CredentialScope, VaultCreate, VaultEntry, VaultKind,
    VaultSecretInput, VaultUpdate, APPS_NOT_PROJECTS, CHOOSE_APP, CHOOSE_BRUH, INVALID_ENTRY,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use zeroize::Zeroizing;

/// Everything the Apps routes, a bruh's app calls and the polls go
/// through. Clone it freely: its [`AppHttp`] is the process's one client,
/// so every clone shares its call slots.
#[derive(Clone)]
pub struct AppsHost {
    vault: SecretManager,
    http: AppHttp,
    mail: Option<MailRuntime>,
    #[expect(dead_code, reason = "used from Task 9")]
    gmail: Arc<GmailClient>,
    #[expect(dead_code, reason = "used from Task 9")]
    outlook: Arc<OutlookClient>,
}

impl AppsHost {
    /// `http` is the process's one [`AppHttp`], cloned (ruling: one per
    /// process). Mailboxes are read through `vault.connections()` (F14).
    pub fn new(
        vault: SecretManager,
        http: AppHttp,
        mail: Option<MailRuntime>,
        gmail: Arc<GmailClient>,
        outlook: Arc<OutlookClient>,
    ) -> Self {
        Self {
            vault,
            http,
            mail,
            gmail,
            outlook,
        }
    }

    pub fn http(&self) -> &AppHttp {
        &self.http
    }

    pub fn vault(&self) -> &SecretManager {
        &self.vault
    }

    /// Connects an account: checks the form, runs the app's test call with
    /// the trimmed token, then saves it as an `AppToken` named by the
    /// account the test named. A failed test saves nothing and is
    /// [`AppsError::TestFailed`] with its copy.
    pub async fn connect(&self, connect: AppConnect) -> Result<VaultEntry, AppsError> {
        let AppConnect {
            app,
            name,
            scope,
            params,
            token,
            key,
        } = connect;
        let (token, key) = (Zeroizing::new(token), key.map(Zeroizing::new));
        let connector = connector(&app).ok_or(AppsError::Invalid(CHOOSE_APP))?;
        match &scope {
            CredentialScope::Project { .. } => return Err(AppsError::Invalid(APPS_NOT_PROJECTS)),
            CredentialScope::Bruh { agent_id } if agent_id.trim().is_empty() => {
                return Err(AppsError::Invalid(CHOOSE_BRUH))
            }
            CredentialScope::Global | CredentialScope::Bruh { .. } => {}
        }
        let name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        if let Some(name) = &name {
            validate_entry_name(name).map_err(AppsError::Invalid)?;
        }
        let params = account_params(connector, &params)?;
        let mut secret = app_secret(connector, &token, key.as_deref().map(String::as_str))?;
        drop((token, key));

        let account = self
            .test(connector, &params, &secret)
            .await
            .map_err(|failure| {
                AppsError::TestFailed(test_failure_message(connector.name, failure))
            })?;
        let taken = self
            .vault
            .app_tokens()
            .await
            .map_err(vault_error)?
            .into_iter()
            .any(|entry| {
                entry.scope == scope
                    && matches!(&entry.kind, VaultKind::AppToken(meta)
                        if meta.app == connector.id && meta.account == account)
            });
        if taken {
            return Err(AppsError::Invalid(ACCOUNT_CONNECTED));
        }
        let name = name.unwrap_or_else(|| default_account_name(connector.name, &account));
        // Moved out, not copied: the store wipes them once saved or refused.
        let create = VaultCreate {
            name,
            scope,
            secret: VaultSecretInput::AppToken {
                app: connector.id.to_owned(),
                account,
                params,
                token: Some(std::mem::take(&mut *secret.token)),
                key: secret.key.as_mut().map(|key| std::mem::take(&mut **key)),
            },
        };
        drop(secret);
        self.vault.vault_create(create).await.map_err(vault_error)
    }

    /// Replaces an account's token (and Trello's key, else the stored one
    /// is kept) once the new one passes the test call and names the same
    /// account. Its name, scope, params and grants stay.
    pub async fn replace(&self, id: &str, replace: AppReplace) -> Result<VaultEntry, AppsError> {
        let AppReplace { token, key } = replace;
        let (token, key) = (Zeroizing::new(token), key.map(Zeroizing::new));
        let (entry, meta, connector) = self.app_entry(id).await?;
        let token = checked(connector, "token", &token)?;
        let key = match key {
            Some(key) => Some(checked(connector, "key", &key)?),
            None => None,
        };
        // A new token alone keeps the stored key, which its test needs too.
        let needs_key = connector.params.iter().any(|param| param.id == "key");
        let mut secret = match key {
            Some(key) => AppSecret {
                token,
                key: Some(key),
            },
            None if needs_key => AppSecret {
                token,
                key: self.resolve(&entry.id).await?.key,
            },
            None => AppSecret { token, key: None },
        };
        let account = self
            .test(connector, &meta.params, &secret)
            .await
            .map_err(|failure| {
                AppsError::TestFailed(test_failure_message(connector.name, failure))
            })?;
        if account != meta.account {
            return Err(AppsError::TestFailed(ANOTHER_ACCOUNT.to_owned()));
        }
        let update = VaultUpdate {
            secret: VaultSecretInput::AppToken {
                app: meta.app,
                account: meta.account,
                params: meta.params,
                token: Some(std::mem::take(&mut *secret.token)),
                key: secret.key.as_mut().map(|key| std::mem::take(&mut **key)),
            },
        };
        drop(secret);
        self.vault
            .vault_update(&entry.id, update)
            .await
            .map_err(vault_error)
    }

    /// Runs a saved account's test call. A failed test is an `ok: false`
    /// result; an entry the vault can't read is an error.
    pub async fn test_saved(&self, id: &str) -> Result<AppTestResult, AppsError> {
        // The keychain read first: it is bounded by the vault's read wait.
        let secret = self.resolve(id).await?;
        let (_, meta, connector) = self.app_entry(id).await?;
        Ok(match self.test(connector, &meta.params, &secret).await {
            Ok(account) => AppTestResult {
                ok: true,
                message: connected_as(&account),
                account: Some(account),
            },
            Err(failure) => AppTestResult {
                ok: false,
                account: None,
                message: test_failure_message(connector.name, failure),
            },
        })
    }

    /// Every app account, value free, with its last failure.
    pub async fn accounts(&self) -> Result<Vec<AppAccount>, AppsError> {
        let entries = self.vault.app_tokens().await.map_err(vault_error)?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| match entry.kind {
                VaultKind::AppToken(meta) => Some(AppAccount {
                    id: entry.id,
                    app: meta.app,
                    name: entry.name,
                    account: meta.account,
                    scope: entry.scope,
                    state: entry.state,
                    attention: entry.attention,
                }),
                _ => None,
            })
            .collect())
    }

    /// The Gmail and Outlook mailboxes whose connection is present now
    /// (F14). None without a mail runtime.
    pub async fn mailboxes(&self) -> Result<Vec<AppMailbox>, AppsError> {
        let Some(mail) = &self.mail else {
            return Ok(Vec::new());
        };
        let snapshot = mail
            .mailboxes()
            .await
            .map_err(|_| AppsError::Unavailable(MAILBOXES_UNAVAILABLE))?;
        let mut connected: HashMap<ServiceProvider, Option<String>> = HashMap::new();
        let mut mailboxes = Vec::new();
        for mailbox in snapshot.mailboxes {
            if !matches!(
                mailbox.provider,
                ServiceProvider::Gmail | ServiceProvider::Outlook
            ) {
                continue;
            }
            let id = match connected.get(&mailbox.provider) {
                Some(id) => id.clone(),
                None => {
                    let id = self
                        .vault
                        .connections()
                        .connection(mailbox.provider)
                        .await
                        .map_err(|_| AppsError::Unavailable(MAILBOXES_UNAVAILABLE))?
                        .id;
                    connected.insert(mailbox.provider, id.clone());
                    id
                }
            };
            if id.as_deref() == Some(mailbox.last_connection_id.as_str()) {
                mailboxes.push(AppMailbox {
                    account: mailbox_account(mailbox.provider, &mailbox.account_label),
                    mailbox_id: mailbox.mailbox_id,
                    provider: mailbox.provider,
                });
            }
        }
        Ok(mailboxes)
    }

    /// Sends a call the gate allowed with `entry`'s token. The call must be
    /// for the entry's app and on its allowlist; the host and limits are
    /// checked again as it is sent.
    pub async fn request(
        &self,
        entry: &VaultEntry,
        call: &AppCall,
    ) -> Result<AppResponse, AppsError> {
        let VaultKind::AppToken(meta) = &entry.kind else {
            return Err(AppsError::Invalid(UNKNOWN_APP));
        };
        let connector = connector(&meta.app).ok_or(AppsError::Invalid(UNKNOWN_APP))?;
        if call.app != connector.id {
            return Err(AppsError::Invalid(UNKNOWN_APP));
        }
        classify(connector, call).map_err(AppsError::Invalid)?;
        let secret = self.resolve(&entry.id).await?;
        self.http.call(connector, &meta.params, &secret, call).await
    }

    /// An app token entry with its metadata and connector; any other kind
    /// is [`UNKNOWN_APP`].
    async fn app_entry(
        &self,
        id: &str,
    ) -> Result<(VaultEntry, AppTokenMeta, &'static AppConnector), AppsError> {
        let entry = self
            .vault
            .vault_entry(id)
            .await
            .map_err(vault_error)?
            .ok_or(AppsError::Vault(SecretError::NotSaved))?;
        let VaultKind::AppToken(meta) = &entry.kind else {
            return Err(AppsError::Invalid(UNKNOWN_APP));
        };
        let meta = meta.clone();
        let connector = connector(&meta.app).ok_or(AppsError::Invalid(UNKNOWN_APP))?;
        Ok((entry, meta, connector))
    }

    async fn resolve(&self, id: &str) -> Result<AppSecret, AppsError> {
        self.vault.resolve_app_token(id).await.map_err(vault_error)
    }

    /// The connector's test call: the account it names, as stored.
    async fn test(
        &self,
        connector: &'static AppConnector,
        params: &BTreeMap<String, String>,
        secret: &AppSecret,
    ) -> Result<String, TestFailure> {
        let test = &connector.test;
        let body = match test.body {
            Some(text) => {
                Some(serde_json::from_str::<Value>(text).map_err(|_| TestFailure::Unexpected)?)
            }
            None => None,
        };
        let reply = self
            .http
            .raw(
                connector,
                params,
                secret,
                test.method,
                test.path,
                body.as_ref(),
            )
            .await
            .map_err(|error| match error {
                AppsError::Refused(_)
                | AppsError::Unavailable(_)
                | AppsError::Unreachable(_)
                | AppsError::TimedOut(_) => TestFailure::Unreachable,
                AppsError::Invalid(_)
                | AppsError::TooLarge(_)
                | AppsError::TestFailed(_)
                | AppsError::Vault(_) => TestFailure::Unexpected,
            })?;
        test_account(test, reply.status, &reply.body)
    }
}

/// The account's params: exactly the app's non-secret values, each as
/// [`validate_param`] reads it (ruling F24). A secret or unknown key is
/// refused, so no token is ever kept as metadata.
fn account_params(
    connector: &AppConnector,
    given: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AppsError> {
    let mut params = BTreeMap::new();
    for param in connector.params.iter().filter(|param| !param.secret) {
        let value = given.get(param.id).map_or("", String::as_str);
        params.insert(
            param.id.to_owned(),
            validate_param(param, value).map_err(AppsError::Invalid)?,
        );
    }
    if given.keys().any(|id| !params.contains_key(id)) {
        return Err(AppsError::Invalid(INVALID_ENTRY));
    }
    Ok(params)
}

/// A new account's token and key, trimmed and checked. A key is required
/// exactly when the app takes one.
fn app_secret(
    connector: &AppConnector,
    token: &str,
    key: Option<&str>,
) -> Result<AppSecret, AppsError> {
    let token = checked(connector, "token", token)?;
    let takes_key = connector.params.iter().any(|param| param.id == "key");
    let key = match (takes_key, key) {
        (true, Some(key)) => Some(checked(connector, "key", key)?),
        (true, None) => return Err(AppsError::Invalid(bruhs_models::apps::ENTER_TOKEN)),
        (false, Some(_)) => return Err(AppsError::Invalid(INVALID_ENTRY)),
        (false, None) => None,
    };
    Ok(AppSecret { token, key })
}

/// One secret param, trimmed and checked by the app's rules.
fn checked(
    connector: &AppConnector,
    id: &str,
    value: &str,
) -> Result<Zeroizing<String>, AppsError> {
    let param = connector
        .params
        .iter()
        .find(|param| param.secret && param.id == id)
        .ok_or(AppsError::Invalid(INVALID_ENTRY))?;
    validate_param(param, value)
        .map(Zeroizing::new)
        .map_err(AppsError::Invalid)
}

/// A vault failure as the host reports it: a refused input keeps its copy,
/// and a keychain that didn't answer within the read wait needs the owner.
fn vault_error(error: SecretError) -> AppsError {
    match error {
        SecretError::Invalid(message) => AppsError::Invalid(message),
        SecretError::Storage => AppsError::Unavailable(KEYCHAIN_NEEDS_OWNER),
        error => AppsError::Vault(error),
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
