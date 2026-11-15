//! Models an agent runtime reports for the signed-in account, and the picker
//! list that merges them into the built-in [`MODEL_CATALOG`].
//!
//! The runtime CLI is the source of truth for which models an account can run
//! (Cursor's `--list-models`), so the pickers list them instead of a copy that
//! would drift. Each one is a `runtime/model` selection, which routes exactly
//! like a typed custom model.

use serde::{Deserialize, Serialize};

use crate::agent::{custom_model_selection, AgentProvider, ModelInfo, MODEL_CATALOG};

/// Runtimes whose CLI can list the account's models.
pub const DISCOVERABLE_PROVIDERS: [AgentProvider; 1] = [AgentProvider::Cursor];

/// One selectable model, owned so a runtime's reported models can sit beside
/// the static catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// The selection a thread stores (`cursor/gpt-5`).
    pub id: String,
    /// Name shown in pickers.
    pub label: String,
    pub provider: AgentProvider,
}

impl From<&ModelInfo> for ModelChoice {
    fn from(model: &ModelInfo) -> Self {
        Self {
            id: model.id.to_string(),
            label: model.label.to_string(),
            provider: model.provider,
        }
    }
}

/// `GET /api/agent/models/{provider}`: the account's models, or why they could
/// not be listed. An error never hides the catalog's default entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeModels {
    #[serde(default)]
    pub models: Vec<ModelChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The catalog in display order, with each runtime's `discovered` models placed
/// after that runtime's catalog entries. Duplicates of a catalog id, repeats,
/// and models filed under another runtime's prefix are dropped.
pub fn picker_models(discovered: &[ModelChoice]) -> Vec<ModelChoice> {
    let mut models: Vec<ModelChoice> = MODEL_CATALOG.iter().map(ModelChoice::from).collect();
    for model in discovered {
        let routed = custom_model_selection(&model.id)
            .is_some_and(|selection| selection.provider() == model.provider);
        if !routed || models.iter().any(|existing| existing.id == model.id) {
            continue;
        }
        let at = models
            .iter()
            .rposition(|existing| existing.provider == model.provider)
            .map_or(models.len(), |index| index + 1);
        models.insert(at, model.clone());
    }
    models
}

/// One runtime's listing, as each surface keeps it.
pub type Listing = (AgentProvider, RuntimeModels);

/// [`picker_models`] for every runtime's listing.
pub fn listed_models(listings: &[Listing]) -> Vec<ModelChoice> {
    let discovered: Vec<ModelChoice> = listings
        .iter()
        .flat_map(|(_, listing)| listing.models.iter().cloned())
        .collect();
    picker_models(&discovered)
}

/// Why `provider` could not list the account's models, as a picker footer line.
/// Models kept from an earlier listing are still offered, so the line says so.
pub fn listing_error_line(listings: &[Listing], provider: AgentProvider) -> Option<String> {
    let (_, listing) = listings.iter().find(|(listed, _)| *listed == provider)?;
    let error = listing.error.as_deref()?;
    Some(if listing.models.is_empty() {
        format!("{} models unavailable: {error}", provider.label())
    } else {
        format!("{} models may be out of date: {error}", provider.label())
    })
}

/// Replace `provider`'s listing. Returns whether anything changed.
pub fn replace_listing(
    listings: &mut Vec<Listing>,
    provider: AgentProvider,
    listing: RuntimeModels,
) -> bool {
    if listings
        .iter()
        .any(|(listed, previous)| *listed == provider && *previous == listing)
    {
        return false;
    }
    listings.retain(|(listed, _)| *listed != provider);
    listings.push((provider, listing));
    true
}

/// The model the picker highlight at `index` is on: a row's id, or the custom
/// row's after the rows.
pub fn highlighted_id<'a>(
    rows: &'a [ModelChoice],
    custom: Option<&'a str>,
    index: usize,
) -> Option<&'a str> {
    rows.get(index)
        .map(|row| row.id.as_str())
        .or_else(|| (index == rows.len()).then_some(custom).flatten())
}

/// Where the highlight goes after the rows change under it: onto the row that
/// now shows `previous` (the model it was on), else the custom row when that
/// offers `previous`, else `fallback` kept within the rows. Arriving rows never
/// move the highlight to another model.
pub fn reanchor_highlight(
    previous: Option<&str>,
    rows: &[ModelChoice],
    custom: Option<&str>,
    fallback: usize,
) -> usize {
    if let Some(previous) = previous {
        if let Some(index) = rows.iter().position(|row| row.id == previous) {
            return index;
        }
        if custom == Some(previous) {
            return rows.len();
        }
    }
    let count = rows.len() + usize::from(custom.is_some());
    fallback.min(count.saturating_sub(1))
}

/// Parse `cursor-agent --list-models` output:
///
/// ```text
/// Available models
///
/// auto - Auto (current, default)
/// gpt-5 - GPT-5
///
/// Tip: use --model <id> …
/// ```
///
/// Only lines between the header and the `Tip:` footer are read, and each must
/// be `id`, optionally followed by `- label` and `(current, default)` markers.
/// ANSI escapes are ignored and invalid model ids are skipped. `None` means the
/// output is not a listing (no header), so the caller can report it rather
/// than show an empty list; Cursor's "No models available" reply is `Some([])`.
pub fn parse_cursor_models(output: &str) -> Option<Vec<ModelChoice>> {
    let provider = AgentProvider::Cursor;
    let text = strip_ansi(output);
    let mut lines = text.lines().map(str::trim);
    if text
        .lines()
        .any(|line| line.trim().starts_with("No models available"))
    {
        return Some(Vec::new());
    }
    lines.find(|line| {
        line.trim_end_matches(':')
            .eq_ignore_ascii_case("Available models")
    })?;
    let mut models: Vec<ModelChoice> = Vec::new();
    for line in lines {
        if line.starts_with("Tip:") {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let (id, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = strip_markers(rest.trim());
        let label = match rest.strip_prefix('-') {
            Some(label) => label.trim(),
            None if rest.is_empty() => "",
            None => continue,
        };
        let Some(selection) = custom_model_selection(&format!("{}{id}", provider.model_prefix()))
        else {
            continue;
        };
        let id = selection.into_inner();
        if models.iter().any(|model| model.id == id) {
            continue;
        }
        models.push(ModelChoice {
            label: if label.is_empty() {
                id[provider.model_prefix().len()..].to_string()
            } else {
                label.to_string()
            },
            id,
            provider,
        });
    }
    Some(models)
}

/// `rest` without a trailing `(current)`, `(default)` or `(current, default)`.
fn strip_markers(rest: &str) -> &str {
    let Some(open) = rest
        .rfind('(')
        .filter(|&open| rest.ends_with(')') && (open == 0 || rest[..open].ends_with(' ')))
    else {
        return rest;
    };
    let markers = &rest[open + 1..rest.len() - 1];
    let only_markers = markers
        .split(',')
        .all(|marker| matches!(marker.trim(), "current" | "default"));
    if only_markers {
        rest[..open].trim_end()
    } else {
        rest
    }
}

/// `text` without terminal escape sequences: CSI (`ESC [ … final byte`), OSC
/// (`ESC ] … BEL` or `ESC ] … ESC \`, as in hyperlinks) and two-byte escapes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A short, readable reason the runtime's models are unavailable, from a failed
/// `--list-models` run's stderr.
pub fn cursor_list_error(stderr: &str) -> String {
    let text = strip_ansi(stderr);
    if text.contains("unknown option") {
        return "This Cursor CLI cannot list models. Run `cursor-agent update`.".to_string();
    }
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Cursor could not list models.");
    let line = line.strip_prefix("Error:").map_or(line, str::trim);
    let mut message: String = line.chars().take(200).collect();
    if message.len() < line.len() {
        message.push('…');
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(id: &str, label: &str) -> ModelChoice {
        ModelChoice {
            id: format!("cursor/{id}"),
            label: label.to_string(),
            provider: AgentProvider::Cursor,
        }
    }

    #[test]
    fn parses_the_cli_listing_with_colour_and_markers() {
        let output = "\u{1b}[2mAvailable models\u{1b}[22m\n\n\
            \u{1b}[32mauto\u{1b}[39m \u{1b}[2m- Auto\u{1b}[22m\u{1b}[2m (current, default)\u{1b}[22m\n\
            \u{1b}[36mgpt-5\u{1b}[39m \u{1b}[2m- GPT-5\u{1b}[22m\n\
            claude-opus-4-8 - Claude 4.8 Opus (Thinking)\n\
            composer-2\n\n\
            \u{1b}[2mTip: use \u{1b}[36m--model <id>\u{1b}[39m to switch.\u{1b}[22m\n";
        assert_eq!(
            parse_cursor_models(output),
            Some(vec![
                cursor("auto", "Auto"),
                cursor("gpt-5", "GPT-5"),
                cursor("claude-opus-4-8", "Claude 4.8 Opus (Thinking)"),
                cursor("composer-2", "composer-2"),
            ])
        );
    }

    #[test]
    fn reads_only_listing_lines_between_the_header_and_footer() {
        let output = "Loading models…\nAvailable models:\n\n\
            auto (current)\n\
            Fetching more\n\
            \u{1b}]8;;https://cursor.com\u{7}gpt-5\u{1b}]8;;\u{1b}\\ - GPT-5\n\n\
            Tip: use --model <id>\n\
            after - Not a model\n";
        assert_eq!(
            parse_cursor_models(output),
            Some(vec![cursor("auto", "auto"), cursor("gpt-5", "GPT-5")])
        );
        assert_eq!(parse_cursor_models("Loading models…\nauto - Auto\n"), None);
        assert_eq!(parse_cursor_models(""), None);
    }

    #[test]
    fn skips_invalid_ids_repeats_and_the_empty_listing() {
        assert_eq!(
            parse_cursor_models("No models available for this account."),
            Some(Vec::new())
        );
        let output = "Available models\n-flag - Not a model\ngpt-5 - GPT-5\ngpt-5 - GPT-5 again\n";
        assert_eq!(
            parse_cursor_models(output),
            Some(vec![cursor("gpt-5", "GPT-5")])
        );
    }

    #[test]
    fn discovered_models_follow_their_runtimes_catalog_entry() {
        let models = picker_models(&[
            cursor("gpt-5", "GPT-5"),
            cursor("default", "Duplicate of the catalog"),
            cursor("gpt-5", "Repeat"),
            cursor("auto", "Auto"),
            ModelChoice {
                id: "opencode/x".into(),
                label: "Filed under the wrong runtime".into(),
                provider: AgentProvider::Cursor,
            },
        ]);
        assert_eq!(models.len(), MODEL_CATALOG.len() + 2);
        let default = models
            .iter()
            .position(|model| model.id == "cursor/default")
            .unwrap();
        assert_eq!(models[default + 1], cursor("gpt-5", "GPT-5"));
        assert_eq!(models[default + 2], cursor("auto", "Auto"));
        assert_eq!(models[default + 3].provider, AgentProvider::Antigravity);
        assert_eq!(picker_models(&[]).len(), MODEL_CATALOG.len());
    }

    #[test]
    fn listings_replace_per_runtime_and_explain_failures() {
        let mut listings = Vec::new();
        let listed = RuntimeModels {
            models: vec![cursor("gpt-5", "GPT-5")],
            error: None,
        };
        assert!(replace_listing(
            &mut listings,
            AgentProvider::Cursor,
            listed.clone()
        ));
        assert!(!replace_listing(
            &mut listings,
            AgentProvider::Cursor,
            listed
        ));
        assert_eq!(listed_models(&listings).len(), MODEL_CATALOG.len() + 1);
        assert_eq!(listing_error_line(&listings, AgentProvider::Cursor), None);
        let failed = RuntimeModels {
            models: Vec::new(),
            error: Some("Run `cursor-agent update`.".into()),
        };
        assert!(replace_listing(
            &mut listings,
            AgentProvider::Cursor,
            failed
        ));
        assert_eq!(listings.len(), 1);
        assert_eq!(
            listing_error_line(&listings, AgentProvider::Cursor).as_deref(),
            Some("Cursor models unavailable: Run `cursor-agent update`.")
        );
        assert_eq!(listing_error_line(&listings, AgentProvider::Claude), None);
        let stale = RuntimeModels {
            models: vec![cursor("gpt-5", "GPT-5")],
            error: Some("timed out".into()),
        };
        replace_listing(&mut listings, AgentProvider::Cursor, stale);
        assert_eq!(
            listing_error_line(&listings, AgentProvider::Cursor).as_deref(),
            Some("Cursor models may be out of date: timed out")
        );
    }

    #[test]
    fn arriving_rows_keep_the_highlight_on_its_model() {
        let before = picker_models(&[]);
        let after = picker_models(&[cursor("gpt-5", "GPT-5"), cursor("auto", "Auto")]);
        // A catalog row after the Cursor group keeps its model, not its index.
        let grok = before.iter().position(|m| m.id == "grok/default").unwrap();
        let previous = highlighted_id(&before, None, grok);
        let moved = reanchor_highlight(previous, &after, None, grok);
        assert_eq!(after[moved].id, "grok/default");
        assert_eq!(moved, grok + 2);
        // The custom row's model becoming a listed row keeps the highlight.
        let previous = highlighted_id(&before, Some("cursor/gpt-5"), before.len());
        assert_eq!(previous, Some("cursor/gpt-5"));
        let moved = reanchor_highlight(previous, &after, None, before.len());
        assert_eq!(after[moved].id, "cursor/gpt-5");
        // A model that disappears clamps the highlight into the rows.
        let short = vec![cursor("gpt-5", "GPT-5")];
        assert_eq!(reanchor_highlight(Some("gone"), &short, None, 9), 0);
        assert_eq!(reanchor_highlight(None, &[], None, 3), 0);
        assert_eq!(highlighted_id(&short, None, 4), None);
    }

    #[test]
    fn list_errors_are_short_and_name_the_fix() {
        assert_eq!(
            cursor_list_error("error: unknown option '--list-models'\n"),
            "This Cursor CLI cannot list models. Run `cursor-agent update`."
        );
        assert_eq!(
            cursor_list_error(
                "Error: Authentication required. Run 'agent login', pass --api-key/--auth-token, or set CURSOR_API_KEY/CURSOR_AUTH_TOKEN.\n"
            ),
            "Authentication required. Run 'agent login', pass --api-key/--auth-token, or set CURSOR_API_KEY/CURSOR_AUTH_TOKEN."
        );
        assert_eq!(cursor_list_error(""), "Cursor could not list models.");
        assert!(cursor_list_error(&"x".repeat(500)).ends_with('…'));
    }

    #[test]
    fn wire_form_omits_a_missing_error() {
        let json = serde_json::to_string(&RuntimeModels {
            models: vec![cursor("gpt-5", "GPT-5")],
            error: None,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"models":[{"id":"cursor/gpt-5","label":"GPT-5","provider":"cursor"}]}"#
        );
        assert_eq!(
            serde_json::from_str::<RuntimeModels>("{}").unwrap(),
            RuntimeModels::default()
        );
    }
}
