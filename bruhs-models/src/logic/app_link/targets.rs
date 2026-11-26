use super::PromptLinkError;
use crate::agent::AgentProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRepository {
    owner: String,
    name: String,
}

impl PromptRepository {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let slug = github_path(raw).map_or(raw, |path| path.trim_end_matches('/'));
        let (owner, name) = slug.split_once('/')?;
        Self::from_parts(owner, name)
    }

    fn from_parts(owner: &str, name: &str) -> Option<Self> {
        let suffix = name.len().saturating_sub(4);
        let name = if name
            .get(suffix..)
            .is_some_and(|s| s.eq_ignore_ascii_case(".git"))
        {
            &name[..suffix]
        } else {
            name
        };
        if !(1..=39).contains(&owner.len())
            || !owner.as_bytes()[0].is_ascii_alphanumeric()
            || !owner
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !(1..=100).contains(&name.len())
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || name.bytes().all(|byte| byte == b'.')
        {
            return None;
        }
        Some(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}.git", self.slug())
    }

    /// Resolve a local checkout's normalized Git origin without contacting it.
    pub fn from_remote_identity(identity: &str) -> Option<Self> {
        let (host, slug) = identity
            .strip_prefix("remote:")
            .and_then(|value| value.split_once('/'))?;
        if !matches!(
            host,
            "github.com" | "www.github.com" | "git.cafe" | "staging.git.cafe"
        ) {
            return None;
        }
        let (owner, name) = slug.split_once('/')?;
        Self::from_parts(owner, name)
    }

    pub fn matches_remote(&self, identity: &str) -> bool {
        Self::from_remote_identity(identity).is_some_and(|other| self.same_repository(&other))
    }

    fn same_repository(&self, other: &Self) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptPullRequest {
    pub repository: PromptRepository,
    pub number: u64,
}

impl PromptPullRequest {
    pub fn checkout_branch(&self, checkout_id: uuid::Uuid) -> String {
        format!("bruhs/pr-{}-{checkout_id}", self.number)
    }

    pub fn matches(&self, other: &Self) -> bool {
        self.number == other.number && self.repository.same_repository(&other.repository)
    }
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (repository, number) = if let Some(path) = github_path(raw) {
            let mut parts = path.split('/');
            let repository = PromptRepository::from_parts(parts.next()?, parts.next()?)?;
            if !parts.next()?.eq_ignore_ascii_case("pull") {
                return None;
            }
            let number = parts.next()?.split(['?', '#']).next()?;
            (repository, number)
        } else {
            let (slug, number) = raw.rsplit_once(['/', '#'])?;
            (PromptRepository::parse(slug)?, number)
        };
        if !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let number = number.parse().ok()?;
        (1..=9_007_199_254_740_991)
            .contains(&number)
            .then_some(Self { repository, number })
    }

    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/pull/{}",
            self.repository.slug(),
            self.number
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptModel {
    pub provider: Option<String>,
    pub id: Option<String>,
}

impl PromptModel {
    pub(crate) fn parse(
        provider: Option<&str>,
        id: Option<&str>,
    ) -> Result<Option<Self>, PromptLinkError> {
        let provider = provider.map(str::trim).filter(|value| !value.is_empty());
        let id = id.map(str::trim).filter(|value| !value.is_empty());
        if provider.is_some_and(|value| {
            value.len() > 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        }) || id.is_some_and(|value| {
            value.len() > 256
                || value.chars().any(|ch| {
                    ch.is_control() || ch.is_whitespace() || crate::imports::is_hidden_char(ch)
                })
        }) {
            return Err(PromptLinkError::Model);
        }
        Ok((provider.is_some() || id.is_some()).then(|| Self {
            provider: provider.map(str::to_ascii_lowercase),
            id: id.map(str::to_owned),
        }))
    }

    /// Select the requested model or a listed default of the requested provider.
    /// The caller supplies the models available in this workspace.
    pub fn resolve<'a>(
        &self,
        models: impl IntoIterator<Item = (AgentProvider, &'a str)>,
    ) -> (Option<&'a str>, bool) {
        let provider = match self.provider.as_deref() {
            Some("claudeagent") => Some(AgentProvider::Claude),
            Some(raw) => match raw.parse() {
                Ok(provider) => Some(provider),
                Err(_) => return (None, false),
            },
            None => None,
        };
        let mut default = None;
        for (choice_provider, choice) in models {
            if provider.is_some_and(|provider| provider != choice_provider) {
                continue;
            }
            if self.id.as_deref().is_some_and(|id| {
                choice == id || (provider.is_some() && choice_provider.model_id(choice) == id)
            }) {
                return (Some(choice), true);
            }
            if provider.is_some() && default.is_none() {
                default = Some(choice);
            }
        }
        (default, default.is_some() && self.id.is_none())
    }
}

fn github_path(raw: &str) -> Option<&str> {
    let (scheme, address) = raw.split_once("://")?;
    let (host, path) = address.split_once('/')?;
    ((scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && (host.eq_ignore_ascii_case("github.com") || host.eq_ignore_ascii_case("www.github.com")))
    .then_some(path)
}
