//! Vault grants on a bruh: which entries it may use, the choices its
//! Settings offer, protocol 4 and the copy both apps show (D2, D3, D23).
//! The entries themselves live in [`crate::vault`].

use super::{service_notice, AccessLevel, VAULT_PROTOCOL};
use crate::agent::AgentProvider;
use crate::secrets::review_text;
use crate::vault::{
    vault_row, CredentialScope, VaultEntry, VaultKind, VaultListing, KIND_APP, KIND_KEY,
    KIND_LOGIN, KIND_ONE_PASSWORD, KIND_OTHER,
};

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
pub const VAULT_NEEDS_CLAUDE: &str = "The vault needs a Claude model.";
pub const VAULT_NEEDS_ASK_OR_FULL: &str = "The vault needs Ask first or Full access.";
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

/// Why a bruh at `level` on `provider` can't use vault entries, if it
/// can't: logins fill only in a Computer and keys go through a Claude
/// tool at Ask or Full (D18), so neither reaches Codex or Read only.
#[must_use]
pub fn vault_unavailable(level: AccessLevel, provider: AgentProvider) -> Option<&'static str> {
    if provider != AgentProvider::Claude {
        Some(VAULT_NEEDS_CLAUDE)
    } else if level == AccessLevel::ReadOnly {
        Some(VAULT_NEEDS_ASK_OR_FULL)
    } else {
        None
    }
}

pub(super) fn grant_id_ok(id: &str) -> bool {
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
/// Global ones it was granted, and nothing else. Never an app token, which
/// only `app_request` and app polls use (D8), nor a newer Bruhs' kind.
#[must_use]
pub fn usable_by(entry: &VaultEntry, agent_id: &str, grants: &[String]) -> bool {
    if !vault_grantable(&entry.kind) {
        return false;
    }
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

/// Whether a bruh's Vault section deals with this kind: not an app token
/// (the Apps section's) nor a newer Bruhs' kind.
fn vault_grantable(kind: &VaultKind) -> bool {
    !matches!(kind, VaultKind::AppToken(_) | VaultKind::Other)
}

/// The choices for bruh `agent_id`. Project entries, other bruhs' entries,
/// app tokens and newer kinds are never offered.
#[must_use]
pub fn vault_choices(entries: &[VaultEntry], agent_id: &str) -> VaultChoices {
    let mut choices = VaultChoices::default();
    for entry in entries.iter().filter(|entry| vault_grantable(&entry.kind)) {
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

/// The order a bruh's Settings list entries in: by name ignoring case,
/// then by name, then by id.
pub(super) fn entry_order(a: &VaultEntry, b: &VaultEntry) -> std::cmp::Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.id.cmp(&b.id))
}

/// How a shared entry reads in a bruh's Settings: its name, as the Vault
/// page shows it, and its kind.
#[must_use]
pub fn vault_choice_label(entry: &VaultEntry) -> String {
    let kind = match entry.kind {
        VaultKind::Value => KIND_KEY,
        VaultKind::Login(_) => KIND_LOGIN,
        VaultKind::OnePassword { .. } => KIND_ONE_PASSWORD,
        VaultKind::AppToken(_) => KIND_APP,
        VaultKind::Other => KIND_OTHER,
    };
    format!("{}, {kind}", review_text(&entry.name))
}

/// Whether a bruh's vault picks should follow the stored grants: while the
/// section is `off` (disabled), while the owner hasn't changed them since
/// the form was filled (`edited == filled`), or once the owner's picks
/// match the stored ones. `Some` is then the grants to show and to count as
/// filled from, so Save never puts back grants another app replaced, nor
/// sends a pick made before the section went off. Compared as sets. Both
/// apps use this rule (as [`super::computer_switch_follows`]).
#[must_use]
pub fn vault_grants_follow(
    edited: &[String],
    filled: &[String],
    stored: &[String],
    off: bool,
) -> Option<Vec<String>> {
    let edited = grant_set(edited);
    (off || edited == grant_set(filled) || edited == grant_set(stored)).then(|| stored.to_vec())
}

/// The line for one of the bruh's own entries.
#[must_use]
pub fn vault_own_line(name: &str) -> String {
    format!("{name}: its own")
}

/// One row of a bruh's Vault section: an entry it owns, as a line, or a
/// shared one it can be granted, as a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultGrantRow {
    Own { id: String, line: String },
    Shared { id: String, label: String, on: bool },
}

/// The rows for bruh `agent_id` with `grants` picked: its own entries,
/// then the shared ones, each in [`vault_choices`]' order. Names and kinds
/// read as on the Vault page. Both apps draw these.
#[must_use]
pub fn vault_grant_rows(
    listing: &VaultListing,
    agent_id: &str,
    grants: &[String],
) -> Vec<VaultGrantRow> {
    let choices = vault_choices(&listing.entries, agent_id);
    let own = choices.own.iter().map(|entry| VaultGrantRow::Own {
        id: entry.id.clone(),
        line: vault_own_line(&vault_row(entry, &listing.bruhs).name),
    });
    let shared = choices.shared.iter().map(|entry| VaultGrantRow::Shared {
        id: entry.id.clone(),
        label: vault_choice_label(entry),
        on: usable_by(entry, agent_id, grants),
    });
    own.chain(shared).collect()
}

/// Adds or removes one grant, keeping the set sorted as the service stores
/// it, so toggling back to the stored set leaves Save off.
pub fn toggle_vault_grant(grants: &mut Vec<String>, id: &str, on: bool) {
    grants.retain(|grant| grant != id);
    if on {
        grants.push(id.to_owned());
        grants.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SavedCredentialState;
    use crate::vault::{LoginFields, LoginMeta, VaultKind};

    fn listed(id: &str, name: &str, scope: CredentialScope, kind: VaultKind) -> VaultEntry {
        VaultEntry {
            id: id.into(),
            name: name.into(),
            scope,
            kind,
            state: SavedCredentialState::Saved,
            attention: None,
        }
    }

    fn login() -> VaultKind {
        VaultKind::Login(LoginMeta {
            url: "https://github.com/login".into(),
            fields: LoginFields::Saved {
                username: "me".into(),
                totp: false,
            },
        })
    }

    fn owned_by(agent_id: &str) -> CredentialScope {
        CredentialScope::Bruh {
            agent_id: agent_id.into(),
        }
    }

    /// The rows both apps draw in a bruh's Settings (V Task 15 minor).
    #[test]
    fn grant_rows_list_own_entries_then_shared_switches() {
        let listing = VaultListing {
            entries: vec![
                listed("v1-g", "GitHub", CredentialScope::Global, login()),
                listed("v1-s", "Shop", owned_by("a"), login()),
                listed("v1-k", "Deploy", CredentialScope::Global, VaultKind::Value),
                listed("v1-o", "Other", owned_by("b"), VaultKind::Value),
                listed(
                    "v1-p",
                    "Project",
                    CredentialScope::Project {
                        path: "/work".into(),
                    },
                    VaultKind::Value,
                ),
            ],
            ..VaultListing::default()
        };
        // Shared entries come in `vault_choices`' order, by name.
        assert_eq!(
            vault_grant_rows(&listing, "a", &["v1-g".into()]),
            vec![
                VaultGrantRow::Own {
                    id: "v1-s".into(),
                    line: "Shop: its own".into(),
                },
                VaultGrantRow::Shared {
                    id: "v1-k".into(),
                    label: "Deploy, Key".into(),
                    on: false,
                },
                VaultGrantRow::Shared {
                    id: "v1-g".into(),
                    label: "GitHub, Login".into(),
                    on: true,
                },
            ]
        );
        // Names show escaped, as on the Vault page.
        let odd = VaultListing {
            entries: vec![listed(
                "v1-x",
                "Ops\u{202e}",
                CredentialScope::Global,
                VaultKind::OnePassword {
                    reference: "op://Ops/Item/field".into(),
                },
            )],
            ..VaultListing::default()
        };
        assert_eq!(
            vault_grant_rows(&odd, "a", &[]),
            vec![VaultGrantRow::Shared {
                id: "v1-x".into(),
                label: "Ops\\u{202e}, 1Password".into(),
                on: false,
            }]
        );
    }

    /// An app token is granted on the Apps page and an entry from a newer
    /// Bruhs isn't understood here: neither is a Vault grant row.
    #[test]
    fn app_tokens_and_other_kinds_are_never_vault_grants() {
        let app = VaultKind::AppToken(crate::vault::AppTokenMeta {
            app: "github".into(),
            account: "me".into(),
            params: std::collections::BTreeMap::new(),
        });
        let listing = VaultListing {
            entries: vec![
                listed("v1-g", "GitHub", CredentialScope::Global, app.clone()),
                listed("v1-a", "Mine", owned_by("a"), app),
                listed("v1-x", "Newer", CredentialScope::Global, VaultKind::Other),
                listed("v1-y", "Newer own", owned_by("a"), VaultKind::Other),
            ],
            ..VaultListing::default()
        };
        let grants = ["v1-g".to_string(), "v1-x".to_string()];
        assert!(vault_grant_rows(&listing, "a", &grants).is_empty());
        assert!(listing
            .entries
            .iter()
            .all(|entry| !usable_by(entry, "a", &grants)));
    }

    #[test]
    fn a_grant_toggle_keeps_the_stored_order() {
        let mut grants = vec!["v1-k".to_string()];
        toggle_vault_grant(&mut grants, "v1-g", true);
        assert_eq!(grants, ["v1-g", "v1-k"]);
        toggle_vault_grant(&mut grants, "v1-g", true);
        assert_eq!(grants, ["v1-g", "v1-k"]);
        toggle_vault_grant(&mut grants, "v1-g", false);
        assert_eq!(grants, ["v1-k"]);
    }

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
            vault_grants_follow(&s(&["a"]), &s(&["a"]), &s(&["a", "b"]), false),
            Some(s(&["a", "b"]))
        );
        assert_eq!(
            vault_grants_follow(&s(&[]), &s(&["a"]), &s(&["a", "b"]), false),
            None
        );
        // Picked in another order: still the same grants.
        assert_eq!(
            vault_grants_follow(&s(&["b", "a"]), &s(&[]), &s(&["a", "b"]), false),
            Some(s(&["a", "b"]))
        );
        // A section that is off snaps back to the stored grants, so Save
        // never sends a pick made before it went off.
        assert_eq!(
            vault_grants_follow(&s(&["c"]), &s(&["a"]), &s(&["a", "b"]), true),
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
    fn a_choice_reads_as_its_name_and_kind() {
        let entry = |name: &str, kind| VaultEntry {
            id: "v1-x".into(),
            name: name.into(),
            scope: CredentialScope::Global,
            kind,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        let login = VaultKind::Login(crate::vault::LoginMeta {
            url: "https://github.com/login".into(),
            fields: crate::vault::LoginFields::Saved {
                username: "me".into(),
                totp: false,
            },
        });
        assert_eq!(vault_choice_label(&entry("GitHub", login)), "GitHub, Login");
        assert_eq!(
            vault_choice_label(&entry("Deploy", VaultKind::Value)),
            "Deploy, Key"
        );
        // Names show escaped, as on the Vault page.
        let reference = VaultKind::OnePassword {
            reference: "op://Ops/Item/field".into(),
        };
        assert_eq!(
            vault_choice_label(&entry("Ops\u{202e}", reference)),
            "Ops\\u{202e}, 1Password"
        );
    }

    #[test]
    fn only_claude_bruhs_at_ask_or_full_use_the_vault() {
        use crate::agent::AgentProvider;
        use crate::crew::AccessLevel;
        for level in [AccessLevel::Ask, AccessLevel::Full] {
            assert_eq!(vault_unavailable(level, AgentProvider::Claude), None);
        }
        assert_eq!(
            vault_unavailable(AccessLevel::ReadOnly, AgentProvider::Claude),
            Some(VAULT_NEEDS_ASK_OR_FULL)
        );
        // The model comes first, since a level change can't fix it.
        for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
            assert_eq!(
                vault_unavailable(level, AgentProvider::Codex),
                Some(VAULT_NEEDS_CLAUDE)
            );
        }
    }

    #[test]
    fn vault_copy_reads_plainly() {
        assert_eq!(vault_own_line("Deploy key"), "Deploy key: its own");
        for copy in [
            VAULT_GRANTS_LABEL,
            VAULT_GRANTS_HINT,
            VAULT_GRANTS_NONE,
            VAULT_NEEDS_CLAUDE,
            VAULT_NEEDS_ASK_OR_FULL,
        ] {
            assert!(
                !copy.contains(['–', '—', '•', '·']) && !copy.contains(" - "),
                "{copy}"
            );
        }
    }
}
