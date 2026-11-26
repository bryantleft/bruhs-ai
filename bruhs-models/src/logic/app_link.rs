//! `bruhs://` links open a thread, a view by its web route, or a prompt draft.

use crate::imports::is_hidden_char;
use crate::AppView;

mod targets;
pub use targets::{PromptModel, PromptPullRequest, PromptRepository};

/// A thread's link, which ⌘⇧C copies.
pub const THREAD_LINK_PREFIX: &str = "bruhs://thread/";
/// A view's link is this followed by its route.
pub const VIEW_LINK_PREFIX: &str = "bruhs://open";
const NEW_PROMPT_LINK: &str = "bruhs://new";
const HOSTED_PROMPT_LINK: &str = "https://bruhs.ai/prompt#";
pub const MAX_LINKED_PROMPT_CHARS: usize = 8_000;
pub const PULL_CHECKOUT_SETUP_NOTICE: &str =
    "Bruhs runs any saved worktree setup commands in the new checkout.";
const MAX_PROMPT_QUERY_BYTES: usize = 128 * 1024;

/// What a `bruhs://` link opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLink {
    Thread(String),
    View(AppView),
}

/// Draft text supplied by a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedPrompt {
    pub text: String,
    pub removed_hidden_characters: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptLink {
    pub destination: PromptDestination,
    pub prompt: Option<LinkedPrompt>,
}

/// A draft's placement and optional model preference. Access and execution
/// settings always come from the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptDestination {
    Thread(String),
    New {
        repository: Option<PromptRepository>,
        model: Option<PromptModel>,
    },
    PullRequest(PromptPullRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PromptLinkError {
    #[error("The link has no prompt.")]
    Missing,
    #[error("Prompt links support up to 8,000 characters.")]
    TooLong,
    #[error("The link's prompt contains only hidden characters.")]
    HiddenOnly,
    #[error("The link contains more than one prompt.")]
    Duplicate,
    #[error("The link has an unreadable repository. Use a GitHub URL or owner/repository.")]
    Repository,
    #[error("The link has an unreadable pull request. Use a GitHub pull request URL or owner/repository#number.")]
    PullRequest,
    #[error("The link has an unreadable model preference.")]
    Model,
    #[error("The link contains more than one value for a destination field.")]
    DuplicateTarget,
}

/// Share text in a new projectless draft on the recipient's device.
pub fn new_prompt_link(text: &str) -> Result<String, PromptLinkError> {
    prompt_link_with_route(NEW_PROMPT_LINK, text, None)
}

/// Share a draft through the hosted handoff page. The prompt stays in the
/// fragment so the website never receives it in an HTTP request.
pub fn hosted_prompt_link(text: &str) -> Result<String, PromptLinkError> {
    prompt_link_with_route("https://bruhs.ai/prompt#/new", text, None)
}

pub struct PromptShareContext<'a> {
    pub repository: &'a PromptRepository,
    pub provider: crate::agent::AgentProvider,
    pub model: &'a str,
}

pub fn hosted_prompt_link_with_context(
    text: &str,
    context: PromptShareContext<'_>,
) -> Result<String, PromptLinkError> {
    prompt_link_with_route("https://bruhs.ai/prompt#/new", text, Some(context))
}

pub fn new_prompt_link_with_context(
    text: &str,
    context: PromptShareContext<'_>,
) -> Result<String, PromptLinkError> {
    prompt_link_with_route(NEW_PROMPT_LINK, text, Some(context))
}

fn prompt_link_with_route(
    route: &str,
    text: &str,
    context: Option<PromptShareContext<'_>>,
) -> Result<String, PromptLinkError> {
    validate_prompt(text)?;
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("prompt", text);
    if let Some(context) = context {
        let provider = context.provider.as_str();
        if let Some(model) = PromptModel::parse(Some(provider), Some(context.model))? {
            query
                .append_pair("repo", &context.repository.slug())
                .append_pair("provider", provider);
            if let Some(id) = model.id {
                query.append_pair("model", &id);
            }
        }
    }
    let query = query.finish();
    Ok(format!("{route}?{query}"))
}

/// Parse a draft link independently of normal thread and view navigation.
pub fn parse_prompt_link(link: &str) -> Result<Option<PromptLink>, PromptLinkError> {
    let link = link.trim();
    let (link, new_route, thread_prefix) = match link.strip_prefix(HOSTED_PROMPT_LINK) {
        Some(fragment) => (fragment, "/new", "thread/"),
        None => (link, NEW_PROMPT_LINK, THREAD_LINK_PREFIX),
    };
    let link = link.split('#').next().unwrap_or_default();
    let (route, query) = link.split_once('?').unwrap_or((link, ""));
    let route = route.trim_end_matches('/');
    let thread_id = if route == new_route {
        None
    } else if let Some(id) = route.strip_prefix(thread_prefix) {
        if id.is_empty() || query.is_empty() {
            return Ok(None);
        }
        Some(id.to_owned())
    } else {
        return Ok(None);
    };
    if query.len() > MAX_PROMPT_QUERY_BYTES {
        return Err(PromptLinkError::TooLong);
    }
    let params: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
    let param = |key: &str| {
        let mut values = params
            .iter()
            .filter_map(|(name, value)| (name == key).then_some(value.as_ref()));
        let first = values.next();
        if values.next().is_some() {
            return Err(if key == "prompt" {
                PromptLinkError::Duplicate
            } else {
                PromptLinkError::DuplicateTarget
            });
        }
        Ok(first)
    };
    let raw = param("prompt")?;
    if thread_id.is_some() && raw.is_none() {
        return Ok(None);
    }
    let destination = if let Some(id) = thread_id {
        PromptDestination::Thread(id)
    } else if let Some(pr) = param("pr")?
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        PromptDestination::PullRequest(
            PromptPullRequest::parse(pr).ok_or(PromptLinkError::PullRequest)?,
        )
    } else {
        PromptDestination::New {
            repository: param("repo")?
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|repo| PromptRepository::parse(repo).ok_or(PromptLinkError::Repository))
                .transpose()?,
            model: PromptModel::parse(param("provider")?, param("model")?)?,
        }
    };
    let prompt = match validate_prompt(raw.unwrap_or_default()) {
        Ok(prompt) => Some(prompt),
        Err(PromptLinkError::Missing)
            if matches!(destination, PromptDestination::PullRequest(_)) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    Ok(Some(PromptLink {
        destination,
        prompt,
    }))
}

fn validate_prompt(raw: &str) -> Result<LinkedPrompt, PromptLinkError> {
    if raw.chars().take(MAX_LINKED_PROMPT_CHARS + 1).count() > MAX_LINKED_PROMPT_CHARS {
        return Err(PromptLinkError::TooLong);
    }
    let normalized = raw
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n");
    let mut removed = false;
    let mut visible = normalized
        .chars()
        .filter(|&character| {
            if matches!(character, '\n' | '\t' | '\u{200d}') {
                return true;
            }
            let hidden = is_hidden_char(character)
                || matches!(character,
                '\u{034f}' | '\u{115f}'..='\u{1160}' | '\u{17b4}'..='\u{17b5}'
                | '\u{180b}'..='\u{180f}' | '\u{3164}' | '\u{fe00}'..='\u{fe0d}'
                | '\u{ffa0}' | '\u{fff0}'..='\u{fffb}' | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}' | '\u{e0000}'..='\u{e0fff}');
            removed |= hidden;
            !hidden
        })
        .peekable();
    let mut previous = None;
    let mut removed_joiner = false;
    let mut text = String::new();
    while let Some(character) = visible.next() {
        // An emoji joiner can change a visible glyph. In other contexts it can hide text.
        if character != '\u{200d}'
            || (previous.is_some_and(is_emoji_part)
                && visible.peek().copied().is_some_and(is_emoji_part))
        {
            text.push(character);
        } else {
            removed_joiner = true;
        }
        previous = Some(character);
    }
    let removed_hidden_characters = removed || removed_joiner;
    let text = text.trim();
    if text.is_empty() {
        return Err(if removed_hidden_characters {
            PromptLinkError::HiddenOnly
        } else {
            PromptLinkError::Missing
        });
    }
    Ok(LinkedPrompt {
        text: text.to_owned(),
        removed_hidden_characters,
    })
}

fn is_emoji_part(character: char) -> bool {
    matches!(character, '\u{1f000}'..='\u{1faff}' | '\u{2190}'..='\u{2bff}' | '\u{fe0f}')
}

/// The link to the thread `id`.
pub fn thread_link(id: &str) -> String {
    format!("{THREAD_LINK_PREFIX}{id}")
}

/// The link to `view`.
pub fn view_link(view: &AppView) -> String {
    format!("{VIEW_LINK_PREFIX}{}", view.route())
}

/// What `link` opens, or `None` for a link this build does not know.
pub fn parse_app_link(link: &str) -> Option<AppLink> {
    let link = link.trim();
    if let Some(rest) = link.strip_prefix(THREAD_LINK_PREFIX) {
        let id = rest.trim_end_matches('/');
        return (!id.is_empty()).then(|| AppLink::Thread(id.to_string()));
    }
    let route = link
        .strip_prefix(VIEW_LINK_PREFIX)
        .filter(|route| route.starts_with('/'))?;
    AppView::from_route(route).map(AppLink::View)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_threads_and_views_by_their_web_routes() {
        assert_eq!(
            parse_app_link(&thread_link("t-1")),
            Some(AppLink::Thread("t-1".into()))
        );
        assert_eq!(
            parse_app_link(" bruhs://thread/t-1/ "),
            Some(AppLink::Thread("t-1".into()))
        );
        let view = AppView::ConversationDetail {
            conversation_id: "c-1".into(),
        };
        assert_eq!(view_link(&view), "bruhs://open/inbox/c-1");
        assert_eq!(parse_app_link(&view_link(&view)), Some(AppLink::View(view)));
        assert_eq!(
            parse_app_link("bruhs://open/workbench"),
            Some(AppLink::View(AppView::Workbench))
        );
        for unknown in [
            "bruhs://thread/",
            "bruhs://open",
            "bruhs://openinbox",
            "bruhs://open/nowhere",
            "https://bruhs.ai/#/inbox",
        ] {
            assert_eq!(parse_app_link(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn draft_links_round_trip_punctuation_newlines_and_emoji() {
        let text = "First & second? #42\n👩‍💻 + 50%";
        let link = new_prompt_link(text).unwrap();
        assert_eq!(
            parse_prompt_link(&link),
            Ok(Some(PromptLink {
                destination: PromptDestination::New {
                    repository: None,
                    model: None
                },
                prompt: Some(LinkedPrompt {
                    text: text.into(),
                    removed_hidden_characters: false,
                }),
            }))
        );
    }

    #[test]
    fn hosted_prompt_is_an_exact_origin_fragment_link() {
        let link = hosted_prompt_link("Café & review?").unwrap();
        assert_eq!(
            link,
            "https://bruhs.ai/prompt#/new?prompt=Caf%C3%A9+%26+review%3F"
        );
        assert_eq!(
            parse_prompt_link(&link),
            parse_prompt_link("bruhs://new?prompt=Caf%C3%A9+%26+review%3F")
        );
        assert_eq!(
            parse_prompt_link("https://other.example/prompt#/new?prompt=x"),
            Ok(None)
        );
    }

    #[test]
    fn contextual_share_round_trips_the_repository_and_model_without_local_paths() {
        let repo =
            PromptRepository::from_remote_identity("remote:github.com/Example/Docs").unwrap();
        let link = hosted_prompt_link_with_context(
            "Review & refine",
            PromptShareContext {
                repository: &repo,
                provider: crate::agent::AgentProvider::Codex,
                model: "gpt-5.4",
            },
        )
        .unwrap();
        assert!(link.starts_with("https://bruhs.ai/prompt#/new?prompt="));
        let link = parse_prompt_link(&link).unwrap().unwrap();
        assert_eq!(
            link.destination,
            PromptDestination::New {
                repository: Some(repo),
                model: Some(PromptModel {
                    provider: Some("codex".into()),
                    id: Some("gpt-5.4".into())
                })
            }
        );
        assert_eq!(link.prompt.unwrap().text, "Review & refine");
    }

    #[test]
    fn draft_links_remove_hidden_text_and_ignore_execution_options() {
        assert_eq!(
            parse_prompt_link("bruhs://thread/t-1/?prompt=Read%0D%0Afirst%E2%80%8B&permission=full&branch=other#fragment"),
            Ok(Some(PromptLink {
                destination: PromptDestination::Thread("t-1".into()),
                prompt: Some(LinkedPrompt {
                    text: "Read\nfirst".into(),
                    removed_hidden_characters: true,
                }),
            }))
        );
        assert_eq!(parse_prompt_link("bruhs://thread/t-1"), Ok(None));
        assert_eq!(parse_prompt_link("bruhs://open/inbox?prompt=x"), Ok(None));
    }

    #[test]
    fn repository_and_model_preferences_remain_separate_from_draft_text() {
        let link = parse_prompt_link("bruhs://new?prompt=Review&repo=https%3A%2F%2Fgithub.com%2FExample%2Fapp.git&provider=CLAUDE&model=claude-sonnet-4-6&permission=bypass").unwrap().unwrap();
        let PromptDestination::New {
            repository: Some(repository),
            model: Some(model),
        } = link.destination
        else {
            panic!("expected a project and model")
        };
        assert_eq!(link.prompt.unwrap().text, "Review");
        assert_eq!(repository.slug(), "Example/app");
        assert!(repository.matches_remote("remote:github.com/example/APP"));
        assert!(!repository.matches_remote("remote:github.com.example/example/APP"));
        assert!(!repository.matches_remote("remote:github.com/example/APP/another"));
        assert_eq!(model.provider.as_deref(), Some("claude"));
        assert_eq!(model.id.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn pull_requests_take_precedence_and_can_open_without_a_draft() {
        let link = parse_prompt_link(
            "https://bruhs.ai/prompt#/new?pr=Example%2Fapp%2342&repo=other%2Fapp&provider=codex",
        )
        .unwrap()
        .unwrap();
        let PromptDestination::PullRequest(pr) = link.destination else {
            panic!("expected a pull request")
        };
        assert_eq!(pr.url(), "https://github.com/Example/app/pull/42");
        assert_eq!(link.prompt, None);
        for invalid in [
            "Example/app/0",
            "Example/app/9007199254740992",
            "https://github.com.example/Example/app/pull/42",
            "https://github.com/Example/app/issues/42",
        ] {
            assert!(PromptPullRequest::parse(invalid).is_none(), "{invalid}");
        }
        assert_eq!(
            parse_prompt_link("bruhs://new?prompt=review&repo=a%2Fx&repo=b%2Fx"),
            Err(PromptLinkError::DuplicateTarget)
        );
    }

    #[test]
    fn draft_links_reject_empty_oversize_hidden_and_ambiguous_payloads() {
        for (link, error) in [
            ("bruhs://new?prompt=+", PromptLinkError::Missing),
            ("bruhs://new?prompt=%E2%80%8D", PromptLinkError::HiddenOnly),
            (
                "bruhs://new?prompt=one&prompt=two",
                PromptLinkError::Duplicate,
            ),
        ] {
            assert_eq!(parse_prompt_link(link), Err(error));
        }
        assert!(new_prompt_link(&"😃".repeat(MAX_LINKED_PROMPT_CHARS)).is_ok());
        assert_eq!(
            new_prompt_link(&"a".repeat(MAX_LINKED_PROMPT_CHARS + 1)),
            Err(PromptLinkError::TooLong)
        );
    }
}
