//! Consent-scoped browser and device operations shared by both clients.
use serde::{Deserialize, Serialize};

mod computer;
mod live_input;
pub use computer::*;
pub use live_input::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolScope {
    Browser,
    Devices,
    Dictation,
}

/// Marks a grant principal that is a bruh rather than a thread.
pub const BRUH_PRINCIPAL_PREFIX: &str = "bruh:";

/// Who a local tool grant or browser belongs to. On the wire it is a plain
/// string: a thread id, or `bruh:<agent id>` for a bruh's Computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    Thread(String),
    Bruh(String),
}

impl Principal {
    /// `bruh:<id>` with a nonempty id is a bruh; anything else is a thread.
    #[must_use]
    pub fn parse(value: &str) -> Principal {
        match value.strip_prefix(BRUH_PRINCIPAL_PREFIX) {
            Some(id) if !id.is_empty() => Principal::Bruh(id.into()),
            _ => Principal::Thread(value.into()),
        }
    }

    #[must_use]
    pub fn wire(&self) -> String {
        match self {
            Principal::Thread(id) => id.clone(),
            Principal::Bruh(id) => format!("{BRUH_PRINCIPAL_PREFIX}{id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGrant {
    pub id: String,
    pub principal: String,
    pub scope: ToolScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantRequest {
    pub principal: String,
    pub scope: ToolScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSession {
    pub id: String,
    pub grant_id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate {
        url: String,
    },
    Snapshot,
    Click {
        selector: String,
    },
    Fill {
        selector: String,
        text: String,
    },
    Evaluate {
        expression: String,
    },
    Screenshot,
    Console,
    Network,
    /// A real press and release at a point in the page view, in CSS px.
    ClickAt {
        x: f64,
        y: f64,
    },
    /// Real key input into `selector` when given, else into the focused
    /// element.
    Type {
        text: String,
        #[serde(default)]
        selector: Option<String>,
    },
    /// One of [`AGENT_KEYS`].
    Key {
        key: String,
    },
    /// A wheel scroll of the page, in CSS px.
    Scroll {
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
}

/// What a browser action does to the page, which decides how it is gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserActionKind {
    Read,
    Navigate,
    Input,
    Script,
}

impl BrowserAction {
    #[must_use]
    pub fn kind(&self) -> BrowserActionKind {
        match self {
            BrowserAction::Snapshot
            | BrowserAction::Screenshot
            | BrowserAction::Console
            | BrowserAction::Network
            | BrowserAction::Scroll { .. } => BrowserActionKind::Read,
            BrowserAction::Navigate { .. } => BrowserActionKind::Navigate,
            BrowserAction::Click { .. }
            | BrowserAction::ClickAt { .. }
            | BrowserAction::Type { .. }
            | BrowserAction::Key { .. }
            | BrowserAction::Fill { .. } => BrowserActionKind::Input,
            BrowserAction::Evaluate { .. } => BrowserActionKind::Script,
        }
    }

    /// Whether a bruh may use this action. Scripts, the console and the
    /// network log are never offered to bruhs and the host refuses them.
    #[must_use]
    pub fn for_bruhs(&self) -> bool {
        !matches!(
            self,
            BrowserAction::Evaluate { .. } | BrowserAction::Console | BrowserAction::Network
        )
    }
}

/// The browser actions a bruh's Computer offers, by wire name.
pub const BRUH_BROWSER_ACTIONS: [&str; 9] = [
    "navigate",
    "snapshot",
    "screenshot",
    "click",
    "click_at",
    "type",
    "key",
    "scroll",
    "fill",
];

/// The named keys an agent may press with [`BrowserAction::Key`].
pub const AGENT_KEYS: [&str; 14] = [
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "Space",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
];

/// The most characters one type action or one owner text insert carries.
pub const MAX_TYPE_CHARS: usize = 2_000;
/// The largest scroll or wheel delta on either axis, in CSS px.
pub const MAX_SCROLL_PX: f64 = 10_000.0;

const CLICK_OUTSIDE: &str = "Click inside the page view.";
/// Names [`COMPUTER_VIEWPORT`]'s size; a test keeps the two in step.
const CLICK_OUTSIDE_COMPUTER: &str = "Click inside the 1280 by 800 view.";
const TOO_MUCH_TEXT: &str = "Type at most 2,000 characters at once.";
const TOO_MUCH_FILL: &str = "Fill at most 2,000 characters at once.";
const ENCODED_HOST: &str = "Use the host's plain ASCII name (punycode for international names).";
const SCROLL_TOO_FAR: &str = "Scroll at most 10,000 pixels at once.";
const UNKNOWN_KEY: &str =
    "Press Enter, Tab, Escape, Backspace, Delete, Space, an arrow key, Home, End, PageUp or PageDown.";

/// Check a browser action against the view it acts on and its budgets.
/// Navigate also refuses hosts on this computer, which is a bruh rule, so
/// thread browsers don't run their navigation through this.
pub fn validate_action(action: &BrowserAction, viewport: Viewport) -> Result<(), &'static str> {
    match action {
        BrowserAction::ClickAt { x, y } if !in_view(*x, *y, viewport) => {
            Err(if viewport == COMPUTER_VIEWPORT {
                CLICK_OUTSIDE_COMPUTER
            } else {
                CLICK_OUTSIDE
            })
        }
        BrowserAction::Type { text, .. } if !within_type_budget(text) => Err(TOO_MUCH_TEXT),
        BrowserAction::Fill { text, .. } if !within_type_budget(text) => Err(TOO_MUCH_FILL),
        BrowserAction::Key { key } if !AGENT_KEYS.contains(&key.as_str()) => Err(UNKNOWN_KEY),
        BrowserAction::Scroll { dx, dy } if !within_scroll(*dx, *dy) => Err(SCROLL_TOO_FAR),
        BrowserAction::Navigate { url } => validate_navigation(url),
        _ => Ok(()),
    }
}

/// The body of a bruh's browser command route.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct BruhBrowserCommand {
    pub command: BrowserAction,
}

/// A finite point within `0..=width` and `0..=height`.
fn in_view(x: f64, y: f64, viewport: Viewport) -> bool {
    x.is_finite()
        && y.is_finite()
        && (0.0..=f64::from(viewport.width)).contains(&x)
        && (0.0..=f64::from(viewport.height)).contains(&y)
}

fn within_scroll(dx: f64, dy: f64) -> bool {
    [dx, dy]
        .iter()
        .all(|delta| delta.is_finite() && delta.abs() <= MAX_SCROLL_PX)
}

fn within_type_budget(text: &str) -> bool {
    text.chars().nth(MAX_TYPE_CHARS).is_none()
}

/// A URL a bruh's browser may open: the browser URL rules, and never a host
/// on this computer. Chrome percent-decodes and maps (UTS46) a host before it
/// resolves it, so a host with `%` or any non-ASCII character is refused
/// rather than checked as written.
fn validate_navigation(url: &str) -> Result<(), &'static str> {
    validate_browser_url(url)?;
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    let authority = rest.and_then(|rest| rest.split(['/', '?', '#']).next());
    if authority.is_some_and(|authority| authority.contains('%') || !authority.is_ascii()) {
        return Err(ENCODED_HOST);
    }
    if authority.is_some_and(crate::crew::is_local_host) {
        return Err(crate::crew::LOCAL_HOST_DENIAL);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCommand {
    pub grant_id: String,
    pub session_id: String,
    pub command: BrowserAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStart {
    pub grant_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolImage {
    pub mime_type: String,
    pub base64: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolReply {
    pub value: serde_json::Value,
    pub image: Option<ToolImage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Android,
    Ios,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub state: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceInventory {
    pub devices: Vec<Device>,
    /// Discovery failures stay visible even when another platform works.
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DeviceAction {
    Boot,
    Shutdown,
    Screenshot,
    Tap { x: u32, y: u32 },
    Type { text: String },
    Home,
    Back,
    OpenUrl { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCommand {
    pub grant_id: String,
    pub device_id: String,
    pub platform: DevicePlatform,
    pub command: DeviceAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceVideoStart {
    pub grant_id: String,
    pub device_id: String,
    pub platform: DevicePlatform,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalToolsSnapshot {
    pub grants: Vec<ToolGrant>,
    pub browsers: Vec<BrowserSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationTranscript {
    /// Draft text only. A transcription never starts an agent turn.
    pub text: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationOptions {
    #[serde(default)]
    pub model: crate::WhisperModel,
    pub language: Option<String>,
}

/// Reject active content and host-file navigation before opening a browser.
pub fn validate_browser_url(url: &str) -> Result<(), &'static str> {
    if url == "about:blank" {
        return Ok(());
    }
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or("Use an http or https URL")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty()
        || authority.contains('@')
        || url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err("The browser URL must have a host and no credentials or whitespace");
    }
    Ok(())
}

/// A transcript replaces the original selection only while that draft is unchanged.
/// A concurrently edited draft keeps its edits and receives the transcript at its end.
pub fn insert_dictation(
    current: &str,
    original: &str,
    selection: std::ops::Range<usize>,
    transcript: &str,
) -> Result<String, &'static str> {
    if transcript.trim().is_empty() {
        return Ok(current.into());
    }
    if current == original {
        if selection.start > selection.end
            || !current.is_char_boundary(selection.start)
            || !current.is_char_boundary(selection.end)
        {
            return Err(
                "The dictation selection is no longer valid. Copy the transcript into the draft.",
            );
        }
        let mut result = current.to_string();
        result.replace_range(selection, transcript);
        Ok(result)
    } else {
        let separator = if current.is_empty() || current.ends_with(char::is_whitespace) {
            ""
        } else {
            " "
        };
        Ok(format!("{current}{separator}{transcript}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_cannot_open_local_files_or_credentials() {
        for invalid in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://u:p@example.com",
            "https://",
            "https://a\\@b",
            "http://a\nb",
        ] {
            assert!(validate_browser_url(invalid).is_err(), "{invalid}");
        }
        for valid in [
            "http://localhost:3000",
            "https://example.com/path?q=x",
            "about:blank",
        ] {
            assert!(validate_browser_url(valid).is_ok(), "{valid}");
        }
    }

    #[test]
    fn dictation_preserves_selection_and_concurrent_edits() {
        assert_eq!(
            insert_dictation("fix bug", "fix bug", 4..7, "tests").unwrap(),
            "fix tests"
        );
        assert_eq!(
            insert_dictation("new draft", "fix bug", 4..7, "tests").unwrap(),
            "new draft tests"
        );
        assert_eq!(
            insert_dictation("keep", "keep", 0..4, "  ").unwrap(),
            "keep"
        );
        assert!(insert_dictation("é", "é", 1..2, "test").is_err());
    }

    #[test]
    fn principals_keep_thread_ids_and_mark_bruhs() {
        assert_eq!(
            Principal::parse("thread-1"),
            Principal::Thread("thread-1".into())
        );
        assert_eq!(Principal::parse("bruh:a1"), Principal::Bruh("a1".into()));
        assert_eq!(Principal::Bruh("a1".into()).wire(), "bruh:a1");
        assert_eq!(Principal::parse("bruh:"), Principal::Thread("bruh:".into()));
    }

    #[test]
    fn real_input_actions_have_stable_wire_names_and_kinds() {
        let at: BrowserAction =
            serde_json::from_str(r#"{"action":"click_at","x":10,"y":20.5}"#).unwrap();
        assert_eq!(at, BrowserAction::ClickAt { x: 10.0, y: 20.5 });
        let typed: BrowserAction =
            serde_json::from_str(r#"{"action":"type","text":"hi"}"#).unwrap();
        assert_eq!(
            typed,
            BrowserAction::Type {
                text: "hi".into(),
                selector: None
            }
        );
        assert_eq!(
            BrowserAction::Scroll { dx: 0.0, dy: 300.0 }.kind(),
            BrowserActionKind::Read
        );
        assert_eq!(
            BrowserAction::Navigate {
                url: "https://a.test".into()
            }
            .kind(),
            BrowserActionKind::Navigate
        );
        assert_eq!(
            BrowserAction::Key {
                key: "Enter".into()
            }
            .kind(),
            BrowserActionKind::Input
        );
        assert_eq!(
            BrowserAction::Evaluate {
                expression: "1".into()
            }
            .kind(),
            BrowserActionKind::Script
        );
        for hidden in [
            BrowserAction::Console,
            BrowserAction::Network,
            BrowserAction::Evaluate {
                expression: "1".into(),
            },
        ] {
            assert!(!hidden.for_bruhs());
        }
        // A login-fill call from a model that guessed project 4's shape doesn't parse.
        assert!(serde_json::from_str::<BrowserAction>(
            r##"{"action":"fill","selector":"#p","secret":{"login_id":"x"}}"##
        )
        .is_err());
    }

    #[test]
    fn bruh_navigation_refuses_encoded_and_mapped_hosts() {
        let v = COMPUTER_VIEWPORT;
        for url in [
            "http://%6Cocalhost:3001/",
            "http://%31%32%37.0.0.1/",
            "http://\u{ff4c}\u{ff4f}\u{ff43}\u{ff41}\u{ff4c}\u{ff48}\u{ff4f}\u{ff53}\u{ff54}/",
            "http://127\u{3002}0\u{3002}0\u{3002}1/",
        ] {
            let navigate = BrowserAction::Navigate { url: url.into() };
            assert_eq!(validate_action(&navigate, v), Err(ENCODED_HOST), "{url}");
        }
        // Encoding in the path is fine, and thread browsers keep their rules.
        let path = BrowserAction::Navigate {
            url: "https://example.com/a%20b?q=%41".into(),
        };
        assert!(validate_action(&path, v).is_ok());
        assert!(validate_browser_url("http://%6Cocalhost:3001/").is_ok());
        let local = BrowserAction::Navigate {
            url: "http://[::1]:3001/".into(),
        };
        assert_eq!(
            validate_action(&local, v),
            Err(crate::crew::LOCAL_HOST_DENIAL)
        );
    }

    #[test]
    fn click_copy_names_the_computer_view() {
        assert_eq!(
            CLICK_OUTSIDE_COMPUTER,
            format!(
                "Click inside the {} by {} view.",
                COMPUTER_VIEWPORT.width, COMPUTER_VIEWPORT.height
            )
        );
    }

    #[test]
    fn fill_text_has_the_type_budget() {
        let v = COMPUTER_VIEWPORT;
        let fill = |text: String| BrowserAction::Fill {
            selector: "#q".into(),
            text,
        };
        assert!(validate_action(&fill("x".repeat(MAX_TYPE_CHARS)), v).is_ok());
        assert_eq!(
            validate_action(&fill("x".repeat(MAX_TYPE_CHARS + 1)), v),
            Err("Fill at most 2,000 characters at once.")
        );
    }

    #[test]
    fn actions_stay_inside_the_view_and_their_budgets() {
        let v = COMPUTER_VIEWPORT;
        assert!(validate_action(
            &BrowserAction::ClickAt {
                x: 1280.0,
                y: 800.0
            },
            v
        )
        .is_ok());
        assert_eq!(
            validate_action(&BrowserAction::ClickAt { x: -1.0, y: 0.0 }, v),
            Err("Click inside the 1280 by 800 view.")
        );
        assert!(validate_action(
            &BrowserAction::ClickAt {
                x: f64::NAN,
                y: 0.0
            },
            v
        )
        .is_err());
        let long = "x".repeat(MAX_TYPE_CHARS + 1);
        assert_eq!(
            validate_action(
                &BrowserAction::Type {
                    text: long,
                    selector: None
                },
                v
            ),
            Err("Type at most 2,000 characters at once.")
        );
        assert_eq!(validate_action(&BrowserAction::Key { key: "F12".into() }, v),
            Err("Press Enter, Tab, Escape, Backspace, Delete, Space, an arrow key, Home, End, PageUp or PageDown."));
        assert!(validate_action(
            &BrowserAction::Scroll {
                dx: 0.0,
                dy: MAX_SCROLL_PX + 1.0
            },
            v
        )
        .is_err());
    }
}
