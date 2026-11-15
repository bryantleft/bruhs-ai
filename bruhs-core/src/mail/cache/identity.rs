//! Stable mailbox, provider-record and host-replica identities.
//!
//! The broker supplies its stable account subject and registration fingerprint
//! after checking mail permissions. Neither a display email nor the rotating
//! authorization id establishes durable mailbox identity.
//!
//! These hashes are identifiers, not credentials or proof of authorization.
//! Every operation must still obtain current broker access while retaining its
//! existing connection lease, then validate its expected authorization reference.
//! A caller cannot establish access by supplying two matching references.

use bruhs_models::connections::ServiceProvider;
use sha2::{Digest as _, Sha256};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailIdentityError {
    NotMailProvider,
    InvalidRegistration,
    EmptyIdentifier,
    IdentifierTooLong,
    MailboxChanged,
    AuthorizationChanged,
    InvalidReference,
}

impl fmt::Display for MailIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotMailProvider => "Choose a Gmail or Outlook Mail connection.",
            Self::InvalidRegistration | Self::EmptyIdentifier | Self::IdentifierTooLong => {
                "The saved mail identity is invalid. Reconnect the account."
            }
            Self::MailboxChanged => {
                "The connected mailbox changed. Review this action for the intended account."
            }
            Self::AuthorizationChanged => {
                "The account was reconnected. Review this action before continuing."
            }
            Self::InvalidReference => {
                "The cached mail reference is invalid. Refresh before continuing."
            }
        })
    }
}

impl std::error::Error for MailIdentityError {}

fn digest_id(domain: &str, components: &[&str]) -> Result<String, MailIdentityError> {
    let mut digest = Sha256::new();
    for component in std::iter::once(domain).chain(components.iter().copied()) {
        let length =
            u64::try_from(component.len()).map_err(|_| MailIdentityError::IdentifierTooLong)?;
        // Fixed-width, big-endian lengths keep ids identical on 32/64-bit clients
        // and avoid delimiter ambiguity for opaque provider identifiers.
        digest.update(length.to_be_bytes());
        digest.update(component.as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn nonempty(value: &str) -> Result<(), MailIdentityError> {
    if value.is_empty() {
        Err(MailIdentityError::EmptyIdentifier)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MailboxId {
    provider: ServiceProvider,
    id: String,
}

impl MailboxId {
    /// Restores a checked local row identity, never a provider capability.
    pub(super) fn from_cache_record(provider: &str, id: &str) -> Result<Self, MailIdentityError> {
        let provider = match provider {
            "gmail" => ServiceProvider::Gmail,
            "outlook" => ServiceProvider::Outlook,
            _ => return Err(MailIdentityError::NotMailProvider),
        };
        let digest = id
            .strip_prefix("mailbox-v1-")
            .ok_or(MailIdentityError::InvalidReference)?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(MailIdentityError::InvalidReference);
        }
        Ok(Self {
            provider,
            id: id.to_owned(),
        })
    }
    /// Identity is stable across token refresh and same-registration reconnects.
    /// A different registration is deliberately a different namespace; it needs
    /// explicit cache reconciliation rather than a merge based on a display email.
    pub fn from_broker_identity(
        provider: ServiceProvider,
        registration: &str,
        account_subject: &str,
    ) -> Result<Self, MailIdentityError> {
        if !matches!(provider, ServiceProvider::Gmail | ServiceProvider::Outlook) {
            return Err(MailIdentityError::NotMailProvider);
        }
        if registration.len() != 64
            || !registration
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(MailIdentityError::InvalidRegistration);
        }
        nonempty(account_subject)?;
        let digest = digest_id(
            "bruhs.mailbox.v1",
            &[provider.id(), registration, account_subject],
        )?;
        Ok(Self {
            provider,
            id: format!("mailbox-v1-{digest}"),
        })
    }

    pub const fn provider(&self) -> ServiceProvider {
        self.provider
    }

    pub fn as_str(&self) -> &str {
        &self.id
    }

    pub fn thread(&self, external_id: &str) -> Result<RemoteThreadRef, MailIdentityError> {
        nonempty(external_id)?;
        let digest = digest_id("bruhs.mail.thread.v1", &[self.as_str(), external_id])?;
        Ok(RemoteThreadRef {
            mailbox: self.clone(),
            external_id: external_id.into(),
            canonical_id: format!("mail-thread-v1-{digest}"),
        })
    }

    pub fn message(&self, external_id: &str) -> Result<RemoteMessageRef, MailIdentityError> {
        nonempty(external_id)?;
        let digest = digest_id("bruhs.mail.message.v1", &[self.as_str(), external_id])?;
        Ok(RemoteMessageRef {
            mailbox: self.clone(),
            external_id: external_id.into(),
            canonical_id: format!("mail-message-v1-{digest}"),
        })
    }

    pub fn attachment(&self, message_id: &str, part_id: &str) -> Result<String, MailIdentityError> {
        nonempty(message_id)?;
        // Gmail uses the empty part id for a message's root MIME part.
        let digest = digest_id(
            "bruhs.mail.attachment.v1",
            &[self.as_str(), message_id, part_id],
        )?;
        Ok(format!("mail-attachment-v1-{digest}"))
    }

    /// A client-generated draft id is independent of any provider message or
    /// thread id. A new email has no reply target until one is explicitly set.
    pub fn draft(
        &self,
        client_id: &str,
        reply_to: Option<&RemoteThreadRef>,
    ) -> Result<MailDraftRef, MailIdentityError> {
        nonempty(client_id)?;
        if let Some(reference) = reply_to {
            if reference.mailbox != *self {
                return Err(MailIdentityError::MailboxChanged);
            }
            if self.thread(&reference.external_id)? != *reference {
                return Err(MailIdentityError::InvalidReference);
            }
        }
        let digest = digest_id("bruhs.mail.draft.v1", &[self.as_str(), client_id])?;
        Ok(MailDraftRef {
            mailbox: self.clone(),
            client_id: client_id.into(),
            canonical_id: format!("mail-draft-v1-{digest}"),
            reply_to: reply_to.cloned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailDraftRef {
    pub mailbox: MailboxId,
    pub client_id: String,
    pub canonical_id: String,
    pub reply_to: Option<RemoteThreadRef>,
}

impl MailDraftRef {
    /// An immutable outgoing file belongs to one draft and client upload id.
    /// This identifier is not a provider attachment reference or authorization.
    pub fn attachment(&self, client_id: &str) -> Result<String, MailIdentityError> {
        self.validate()?;
        nonempty(client_id)?;
        let digest = digest_id(
            "bruhs.mail.draft.attachment.v1",
            &[self.mailbox.as_str(), &self.canonical_id, client_id],
        )?;
        Ok(format!("mail-draft-attachment-v1-{digest}"))
    }

    pub fn validate(&self) -> Result<(), MailIdentityError> {
        if self
            .mailbox
            .draft(&self.client_id, self.reply_to.as_ref())?
            != *self
        {
            return Err(MailIdentityError::InvalidReference);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteThreadRef {
    pub mailbox: MailboxId,
    pub external_id: String,
    pub canonical_id: String,
}

/// Stable identity loaded from the connected-mail store's host record. It is
/// independent of a server URL/port, which can change across restarts. Callers
/// obtain the actual host identity through their authenticated server session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MailCacheHostId(String);

impl MailCacheHostId {
    pub fn from_host_record(value: &str) -> Result<Self, MailIdentityError> {
        nonempty(value)?;
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl RemoteThreadRef {
    /// Client Inbox stores use this host-scoped key, and retain the canonical
    /// thread reference separately for server operations. Connecting to another
    /// host cannot overwrite a cached conversation from the previous host.
    pub fn replica_id(&self, host: &MailCacheHostId) -> Result<String, MailIdentityError> {
        let digest = digest_id(
            "bruhs.mail.replica.thread.v1",
            &[host.as_str(), &self.canonical_id],
        )?;
        Ok(format!("mail-replica-v1-{digest}"))
    }
}

/// Message identity is mailbox-scoped, independent of folder membership or the
/// current thread projection. Folder moves must not create a new message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMessageRef {
    pub mailbox: MailboxId,
    pub external_id: String,
    pub canonical_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailAuthorizationRef {
    mailbox: MailboxId,
    connection_id: String,
}

impl MailAuthorizationRef {
    pub fn from_broker_access(
        mailbox: MailboxId,
        connection_id: &str,
    ) -> Result<Self, MailIdentityError> {
        nonempty(connection_id)?;
        Ok(Self {
            mailbox,
            connection_id: connection_id.into(),
        })
    }

    pub const fn mailbox(&self) -> &MailboxId {
        &self.mailbox
    }

    /// Public connection metadata id, never the OAuth access token.
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    /// `current` must come from a freshly validated broker lease. A queued
    /// operation cannot silently adopt a new authorization, even for the same
    /// mailbox; deliberate review/reconciliation produces a new operation.
    pub fn validate_current(&self, current: &Self) -> Result<(), MailIdentityError> {
        if self.mailbox != current.mailbox {
            return Err(MailIdentityError::MailboxChanged);
        }
        if self.connection_id != current.connection_id {
            return Err(MailIdentityError::AuthorizationChanged);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
