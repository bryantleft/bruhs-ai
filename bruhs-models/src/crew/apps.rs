//! App grants on a bruh: which app accounts and mailboxes it may use, the
//! choices its Settings offer, protocol 5 and the copy both apps show (D9,
//! D10, D23). The app tokens themselves are vault entries of kind
//! [`VaultKind::AppToken`]; the connectors live in [`crate::apps`].

use serde::{Deserialize, Serialize};

use super::vault::{entry_order, grant_id_ok, grant_set};
use super::{service_notice, APPS_PROTOCOL};
use crate::vault::{CredentialScope, VaultEntry, VaultKind};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SavedCredentialState;
    use crate::vault::AppTokenMeta;
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
}
