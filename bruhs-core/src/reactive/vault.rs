//! A bruh's vault in the runtime: the logins and keys it may use (D2), the
//! login the browser gate checks a fill against (D9), the value the command
//! route hands the host (D12), the preamble's flag (D26), and deleting a
//! deleted bruh's own entries (ruling F15).
//!
//! Every use reads the bruh's grants from the store again. Every vault
//! lookup is bounded by the runtime's vault wait and fails closed: a lookup
//! that errs or doesn't answer in time finds no login.

use super::{store, worker::TurnVault, ReactiveError, ReactiveRuntime};
use crate::{
    local_tools::FillValue,
    secrets::{OnePasswordError, SecretError},
};
use bruhs_models::{
    crew::{
        usable_by, AccessLevel, LoginTarget, KEYCHAIN_NEEDS_OWNER, LOGIN_NOT_GRANTED,
        LOGIN_NO_FIELD, LOGIN_WRONG_ORIGIN, ONE_PASSWORD_NEEDS_OWNER, ONE_PASSWORD_UNAVAILABLE,
        OWNER_ONLY_ACCESS, OWNER_ONLY_APPS, PAGE_UNKNOWN, VAULT_READ_FAILED,
    },
    imports::OWNER_ONLY_MEMORY,
    local_tools::SecretRef,
    reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand},
    secrets::review_text,
    vault::{
        same_origin, LoginFields, VaultEntry, VaultKind, OWNER_ONLY, OWNER_ONLY_CARD,
        OWNER_ONLY_DELETE_BRUH,
    },
};
use std::{future::Future, time::Duration};

/// The most logins `logins` lists.
const MAX_LOGINS: usize = 50;
/// The most key names a run's preamble lists (ruling F28).
const MAX_KEYS: usize = 20;
/// How long past the manager's own read wait (which ends a stuck read
/// first and records the entry's attention) a fill still waits for it.
const BACKSTOP: Duration = Duration::from_secs(1);
/// A vault lookup that failed or didn't answer in time.
const VAULT_UNAVAILABLE: &str = "The vault could not be read.";

/// Why a fill gets no value: refused (the bruh may not fill that), or
/// unavailable (the keychain or 1Password needs the owner, or the read
/// failed). Each holds model-facing copy, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultRefusal {
    Refused(&'static str),
    Unavailable(&'static str),
}

/// `read`, unless it outlasts `wait`. A vault error or the wait is
/// [`VAULT_UNAVAILABLE`], with no detail.
pub(super) async fn within<T>(
    wait: Duration,
    read: impl Future<Output = Result<T, SecretError>>,
) -> Result<T, ReactiveError> {
    match tokio::time::timeout(wait, read).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) | Err(_) => Err(ReactiveError::Storage(VAULT_UNAVAILABLE.into())),
    }
}

/// `entries` sorted by name as the owner sees them, then by id.
fn by_name(entries: &mut [VaultEntry]) {
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// A login as `logins` lists it and the gate checks it: `None` for any
/// other kind, or a login whose sign-in page has no origin.
fn login_target(entry: &VaultEntry) -> Option<LoginTarget> {
    let VaultKind::Login(meta) = &entry.kind else {
        return None;
    };
    Some(LoginTarget {
        login_id: entry.id.clone(),
        name: review_text(&entry.name),
        url: meta.url.clone(),
        origin: meta.origin()?,
        fields: meta.fields(),
    })
}

/// Whether a read of `entry` goes to 1Password.
fn from_one_password(entry: &VaultEntry) -> bool {
    match &entry.kind {
        VaultKind::OnePassword { .. } => true,
        VaultKind::Login(meta) => matches!(meta.fields, LoginFields::OnePassword { .. }),
        VaultKind::Value | VaultKind::AppToken(_) | VaultKind::Other => false,
    }
}

/// Whether a bruh may use `kind` as a key: a value or a 1Password
/// reference, never a login, an app token (D8) or a newer Bruhs' kind.
fn is_key(kind: &VaultKind) -> bool {
    matches!(kind, VaultKind::Value | VaultKind::OnePassword { .. })
}

/// The owner copy a failed value read ends the fill with.
fn unavailable(error: &SecretError) -> VaultRefusal {
    VaultRefusal::Unavailable(match error {
        SecretError::OnePassword(OnePasswordError::NeedsOwner) => ONE_PASSWORD_NEEDS_OWNER,
        SecretError::OnePassword(OnePasswordError::Missing | OnePasswordError::NotFound) => {
            ONE_PASSWORD_UNAVAILABLE
        }
        SecretError::Storage => KEYCHAIN_NEEDS_OWNER,
        _ => VAULT_READ_FAILED,
    })
}

impl ReactiveRuntime {
    /// The vault entries bruh `agent_id` may use, by name: its own and the
    /// Global ones it is granted ([`usable_by`]). Empty without a vault or
    /// for an id no agent has (a deleted bruh's entries wait for the sync
    /// pass).
    async fn usable_entries(&self, agent_id: &str) -> Result<Vec<VaultEntry>, ReactiveError> {
        let Some(vault) = self.vault() else {
            return Ok(Vec::new());
        };
        let id = agent_id.to_owned();
        let Some(grants) = self
            .call(move |connection| store::vault_grants(connection, &id))
            .await?
        else {
            return Ok(Vec::new());
        };
        let mut usable: Vec<VaultEntry> = within(self.vault_wait, vault.vault_entries())
            .await?
            .into_iter()
            .filter(|entry| usable_by(entry, agent_id, &grants))
            .collect();
        by_name(&mut usable);
        Ok(usable)
    }

    /// The logins bruh `agent_id` may fill, by name, at most 50: names,
    /// sign-in pages, origins and fields, never values or references.
    pub async fn logins(&self, agent_id: &str) -> Result<Vec<LoginTarget>, ReactiveError> {
        Ok(self
            .usable_entries(agent_id)
            .await?
            .iter()
            .filter_map(login_target)
            .take(MAX_LOGINS)
            .collect())
    }

    /// Login `login_id` as the gate checks a fill against it, when bruh
    /// `agent_id` may use it now.
    pub(super) async fn login_target(
        &self,
        agent_id: &str,
        login_id: &str,
    ) -> Result<Option<LoginTarget>, ReactiveError> {
        Ok(self
            .usable_entries(agent_id)
            .await?
            .iter()
            .find(|entry| entry.id == login_id)
            .and_then(login_target))
    }

    /// The value to type for `secret`, on `page_url`. The grant, the field
    /// and the origin are checked before the read and the login again
    /// after it, so a grant revoked or a login deleted while a slow read
    /// waited (1Password asking the owner) types nothing.
    ///
    /// The caller must first use up the step's allowance
    /// ([`Self::consume_browser_allowance`]) and pass that allowance's
    /// `page_url`, the page the gate let the fill through on: never the
    /// live address or anything the request names.
    ///
    /// The listing waits at most the vault wait, the read the vault wait
    /// plus one second, and the check after it the time left, so a fill
    /// ends within about 61 s with the default 30 s wait.
    ///
    /// # Errors
    ///
    /// [`VaultRefusal::Refused`] when the bruh may not fill it there, and
    /// [`VaultRefusal::Unavailable`] when it couldn't be read.
    pub async fn fill_value(
        &self,
        agent_id: &str,
        secret: &SecretRef,
        page_url: Option<&str>,
    ) -> Result<FillValue, VaultRefusal> {
        let Some(vault) = self.vault() else {
            return Err(VaultRefusal::Refused(LOGIN_NOT_GRANTED));
        };
        let usable = self
            .usable_entries(agent_id)
            .await
            .map_err(|_| VaultRefusal::Unavailable(VAULT_READ_FAILED))?;
        let Some((entry, login)) = usable.iter().find_map(|entry| {
            (entry.id == secret.login_id)
                .then(|| login_target(entry).map(|login| (entry, login)))
                .flatten()
        }) else {
            return Err(VaultRefusal::Refused(LOGIN_NOT_GRANTED));
        };
        if page_url.is_none() {
            return Err(VaultRefusal::Refused(PAGE_UNKNOWN));
        }
        if !login.fields.contains(&secret.field) {
            return Err(VaultRefusal::Refused(LOGIN_NO_FIELD));
        }
        if !same_origin(page_url, &login.origin) {
            return Err(VaultRefusal::Refused(LOGIN_WRONG_ORIGIN));
        }
        let deadline = tokio::time::Instant::now() + self.vault_wait * 2 + BACKSTOP;
        let read = vault.resolve_login(&login.login_id, secret.field);
        let value = match tokio::time::timeout(self.vault_wait + BACKSTOP, read).await {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => return Err(unavailable(&error)),
            Err(_) if from_one_password(entry) => {
                return Err(VaultRefusal::Unavailable(ONE_PASSWORD_NEEDS_OWNER));
            }
            Err(_) => return Err(VaultRefusal::Unavailable(KEYCHAIN_NEEDS_OWNER)),
        };
        // Checked again now the value is here: dropping it zeroes it.
        let now = tokio::time::timeout_at(deadline, self.login_target(agent_id, &login.login_id));
        match now.await {
            Ok(Ok(Some(still)))
                if still.origin == login.origin && still.fields.contains(&secret.field) => {}
            Ok(Ok(_)) => return Err(VaultRefusal::Refused(LOGIN_NOT_GRANTED)),
            Ok(Err(_)) | Err(_) => return Err(VaultRefusal::Unavailable(VAULT_READ_FAILED)),
        }
        Ok(FillValue {
            login_id: login.login_id,
            origin: login.origin,
            field: secret.field,
            value,
        })
    }

    /// Take a deleted vault entry's id out of every bruh's grants, under
    /// one database transaction, so a grant change made at the same time
    /// is neither lost nor brought back. The count of bruhs it changed.
    ///
    /// # Errors
    ///
    /// A store failure; no grant changed.
    pub async fn forget_vault_entry(&self, id: &str) -> Result<usize, ReactiveError> {
        let id = id.to_owned();
        let changed = self
            .call(move |connection| store::crew::forget_vault_entry(connection, &id))
            .await?;
        if changed > 0 {
            self.changed();
        }
        Ok(changed)
    }

    /// Why `command` takes the owner's own app, as its refusal's copy, or
    /// `None` when any allowed page may send it (rulings F26 and BC I2): a
    /// bruh's vault and app grants, an answer to its ask to use a vault key,
    /// call an app or send an email (`Self::is_owner_only_approval`),
    /// deleting an agent, which deletes a bruh's own vault entries, applied
    /// or approved as a proposal, and turning an app poll routine on or off,
    /// since an enabled one polls with the owner's own account. Replying to
    /// or dismissing one of those cards and approving a run an app poll
    /// started take the owner too (final review BC M1). So does anything that
    /// raises a bruh: its access, its Computer, or a model change that turns
    /// on its app grants or Computer, applied directly or through a
    /// routine's setup (A2 I1, re-review N1), and creating a bruh at Full or
    /// with Send as me or Can deliver (M1); lowering stays open. Changing a
    /// bruh's memory takes the owner too, through `SetMemory` or a save,
    /// applied or approved, so imported memory waits for the owner's own
    /// Approve (final review B I1). A save from another page first gets
    /// the stored memory back through [`Self::with_stored_memory`].
    ///
    /// # Errors
    ///
    /// A store failure reading the card or the proposal.
    pub async fn owner_only_refusal(
        &self,
        command: &ReactiveCommand,
    ) -> Result<Option<&'static str>, ReactiveError> {
        Ok(match command {
            ReactiveCommand::SetVault { .. } => Some(OWNER_ONLY),
            ReactiveCommand::SetAppGrant { .. } => Some(OWNER_ONLY_APPS),
            ReactiveCommand::Apply {
                change: ReactiveChange::DeleteAgent { .. },
            } => Some(OWNER_ONLY_DELETE_BRUH),
            ReactiveCommand::Apply {
                change: ReactiveChange::SetMemory { .. },
            } => Some(OWNER_ONLY_MEMORY),
            ReactiveCommand::Apply { change } if store::crew::saves_app_poll(change) => {
                Some(OWNER_ONLY_APPS)
            }
            ReactiveCommand::SetAutomationEnabled { id, .. } => {
                let id = id.clone();
                self.call(move |connection| store::crew::automation_polls_app(connection, &id))
                    .await?
                    .then_some(OWNER_ONLY_APPS)
            }
            ReactiveCommand::DecideProposal { id, approve: true } => {
                let id = id.clone();
                let (refusal, agent) = self
                    .call(move |connection| {
                        Ok(if store::proposal_deletes_agent(connection, &id)? {
                            (Some(OWNER_ONLY_DELETE_BRUH), None)
                        } else if store::crew::proposal_saves_app_poll(connection, &id)? {
                            (Some(OWNER_ONLY_APPS), None)
                        } else if store::crew::proposal_changes_memory(connection, &id)? {
                            (Some(OWNER_ONLY_MEMORY), None)
                        } else {
                            (None, store::crew::proposal_saves_agent(connection, &id)?)
                        })
                    })
                    .await?;
                match (refusal, agent) {
                    (Some(refusal), _) => Some(refusal),
                    (None, Some(agent)) => self.save_agent_refusal(agent).await?,
                    (None, None) => None,
                }
            }
            ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent { agent },
            } => self.save_refusal(agent).await?,
            // A routine's setup saves its bruh too (re-review N1).
            ReactiveCommand::Apply {
                change: ReactiveChange::SaveAutomation { automation },
            } => self.save_refusal(&automation.agent).await?,
            // A new bruh that could send or deliver without a card once the
            // owner grants it something (re-review M1).
            ReactiveCommand::CreateBruh { access, .. } => {
                (access.level == AccessLevel::Full || access.send_as_me || access.can_deliver)
                    .then_some(OWNER_ONLY_ACCESS)
            }
            ReactiveCommand::SetBruh {
                agent_id, access, ..
            } => {
                let (id, access) = (agent_id.clone(), access.clone());
                self.call(move |connection| store::crew::raises_bruh(connection, &id, &access))
                    .await?
                    .then_some(OWNER_ONLY_ACCESS)
            }
            ReactiveCommand::SetComputer { agent_id, on } => {
                let (id, on) = (agent_id.clone(), *on);
                self.call(move |connection| store::crew::turns_computer_on(connection, &id, on))
                    .await?
                    .then_some(OWNER_ONLY_ACCESS)
            }
            ReactiveCommand::DecideApproval { id, .. }
            | ReactiveCommand::AnswerOutbox { id, .. }
            | ReactiveCommand::DismissOutbox { id } => self
                .is_owner_only_approval(id)
                .await?
                .then_some(OWNER_ONLY_CARD),
            // A retry after a cancel starts the run just the same (M3).
            ReactiveCommand::ApproveRun { id } | ReactiveCommand::RetryRun { id } => {
                let id = id.clone();
                self.call(move |connection| store::crew::run_polls_app(connection, &id))
                    .await?
                    .then_some(OWNER_ONLY_APPS)
            }
            _ => None,
        })
    }

    /// Why saving `agent` takes the owner's own app: it raises the bruh
    /// (`Self::save_agent_refusal`) or changes its memory.
    async fn save_refusal(
        &self,
        agent: &ReactiveAgent,
    ) -> Result<Option<&'static str>, ReactiveError> {
        if let Some(refusal) = self.save_agent_refusal(agent.clone()).await? {
            return Ok(Some(refusal));
        }
        let id = agent.id.clone();
        let stored = self
            .call(move |connection| store::crew::stored_memory(connection, &id))
            .await?;
        Ok((stored != agent.memory).then_some(OWNER_ONLY_MEMORY))
    }

    /// `command` from a page that isn't the owner's, with each agent save
    /// that has no memory, applied or proposed, given the bruh's stored
    /// memory: that page is sent every bruh's memory blank
    /// (`ReactiveSnapshot::without_owner_details`), so its pause, rename
    /// or edit keeps the memory it couldn't see (final review B I1).
    ///
    /// # Errors
    ///
    /// A store failure reading the stored agent.
    pub async fn with_stored_memory(
        &self,
        mut command: ReactiveCommand,
    ) -> Result<ReactiveCommand, ReactiveError> {
        let agent = match &mut command {
            ReactiveCommand::Apply { change } | ReactiveCommand::Propose { change } => match change
            {
                ReactiveChange::SaveAgent { agent } => agent,
                ReactiveChange::SaveAutomation { automation } => &mut automation.agent,
                _ => return Ok(command),
            },
            _ => return Ok(command),
        };
        if agent.memory.is_empty() {
            let id = agent.id.clone();
            agent.memory = self
                .call(move |connection| store::crew::stored_memory(connection, &id))
                .await?;
        }
        Ok(command)
    }

    /// After an entry of bruh `agent_id`'s own was made: whether that bruh
    /// still exists. One deleted meanwhile gets its deletion record again
    /// and the forget pass runs at once, so the entry goes with the bruh's
    /// others (ruling A1 M8).
    ///
    /// # Errors
    ///
    /// A store failure; the entry stays until a later pass or a delete.
    pub async fn keep_bruh_entry(&self, agent_id: &str) -> Result<bool, ReactiveError> {
        let id = agent_id.to_owned();
        let kept = self
            .call(move |connection| store::keep_bruh_entry(connection, &id))
            .await?;
        if !kept {
            self.forget_deleted_bruhs().await;
        }
        Ok(kept)
    }

    /// Whether bruh `agent_id` has a login it may fill. False when the
    /// vault can't say.
    pub(super) async fn has_usable_logins(&self, agent_id: &str) -> bool {
        self.logins(agent_id)
            .await
            .is_ok_and(|logins| !logins.is_empty())
    }

    /// What a run's turn needs from the vault: the folders no run may
    /// reach, whether the preamble tells a bruh whose Computer is on about
    /// its logins, for a run that may have the key tool (`keys`) the keys
    /// it may use, and for one that may have the app tools (`apps`) the
    /// apps and mailboxes it may use.
    pub(super) async fn turn_vault(
        &self,
        agent_id: &str,
        computer: bool,
        keys: bool,
        apps: bool,
    ) -> TurnVault {
        let (app_names, mail) = if apps {
            self.usable_apps(agent_id).await
        } else {
            (Vec::new(), Vec::new())
        };
        TurnVault {
            private: self.private_rule_folders().to_vec(),
            logins: computer && self.has_usable_logins(agent_id).await,
            keys: if keys {
                self.usable_keys(agent_id).await
            } else {
                Vec::new()
            },
            apps: app_names,
            mail,
        }
    }

    /// The names of the keys bruh `agent_id` may use (Value and 1Password
    /// entries, never logins), sorted, at most 20, each made safe to show.
    pub(super) async fn usable_keys(&self, agent_id: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .usable_entries(agent_id)
            .await
            .unwrap_or_default()
            .iter()
            .filter(|entry| is_key(&entry.kind))
            .map(|entry| review_text(&entry.name))
            .collect();
        names.sort();
        names.dedup();
        names.truncate(MAX_KEYS);
        names
    }

    /// The id of the key named exactly `name` that bruh `agent_id` may use:
    /// its own entry before a granted Global one. Logins never match.
    pub(super) async fn usable_value(&self, agent_id: &str, name: &str) -> Option<String> {
        let usable = self.usable_entries(agent_id).await.ok()?;
        let keys = || {
            usable
                .iter()
                .filter(move |entry| entry.name == name && is_key(&entry.kind))
        };
        let own = keys().find(|entry| {
            matches!(
                &entry.scope,
                bruhs_models::vault::CredentialScope::Bruh { .. }
            )
        });
        own.or_else(|| keys().next()).map(|entry| entry.id.clone())
    }

    /// Delete the own vault entries of bruhs deleted from this database,
    /// by their deletion records, then the records. A failure is logged
    /// and the next pass tries again.
    pub(super) async fn forget_deleted_bruhs(&self) {
        let Some(vault) = self.vault() else {
            return;
        };
        let gone = match self
            .call(|connection| store::vault_forgets(connection))
            .await
        {
            Ok(gone) => gone,
            Err(error) => {
                tracing::warn!(%error, "Deleted bruhs' vault entries could not be checked");
                return;
            }
        };
        if gone.is_empty() {
            return;
        }
        if let Err(error) = within(self.vault_wait, vault.forget_bruh_entries(&gone)).await {
            tracing::warn!(%error, "A deleted bruh's vault entries could not be deleted");
            return;
        }
        if let Err(error) = self
            .call(move |connection| store::clear_vault_forgets(connection, &gone))
            .await
        {
            tracing::warn!(%error, "Deleted bruhs' vault records could not be cleared");
        }
    }

    /// The deleted bruhs whose own vault entries are still to go.
    #[cfg(test)]
    pub(super) async fn vault_forgets_for_tests(&self) -> std::collections::HashSet<String> {
        self.call(|connection| store::vault_forgets(connection))
            .await
            .unwrap()
    }
}
