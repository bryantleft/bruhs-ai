//! When a Claude thread should compact. Codex has no switch, so callers
//! keep the unavailable notice for that provider.
//!
//! Auto-compact is Claude's `--autocompact`. An empty setting omits the
//! flag. Idle compaction is bruhs sending `/compact` while the app is open.

/// Tokens in the last turn before a return chip or an idle compact.
pub const IDLE_TOKEN_FLOOR: u64 = 200_000;

/// Prompt-cache window on a Claude subscription login.
pub const CACHE_OAUTH_MS: f64 = 3_600_000.0;

/// Prompt-cache window when `BRUHS_CLAUDE_USE_API_KEY=1` keeps API billing.
pub const CACHE_API_MS: f64 = 300_000.0;

/// How long before the cache window an idle compact may start.
pub const BEFORE_LEAD_MS: f64 = 120_000.0;

/// Claude's `--autocompact` range, in tokens.
pub const AUTOCOMPACT_MIN: u64 = 100_000;
pub const AUTOCOMPACT_MAX: u64 = 1_000_000;

/// What to do with an idle Claude thread that is over [`IDLE_TOKEN_FLOOR`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IdleCompact {
    /// After the cache window, offer Compact beside send.
    #[default]
    Return,
    /// Compact shortly before the cache window ends.
    Before,
    /// No chip and no idle `/compact`. Manual compact still works.
    Never,
}

impl IdleCompact {
    pub fn parse(stored: &str) -> Self {
        match stored {
            "before" => Self::Before,
            "never" => Self::Never,
            _ => Self::Return,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::Before => "before",
            Self::Never => "never",
        }
    }
}

/// `None` when the field is empty or outside Claude's range.
pub fn autocompact_tokens(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens: u64 = trimmed.parse().ok()?;
    (AUTOCOMPACT_MIN..=AUTOCOMPACT_MAX)
        .contains(&tokens)
        .then_some(tokens)
}

/// The `--autocompact` value, or `None` when the flag should be omitted.
pub fn autocompact_flag(raw: Option<&str>) -> Option<String> {
    raw.and_then(autocompact_tokens)
        .map(|tokens| tokens.to_string())
}

/// Input tokens plus cache reads and writes. Output tokens are the reply.
pub fn context_tokens(input: u64, cache_read: u64, cache_creation: u64) -> u64 {
    input
        .saturating_add(cache_read)
        .saturating_add(cache_creation)
}

/// How full the context window is after a turn: the provider's reported
/// window (`context`) when known, else the older estimate from the counts.
/// The estimate over-counts a turn with several model requests, because a
/// turn's counts are summed over its requests.
pub fn window_tokens(
    context: Option<u64>,
    input: u64,
    cache_read: u64,
    cache_creation: u64,
) -> u64 {
    context.unwrap_or_else(|| context_tokens(input, cache_read, cache_creation))
}

pub fn cache_window_ms(api_key_opt_in: bool) -> f64 {
    if api_key_opt_in {
        CACHE_API_MS
    } else {
        CACHE_OAUTH_MS
    }
}

/// Compact chip beside send. The thread has been idle at least one cache window.
pub fn show_return_chip(mode: IdleCompact, tokens: u64, idle_for_ms: f64, window_ms: f64) -> bool {
    mode == IdleCompact::Return && tokens >= IDLE_TOKEN_FLOOR && idle_for_ms >= window_ms
}

/// An idle `/compact` is due in the lead time before the cache window ends.
pub fn before_expiry_due(mode: IdleCompact, tokens: u64, idle_for_ms: f64, window_ms: f64) -> bool {
    if mode != IdleCompact::Before || tokens < IDLE_TOKEN_FLOOR {
        return false;
    }
    let start = window_ms - BEFORE_LEAD_MS;
    idle_for_ms >= start && idle_for_ms < window_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reported_window_beats_the_summed_estimate() {
        // A two-request turn: 96.4k summed, 49.2k actually in the window.
        assert_eq!(window_tokens(Some(49_191), 4, 74_003, 22_447), 49_191);
        assert_eq!(window_tokens(None, 4, 74_003, 22_447), 96_454);
    }

    #[test]
    fn autocompact_keeps_only_the_cli_range() {
        assert_eq!(autocompact_tokens(""), None);
        assert_eq!(autocompact_tokens("99999"), None);
        assert_eq!(autocompact_tokens("100000"), Some(100_000));
        assert_eq!(autocompact_tokens(" 300000 "), Some(300_000));
        assert_eq!(autocompact_tokens("1000001"), None);
        assert_eq!(autocompact_flag(Some("nope")), None);
        assert_eq!(autocompact_flag(None), None);
        assert_eq!(autocompact_flag(Some("300000")).as_deref(), Some("300000"));
    }

    #[test]
    fn return_chip_and_before_expiry_use_the_token_floor_and_the_window() {
        let over = IDLE_TOKEN_FLOOR;
        let under = IDLE_TOKEN_FLOOR - 1;
        assert!(!show_return_chip(
            IdleCompact::Return,
            over,
            CACHE_OAUTH_MS - 1.0,
            CACHE_OAUTH_MS
        ));
        assert!(show_return_chip(
            IdleCompact::Return,
            over,
            CACHE_OAUTH_MS,
            CACHE_OAUTH_MS
        ));
        assert!(!show_return_chip(
            IdleCompact::Return,
            under,
            CACHE_OAUTH_MS,
            CACHE_OAUTH_MS
        ));
        assert!(!show_return_chip(
            IdleCompact::Never,
            over,
            CACHE_OAUTH_MS,
            CACHE_OAUTH_MS
        ));
        let start = CACHE_OAUTH_MS - BEFORE_LEAD_MS;
        assert!(!before_expiry_due(
            IdleCompact::Before,
            over,
            start - 1.0,
            CACHE_OAUTH_MS
        ));
        assert!(before_expiry_due(
            IdleCompact::Before,
            over,
            start,
            CACHE_OAUTH_MS
        ));
        assert!(!before_expiry_due(
            IdleCompact::Before,
            over,
            CACHE_OAUTH_MS,
            CACHE_OAUTH_MS
        ));
        assert!(!before_expiry_due(
            IdleCompact::Return,
            over,
            start,
            CACHE_OAUTH_MS
        ));
        assert_eq!(cache_window_ms(false), CACHE_OAUTH_MS);
        assert_eq!(cache_window_ms(true), CACHE_API_MS);
    }
}
