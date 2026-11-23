//! Match quota observations across sources without treating an auth-file name as identity.
use super::{AccountUsage, Hub};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct Identity {
    pub email: Option<String>,
    pub fingerprint: Option<String>,
    pub workspace: Option<String>,
}
impl Identity {
    fn key(&self, provider: &'static str) -> Option<Base> {
        if let Some(email) = self
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(Base::Email(provider, email.to_lowercase()))
        } else {
            self.fingerprint
                .as_ref()
                .filter(|id| !id.trim().is_empty())
                .map(|id| Base::Fingerprint(provider, id.clone()))
        }
    }
}
#[derive(Clone, Eq, Hash, PartialEq)]
enum Base {
    Email(&'static str, String),
    Fingerprint(&'static str, String),
}

#[derive(Clone, Copy)]
pub enum Origin<'a> {
    Device,
    Hub(&'a Hub),
    Environment(&'a str),
}
impl Origin<'_> {
    pub fn label(&self) -> &str {
        match self {
            Self::Device => "This computer",
            Self::Hub(h) => &h.label,
            Self::Environment(name) => name,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Reading<'a> {
    pub origin: Origin<'a>,
    pub account: &'a AccountUsage,
}
pub struct AccountGroup<'a> {
    pub readings: Vec<Reading<'a>>,
    pub quota: Reading<'a>,
    pub credits: Option<Reading<'a>>,
    pub workspace: Option<&'a str>,
    pub show_workspace: bool,
}
impl AccountGroup<'_> {
    pub fn email(&self) -> Option<&str> {
        self.readings
            .iter()
            .find_map(|r| r.account.identity.email.as_deref())
    }

    pub fn latest_plan(&self) -> Option<&str> {
        self.readings
            .iter()
            .filter_map(|r| Some((r.account.checked_at, r.account.plan.as_deref()?)))
            .max_by_key(|(at, _)| *at)
            .map(|(_, plan)| plan)
    }
}
fn rank(a: &AccountUsage) -> (u8, i64) {
    let quality = match &a.windows {
        Ok(w) if !w.is_empty() => 2,
        Ok(_) => 1,
        Err(_) => 0,
    };
    (quality, a.checked_at)
}
pub fn group_accounts(readings: Vec<Reading<'_>>) -> Vec<AccountGroup<'_>> {
    let mut spaces: HashMap<Base, BTreeSet<&str>> = HashMap::new();
    for reading in &readings {
        let a = reading.account;
        if let (Some(key), Some(workspace)) =
            (a.identity.key(a.provider), a.identity.workspace.as_deref())
        {
            spaces.entry(key).or_default().insert(workspace);
        }
    }
    let mut indices = HashMap::new();
    let mut groups: Vec<AccountGroup<'_>> = vec![];
    for reading in readings {
        let a = reading.account;
        let base = a.identity.key(a.provider);
        let known = base.as_ref().and_then(|key| spaces.get(key));
        let ambiguous = known.is_some_and(|s| s.len() > 1);
        let workspace = a
            .identity
            .workspace
            .as_deref()
            .or_else(|| known.filter(|s| s.len() == 1)?.first().copied());
        // A report without a workspace can join only one possible workspace.
        let key = base
            .filter(|_| workspace.is_some() || !ambiguous)
            .map(|base| (base, workspace));
        let index = key.map_or(groups.len(), |key| {
            *indices.entry(key).or_insert(groups.len())
        });
        if index == groups.len() {
            groups.push(AccountGroup {
                readings: vec![],
                quota: reading,
                credits: None,
                workspace,
                show_workspace: ambiguous,
            });
        }
        // Preserve source order for identical timestamps.
        let group = &mut groups[index];
        if rank(a) > rank(group.quota.account) {
            group.quota = reading;
        }
        if a.credits.is_some()
            && group
                .credits
                .is_none_or(|old| a.checked_at > old.account.checked_at)
        {
            group.credits = Some(reading);
        }
        group.readings.push(reading);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_hubs::ResetCredits;
    use bruhs_models::agent_extensions::QuotaWindow;
    fn account(
        provider: &'static str,
        email: Option<&str>,
        space: Option<&str>,
        at: i64,
    ) -> AccountUsage {
        AccountUsage {
            zero_spend_cap: false,
            external: None,
            plan: None,
            id: "same-name.json".into(),
            label: "Same label".into(),
            provider,
            checked_at: at,
            identity: Identity {
                email: email.map(str::to_owned),
                workspace: space.map(str::to_owned),
                fingerprint: None,
            },
            windows: Ok(vec![QuotaWindow {
                window_id: None,
                provider: provider.into(),
                account: "current".into(),
                name: "session".into(),
                duration_minutes: Some(300),
                used_percent: 25.,
                observed_at: at,
                resets_at: None,
            }]),
            credits: None,
        }
    }
    fn grouped<'a>(accounts: &'a [&'a AccountUsage]) -> Vec<AccountGroup<'a>> {
        group_accounts(
            accounts
                .iter()
                .map(|account| Reading {
                    origin: Origin::Device,
                    account,
                })
                .collect(),
        )
    }
    #[test]
    fn plan_uses_the_latest_report_that_identifies_it() {
        let mut old = account("Codex", Some("demo@example.test"), None, 10);
        old.plan = Some("plus".into());
        let mut upgraded = account("Codex", Some("demo@example.test"), None, 20);
        upgraded.plan = Some("pro".into());
        let local = account("Codex", Some("demo@example.test"), None, 30);
        let inputs = [&upgraded, &local, &old];
        let groups = grouped(&inputs);
        assert!(std::ptr::eq(groups[0].quota.account, &local));
        assert_eq!(groups[0].latest_plan(), Some("pro"));
    }

    #[test]
    fn plain_email_joins_exactly_one_workspace_and_credits_keep_their_source() {
        let local = account("Codex", Some(" Demo@example.test "), None, 30);
        let mut hub = account("Codex", Some("demo@EXAMPLE.test"), Some("work"), 20);
        hub.credits = Some(ResetCredits {
            count: 0,
            next: None,
        });
        let mut failed = account("Codex", Some("demo@example.test"), Some("work"), 40);
        failed.windows = Err(std::io::Error::other("Unavailable"));
        let inputs = [&local, &hub, &failed];
        let g = grouped(&inputs);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].readings.len(), 3);
        assert_eq!(g[0].workspace, Some("work"));
        assert!(std::ptr::eq(g[0].quota.account, &local));
        assert!(std::ptr::eq(g[0].credits.unwrap().account, &hub));
    }
    #[test]
    fn ambiguous_workspaces_unverified_names_and_different_drivers_remain_separate() {
        let left = account("Codex", Some("a"), Some("left"), 0);
        let right = account("Codex", Some("a"), Some("right"), 0);
        let unspecified = account("Codex", Some("a"), None, 0);
        let other = account("Claude", Some("a"), Some("left"), 0);
        let blank = account("Codex", None, None, 0);
        let alias = account("Codex", None, None, 0);
        for inputs in [
            [&unspecified, &left, &right, &other, &blank, &alias],
            [&right, &left, &unspecified, &other, &alias, &blank],
        ] {
            let groups = grouped(&inputs);
            assert_eq!(groups.len(), 6);
            assert!(groups[..3].iter().all(|g| g.show_workspace));
        }
    }
    #[test]
    fn linked_usage_coalesces_verified_accounts_and_preserves_unknown_accounts() {
        let local = account("Codex", Some("me@example.test"), Some("studio"), 2);
        let remote = account("Codex", Some("ME@example.test"), None, 3);
        let mut unknown = account("Codex", Some(" "), None, 3);
        unknown.identity.fingerprint = Some(String::new());
        let groups = group_accounts(vec![
            Reading {
                origin: Origin::Device,
                account: &local,
            },
            Reading {
                origin: Origin::Environment("Studio"),
                account: &remote,
            },
            Reading {
                origin: Origin::Environment("Studio"),
                account: &unknown,
            },
            Reading {
                origin: Origin::Environment("Build"),
                account: &unknown,
            },
        ]);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].quota.origin.label(), "Studio");
        assert_eq!(groups[0].workspace, Some("studio"));
        assert_eq!(groups[0].readings.len(), 2);
    }

    #[test]
    fn equal_fingerprints_match_only_within_the_same_provider() {
        let mut first = account("Antigravity", None, None, 0);
        first.identity.fingerprint = Some("abc".into());
        let mut next = account("Antigravity", None, None, 1);
        next.identity.fingerprint = Some("abc".into());
        next.id = "other.json".into();
        let mut different = account("Grok", None, None, 2);
        different.identity.fingerprint = Some("abc".into());
        let inputs = [&first, &next, &different];
        let groups = grouped(&inputs);
        assert_eq!(groups.len(), 2);
        assert!(std::ptr::eq(groups[0].quota.account, &next));
    }
}
