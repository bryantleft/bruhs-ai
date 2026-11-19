//! Vault grants on a bruh: which entries it may use, the choices its
//! Settings offer, protocol 4 and the copy both apps show (D2, D3, D23).
//! The entries themselves live in [`crate::vault`].

use super::{service_notice, VAULT_PROTOCOL};
use crate::vault::{CredentialScope, VaultEntry};

/// A bruh holds at most this many grants.
pub const MAX_VAULT_GRANTS: usize = 64;
/// The longest entry id a grant names (64 hex legacy ids and `v1-…` fit).
const MAX_GRANT_ID_CHARS: usize = 80;

pub const VAULT_GRANTS_LABEL: &str = "Vault";
pub const VAULT_GRANTS_HINT: &str =
    "Logins it may fill on their own sites without asking. A key still asks each time it's used.";
pub const VAULT_GRANTS_NONE: &str = "No shared logins or keys yet. Add one in Settings, Vault.";
/// A new agent's id is one a deleted bruh had, and that bruh's own vault
/// entries aren't deleted yet: the new agent would inherit them.
pub const BRUH_ID_PENDING_DELETE: &str =
    "A deleted bruh used this ID and its own vault entries aren't removed yet. Use another ID.";
const TOO_MANY_GRANTS: &str = "Grant at most 64 vault entries.";
const NOT_AN_ENTRY: &str = "That isn't a vault entry.";

/// Whether a service understands vault grants.
#[must_use]
pub fn vault_ready(protocol: u32) -> bool {
    protocol >= VAULT_PROTOCOL
}

/// Why a bruh's vault grants are off on this service: an older service's
/// update notice once its version is known, and `None` while grants work or
/// the version is still unknown.
#[must_use]
pub fn vault_reason(protocol: Option<u32>) -> Option<&'static str> {
    let protocol = protocol.filter(|protocol| !vault_ready(*protocol))?;
    service_notice(protocol, None).map(|notice| notice.message())
}

fn grant_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_GRANT_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Grants as a sorted set, without checking them.
pub(super) fn grant_set(entries: &[String]) -> Vec<String> {
    let mut set = entries.to_vec();
    set.sort();
    set.dedup();
    set
}

/// The grants a bruh stores: sorted, deduped, each an entry id
/// (`[A-Za-z0-9_-]`, 1 to 80 characters), at most [`MAX_VAULT_GRANTS`].
///
/// # Errors
///
/// The owner-facing reason when an id isn't an entry id or there are too
/// many.
pub fn normalize_grants(entries: Vec<String>) -> Result<Vec<String>, &'static str> {
    if !entries.iter().all(|id| grant_id_ok(id)) {
        return Err(NOT_AN_ENTRY);
    }
    let set = grant_set(&entries);
    if set.len() > MAX_VAULT_GRANTS {
        return Err(TOO_MANY_GRANTS);
    }
    Ok(set)
}

/// Whether bruh `agent_id` may use `entry` (D2): its own entries always,
/// Global ones it was granted, and nothing else.
#[must_use]
pub fn usable_by(entry: &VaultEntry, agent_id: &str, grants: &[String]) -> bool {
    match &entry.scope {
        CredentialScope::Bruh { agent_id: owner } => owner == agent_id,
        CredentialScope::Global => grants.contains(&entry.id),
        CredentialScope::Project { .. } => false,
    }
}

/// What a bruh's Settings offer: its own entries, always usable, and the
/// Global ones it can be granted. Each sorted by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VaultChoices {
    pub own: Vec<VaultEntry>,
    pub shared: Vec<VaultEntry>,
}

/// The choices for bruh `agent_id`. Project entries and other bruhs'
/// entries are never offered.
#[must_use]
pub fn vault_choices(entries: &[VaultEntry], agent_id: &str) -> VaultChoices {
    let mut choices = VaultChoices::default();
    for entry in entries {
        match &entry.scope {
            CredentialScope::Bruh { agent_id: owner } if owner == agent_id => {
                choices.own.push(entry.clone());
            }
            CredentialScope::Global => choices.shared.push(entry.clone()),
            CredentialScope::Bruh { .. } | CredentialScope::Project { .. } => {}
        }
    }
    let by_name = |a: &VaultEntry, b: &VaultEntry| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    };
    choices.own.sort_by(by_name);
    choices.shared.sort_by(by_name);
    choices
}

/// Whether a bruh's vault picks should follow the stored grants: while the
/// owner hasn't changed them since the form was filled (`edited == filled`),
/// or once the owner's picks match the stored ones. `Some` is then the
/// grants to show and to count as filled from, so Save never puts back
/// grants another app replaced. Compared as sets. Both apps use this rule
/// (as [`super::computer_switch_follows`]).
#[must_use]
pub fn vault_grants_follow(
    edited: &[String],
    filled: &[String],
    stored: &[String],
) -> Option<Vec<String>> {
    let edited = grant_set(edited);
    (edited == grant_set(filled) || edited == grant_set(stored)).then(|| stored.to_vec())
}

/// The line for one of the bruh's own entries.
#[must_use]
pub fn vault_own_line(name: &str) -> String {
    format!("{name}: its own")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SavedCredentialState;
    use crate::vault::VaultKind;

    #[test]
    fn a_bruh_uses_its_own_entries_and_granted_shared_ones() {
        let entry = |id: &str, scope| VaultEntry {
            id: id.into(),
            name: id.into(),
            scope,
            kind: VaultKind::Value,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        let own = entry(
            "v1-own",
            CredentialScope::Bruh {
                agent_id: "a".into(),
            },
        );
        let shared = entry("v1-g", CredentialScope::Global);
        let other = entry(
            "v1-b",
            CredentialScope::Bruh {
                agent_id: "b".into(),
            },
        );
        let project = entry("v1-p", CredentialScope::Project { path: "/p".into() });
        assert!(usable_by(&own, "a", &[]));
        assert!(!usable_by(&shared, "a", &[]) && usable_by(&shared, "a", &["v1-g".into()]));
        assert!(
            !usable_by(&other, "a", &["v1-b".into()])
                && !usable_by(&project, "a", &["v1-p".into()])
        );
        let choices = vault_choices(&[shared.clone(), own.clone(), other, project], "a");
        assert_eq!((choices.own, choices.shared), (vec![own], vec![shared]));
    }

    #[test]
    fn choices_are_sorted_by_name() {
        let entry = |id: &str, name: &str| VaultEntry {
            id: id.into(),
            name: name.into(),
            scope: CredentialScope::Global,
            kind: VaultKind::Value,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        let choices = vault_choices(
            &[
                entry("v1-1", "beta"),
                entry("v1-2", "Alpha"),
                entry("v1-3", "gamma"),
            ],
            "a",
        );
        let names: Vec<_> = choices.shared.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "beta", "gamma"]);
        assert!(choices.own.is_empty());
    }

    #[test]
    fn grants_are_bounded_and_follow_the_store_until_edited() {
        assert_eq!(
            normalize_grants(vec!["b".into(), "a".into(), "b".into()]),
            Ok(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            normalize_grants((0..65).map(|i| format!("v{i}")).collect()),
            Err("Grant at most 64 vault entries.")
        );
        assert_eq!(
            normalize_grants(vec!["../x".into()]),
            Err("That isn't a vault entry.")
        );
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            vault_grants_follow(&s(&["a"]), &s(&["a"]), &s(&["a", "b"])),
            Some(s(&["a", "b"]))
        );
        assert_eq!(
            vault_grants_follow(&s(&[]), &s(&["a"]), &s(&["a", "b"])),
            None
        );
        // Picked in another order: still the same grants.
        assert_eq!(
            vault_grants_follow(&s(&["b", "a"]), &s(&[]), &s(&["a", "b"])),
            Some(s(&["a", "b"]))
        );
    }

    #[test]
    fn grant_ids_are_entry_ids() {
        let legacy = "0123456789abcdef".repeat(4);
        let fresh = format!("v1-{}", "f".repeat(32));
        assert_eq!(
            normalize_grants(vec![legacy.clone(), fresh.clone()]),
            Ok(vec![legacy, fresh])
        );
        for bad in ["", "a b", "a/b", "é", &"x".repeat(81)] {
            assert_eq!(
                normalize_grants(vec![bad.to_string()]),
                Err(NOT_AN_ENTRY),
                "{bad:?}"
            );
        }
        assert!(normalize_grants(vec!["x".repeat(80)]).is_ok());
        // 64 distinct grants, repeated, still fit.
        let many: Vec<String> = (0..64).map(|i| format!("v{i}")).collect();
        let twice = many.iter().chain(&many).cloned().collect();
        assert_eq!(normalize_grants(twice).map(|set| set.len()), Ok(64));
    }

    #[test]
    fn vault_copy_reads_plainly() {
        assert_eq!(vault_own_line("Deploy key"), "Deploy key: its own");
        for copy in [VAULT_GRANTS_LABEL, VAULT_GRANTS_HINT, VAULT_GRANTS_NONE] {
            assert!(
                !copy.contains(['–', '—', '•', '·']) && !copy.contains(" - "),
                "{copy}"
            );
        }
    }
}
