//! Who gets a Computer, how each browser call is gated, and what the
//! Computer tab offers and says. Shared by both apps and the service.

use serde_json::Value;

use super::{
    computer_ready, effective_level, gate, is_local_host, service_notice, AccessLevel, BruhAccess,
    Gate, GatedAction, FORGE_COMMAND_HOSTS, LOCAL_HOST_DENIAL,
};
use crate::agent::AgentProvider;
use crate::local_tools::{
    validate_action, validate_browser_url, BrowserAction, BrowserActionKind, ComputerState,
    Control, COMPUTER_VIEWPORT,
};

/// The tool a bruh's browser calls arrive as.
pub const BRUH_BROWSER_TOOL: &str = "mcp__bruhs_workbench__local_browser";

/// How long an unwatched, unheld browser stays open after its last use.
pub const COMPUTER_IDLE_MS: u64 = 10 * 60 * 1000;
/// The most bruh browsers open at once.
pub const MAX_COMPUTERS: usize = 4;
/// The most live views one browser streams to at once.
pub const MAX_VIEWERS: usize = 4;
/// JPEG quality of live view frames.
pub const SCREENCAST_QUALITY: u8 = 70;
/// The shortest gap between two frames Chrome is acked for, in ms.
pub const MIN_FRAME_MS: u64 = 100;
/// How long a closing browser gets to exit before it is killed, in ms.
pub const CLOSE_GRACE_MS: u64 = 5_000;

// Model-facing copy.

pub const COMPUTER_OFF_DENIAL: &str =
    "Your computer is off. The owner can turn it on in your Settings.";
pub const NOT_FOR_BRUHS: &str =
    "This browser action isn't available to bruhs. Take a snapshot to read the page.";
pub const UNKNOWN_BROWSER_ACTION: &str = "Use one of the local_browser actions: navigate, snapshot, screenshot, click, click_at, type, key, scroll, fill.";
pub const OWNER_HAS_COMPUTER: &str =
    "The owner has taken control of your computer. Wait for them to hand it back.";
pub const CONTROL_WAIT_EXPIRED: &str =
    "The owner still has your computer. Stop here and say you are waiting for it.";
pub const CLOSED_BY_OWNER: &str =
    "The owner closed your browser. It opens again on your next step.";
pub const ACTION_STOPPED: &str = "Stopped by the owner.";
pub const PAGE_CHANGED: &str =
    "The page changed while the owner decided. Take a snapshot and try again.";
/// The command route got a step the gate never let through, or one that
/// already ran.
pub const STEP_NOT_APPROVED: &str = "This browser step wasn't approved.";
/// The command route got a step from a web page, not the bruh's helper.
pub const STEP_FROM_A_PAGE: &str = "Only the bruh's own browser tool can send its steps.";

/// What the bruh's next browser call learns after the owner hands back.
#[must_use]
pub fn handed_back_note(url: &str) -> String {
    format!(
        "The owner used your computer and handed it back. It is now on {url}. Take a snapshot before you act."
    )
}

// App copy.

pub const COMPUTER_TAB: &str = "Computer";
pub const COMPUTER_SWITCH: &str = "Computer";
pub const COMPUTER_HINT: &str = "Its own browser, which keeps its logins. Watch it and take control in the Computer tab. At Full it acts without asking on sites you sign in to there.";
pub const OPEN_BROWSER: &str = "Open browser";
pub const CLOSE_BROWSER: &str = "Close browser";
pub const TAKE_CONTROL: &str = "Take control";
pub const HAND_BACK: &str = "Hand back";
pub const OPEN_IN_WINDOW: &str = "Open in a window";
pub const BACK: &str = "Back";
pub const RELOAD: &str = "Reload";
pub const ADDRESS_LABEL: &str = "Address";
pub const LIVE_VIEW_LABEL: &str = "Live view";
pub const LIVE_VIEW_HINT: &str = "Keys go to the page. Press F6 to leave.";
pub const TAKE_CONTROL_FIRST: &str = "Take control to use the page.";
pub const USE_THE_WINDOW: &str = "Use the window.";
pub const COMPUTERS_FULL: &str = "Four bruh browsers are open. Close one in its Computer tab.";
pub const VIEWERS_FULL: &str = "Four live views are open for this browser. Close one first.";
pub const LIVE_VIEW_ENDED: &str = "This live view has ended. Open the browser again.";
/// The live view didn't acknowledge a frame within the tool deadline.
pub const LIVE_VIEW_STALLED: &str = "The live view stopped showing frames. Open it again.";
/// A page from another site asked for a live view.
pub const LIVE_VIEW_FOREIGN_PAGE: &str = "This page can't open the live view.";
/// The live view sent something other than an acknowledgement or input.
pub const LIVE_VIEW_UNREADABLE: &str = "The live view sent a message the server can't read.";
pub const TURN_ON_IN_SETTINGS: &str = "Turn on Computer in the bruh's Settings.";

pub const NEEDS_CLAUDE: &str = "Computer needs a Claude model.";
const NEEDS_ASK_OR_FULL: &str = "Computer needs Ask first or Full access.";

/// Why a bruh at `level` on `provider` can't have a Computer, if it can't.
/// The provider comes first, since a level change can't fix it.
#[must_use]
pub fn computer_unavailable(level: AccessLevel, provider: AgentProvider) -> Option<&'static str> {
    if provider != AgentProvider::Claude {
        Some(NEEDS_CLAUDE)
    } else if level == AccessLevel::ReadOnly {
        Some(NEEDS_ASK_OR_FULL)
    } else {
        None
    }
}

/// Why the Computer switch in a bruh's Settings is off, if it is: the
/// model or level first, since the owner can change those there, then an
/// older service. Both apps show this one reason.
#[must_use]
pub fn computer_switch_reason(
    level: AccessLevel,
    provider: AgentProvider,
    protocol: Option<u32>,
) -> Option<&'static str> {
    computer_unavailable(level, provider).or_else(|| computer_reason(protocol))
}

/// Whether a bruh's Computer is on: switched on, and available to it.
#[must_use]
pub fn computer_on(on: bool, access: &BruhAccess, provider: AgentProvider) -> bool {
    on && computer_unavailable(effective_level(access.level, provider), provider).is_none()
}

/// Why the Computer is off on this service: an older service's update
/// notice once its version is known, and `None` while the Computer works or
/// the version is still unknown.
#[must_use]
pub fn computer_reason(protocol: Option<u32>) -> Option<&'static str> {
    let protocol = protocol.filter(|protocol| !computer_ready(*protocol))?;
    service_notice(protocol, None).map(|notice| notice.message())
}

/// Whether `host` is GitHub, GitLab or one of their subdomains, where a
/// page acts in the owner's name. A port (`github.com:443`), a trailing dot
/// and case are ignored.
#[must_use]
pub fn is_forge_host(host: &str) -> bool {
    let host = match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') && port.bytes().all(|b| b.is_ascii_digit()) => {
            name
        }
        _ => host,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    FORGE_COMMAND_HOSTS.iter().any(|forge| {
        host == *forge
            || host
                .strip_suffix(forge)
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

/// The host of a page URL, lowercased, without its port or login.
fn page_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Whether two page URLs are on the same host, as the gate tells pages
/// apart. Two pages whose host is unknown count as the same.
#[must_use]
pub fn same_page_host(a: Option<&str>, b: Option<&str>) -> bool {
    a.and_then(page_host) == b.and_then(page_host)
}

/// The gate for one browser call of a bruh run. `input` is the tool's input
/// (`{"command": {…}}`); `page_url` is the page the call acts on, if known.
/// Input on a page whose host is unknown follows the forge rule, since the
/// page may be GitHub or GitLab; `about:blank` is a known, plain page.
#[must_use]
pub fn browser_gate(
    access: &BruhAccess,
    provider: AgentProvider,
    computer: bool,
    input: &Value,
    page_url: Option<&str>,
) -> Gate {
    let level = effective_level(access.level, provider);
    if let Some(reason) = computer_unavailable(level, provider) {
        return Gate::Deny(reason);
    }
    if !computer {
        return Gate::Deny(COMPUTER_OFF_DENIAL);
    }
    let Ok(action) = serde_json::from_value::<BrowserAction>(input["command"].clone()) else {
        return Gate::Deny(UNKNOWN_BROWSER_ACTION);
    };
    if !action.for_bruhs() {
        return Gate::Deny(NOT_FOR_BRUHS);
    }
    if let Err(reason) = validate_action(&action, COMPUTER_VIEWPORT) {
        return Gate::Deny(reason);
    }
    // A backstop: `validate_action` already refuses these, and more strictly.
    if let BrowserAction::Navigate { url } = &action {
        if let Err(reason) = validate_browser_url(url) {
            return Gate::Deny(reason);
        }
        if page_host(url).is_some_and(|host| is_local_host(&host)) {
            return Gate::Deny(LOCAL_HOST_DENIAL);
        }
    }
    match action.kind() {
        BrowserActionKind::Read => Gate::Allow,
        BrowserActionKind::Navigate => gate(access, level, GatedAction::Write),
        BrowserActionKind::Input
            if page_url
                .and_then(page_host)
                .is_none_or(|host| is_forge_host(&host)) =>
        {
            if level == AccessLevel::Full && access.send_as_me && access.can_deliver {
                Gate::Allow
            } else {
                Gate::Approve
            }
        }
        BrowserActionKind::Input => gate(access, level, GatedAction::Write),
        BrowserActionKind::Script => Gate::Deny(NOT_FOR_BRUHS),
    }
}

/// An approval card's summary of a browser call, before it is cut to size.
pub(super) fn browser_summary(input: &Value) -> Option<String> {
    let action = serde_json::from_value::<BrowserAction>(input["command"].clone()).ok()?;
    let what = match &action {
        BrowserAction::Navigate { url } => return Some(format!("Open {url}")),
        BrowserAction::Click { selector } => format!("Click {selector}"),
        BrowserAction::ClickAt { x, y } => format!("Click at {x}, {y}"),
        BrowserAction::Type { .. } => "Type".to_string(),
        BrowserAction::Key { key } => format!("Press {key}"),
        BrowserAction::Fill { selector, .. } => format!("Fill {selector}"),
        BrowserAction::Scroll { .. } => "Scroll".to_string(),
        BrowserAction::Snapshot => "Read the page".to_string(),
        BrowserAction::Screenshot => "Take a screenshot".to_string(),
        BrowserAction::Evaluate { .. } | BrowserAction::Console | BrowserAction::Network => {
            return None
        }
    };
    Some(match input["page_url"].as_str().and_then(page_host) {
        Some(host) => format!("{what} on {host}"),
        None => what,
    })
}

/// What a browser call's card shows in full: the text it types or fills,
/// or a URL to open whose summary is longer than `summary_max` characters.
pub(super) fn browser_detail(input: &Value, summary_max: usize) -> Option<String> {
    match serde_json::from_value::<BrowserAction>(input["command"].clone()).ok()? {
        BrowserAction::Type { text, .. } | BrowserAction::Fill { text, .. } => Some(text),
        BrowserAction::Navigate { url } => browser_summary(input)
            .is_some_and(|summary| summary.chars().count() > summary_max)
            .then_some(url),
        _ => None,
    }
}

/// What the Computer tab offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComputerControls {
    pub open: bool,
    pub close: bool,
    pub take: bool,
    pub hand_back: bool,
    /// Open in a window; desktop only.
    pub window: bool,
    /// The address bar, Back and Reload.
    pub navigate: bool,
    /// Owner input into the live view.
    pub input: bool,
}

/// What the Computer tab offers for `state`. Nothing while the Computer is
/// off or the service can't run it (`ready`); `desktop` adds Open in a
/// window, which web can't do.
#[must_use]
pub fn computer_controls(
    state: Option<&ComputerState>,
    on: bool,
    ready: bool,
    desktop: bool,
) -> ComputerControls {
    if !(on && ready) {
        return ComputerControls::default();
    }
    let Some(state) = state.filter(|state| state.session_id.is_some()) else {
        return ComputerControls {
            open: true,
            ..ComputerControls::default()
        };
    };
    if state.headed {
        return ComputerControls {
            close: true,
            hand_back: true,
            ..ComputerControls::default()
        };
    }
    match state.control {
        Control::Bruh => ComputerControls {
            close: true,
            take: true,
            window: desktop,
            ..ComputerControls::default()
        },
        Control::Owner => ComputerControls {
            close: true,
            hand_back: true,
            navigate: true,
            input: true,
            window: desktop,
            ..ComputerControls::default()
        },
    }
}

/// The line at the top of the Computer tab.
#[must_use]
pub fn computer_status_line(state: Option<&ComputerState>, on: bool, name: &str) -> String {
    if !on {
        return "Computer is off. Turn it on in Settings.".to_string();
    }
    if let Some(error) = state.and_then(|state| state.error.as_deref()) {
        return error.to_string();
    }
    match state.filter(|state| state.session_id.is_some()) {
        None => format!(
            "The browser is closed. {name} opens it on its next step, or you can open it now."
        ),
        Some(state) if state.headed => {
            "The browser is open in a window on this computer. Hand back when you're done."
                .to_string()
        }
        Some(state) => match state.control {
            Control::Bruh => format!("{name} has the browser."),
            Control::Owner => format!("You have control. {name} waits until you hand it back."),
        },
    }
}

/// What Delete asks before a bruh goes.
#[must_use]
pub fn delete_bruh_confirm(name: &str) -> String {
    format!(
        "Delete {name}? Its chat, memory, browser and private work folder go with it. A folder you chose stays."
    )
}

/// The confirm's buttons: Delete goes ahead, Keep is the safe choice.
pub const DELETE_BRUH_YES: &str = "Delete";
pub const DELETE_BRUH_NO: &str = "Keep";

/// How a proposal to delete an agent reads.
#[must_use]
pub fn delete_agent_line(name: &str) -> String {
    format!("Delete {name}, its subscriptions and its private folder and browser.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentProvider;
    use crate::crew::{
        approval_detail, approval_summary, AccessLevel, BruhAccess, Gate, LOCAL_HOST_DENIAL,
    };
    use crate::local_tools::{ComputerState, Control};

    fn access(level: AccessLevel, send_as_me: bool, can_deliver: bool) -> BruhAccess {
        BruhAccess {
            level,
            send_as_me,
            can_deliver,
        }
    }

    fn call(command: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "command": command })
    }

    #[test]
    fn computer_needs_a_claude_bruh_that_may_act() {
        use AgentProvider::*;
        assert_eq!(
            computer_unavailable(AccessLevel::ReadOnly, Claude),
            Some("Computer needs Ask first or Full access.")
        );
        assert_eq!(
            computer_unavailable(AccessLevel::Full, Codex),
            Some("Computer needs a Claude model.")
        );
        // The provider comes first: a level change can't fix it.
        assert_eq!(
            computer_unavailable(AccessLevel::ReadOnly, Codex),
            Some("Computer needs a Claude model.")
        );
        assert_eq!(computer_unavailable(AccessLevel::Ask, Claude), None);
        assert!(computer_on(
            true,
            &access(AccessLevel::Full, false, false),
            Claude
        ));
        assert!(!computer_on(
            false,
            &access(AccessLevel::Full, false, false),
            Claude
        ));
        assert!(!computer_on(
            true,
            &access(AccessLevel::ReadOnly, false, false),
            Claude
        ));
        assert!(!computer_on(
            true,
            &access(AccessLevel::Full, false, false),
            Codex
        ));
    }

    #[test]
    fn pages_are_told_apart_by_host() {
        let pull = Some("https://github.com/o/r/pull/1");
        assert!(same_page_host(
            pull,
            Some("https://GitHub.com:443/o/r/pull/2")
        ));
        assert!(!same_page_host(pull, Some("https://shop.test/")));
        assert!(!same_page_host(pull, None));
        assert!(same_page_host(None, Some("")));
        assert!(same_page_host(None, None));
    }

    #[test]
    fn the_browser_gate_follows_the_level_and_the_forge_rule() {
        use serde_json::json;
        let claude = AgentProvider::Claude;
        let ask = access(AccessLevel::Ask, false, false);
        let full = access(AccessLevel::Full, false, false);
        let shop = Some("https://shop.test/cart");
        let pull = Some("https://github.com/o/r/pull/1");
        let click = call(json!({"action": "click", "selector": "#buy"}));
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &call(json!({"action": "snapshot"})),
                shop
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "https://a.test"})),
                None
            ),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(&ask, claude, true, &click, shop),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "https://github.com/o/r"})),
                None
            ),
            Gate::Allow
        );
        assert_eq!(browser_gate(&full, claude, true, &click, shop), Gate::Allow);
        assert_eq!(
            browser_gate(&full, claude, true, &click, pull),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, true, false),
                claude,
                true,
                &click,
                Some("https://gist.github.com/x")
            ),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, true, true),
                claude,
                true,
                &click,
                Some("https://gitlab.com/x")
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "http://127.1:3001/"})),
                None
            ),
            Gate::Deny(LOCAL_HOST_DENIAL)
        );
        assert!(matches!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "file:///etc/passwd"})),
                None
            ),
            Gate::Deny(_)
        ));
        assert_eq!(
            browser_gate(
                &full,
                claude,
                false,
                &call(json!({"action": "snapshot"})),
                shop
            ),
            Gate::Deny(COMPUTER_OFF_DENIAL)
        );
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "evaluate", "expression": "document.cookie"})),
                shop
            ),
            Gate::Deny(NOT_FOR_BRUHS)
        );
        for action in ["console", "network"] {
            assert_eq!(
                browser_gate(&full, claude, true, &call(json!({"action": action})), shop),
                Gate::Deny(NOT_FOR_BRUHS),
                "{action}"
            );
        }
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "teleport"})),
                shop
            ),
            Gate::Deny(UNKNOWN_BROWSER_ACTION)
        );
        assert_eq!(
            browser_gate(
                &full,
                AgentProvider::Codex,
                true,
                &call(json!({"action": "snapshot"})),
                shop
            ),
            Gate::Deny("Computer needs a Claude model.")
        );
        assert!(matches!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "click_at", "x": 9000, "y": 1})),
                shop
            ),
            Gate::Deny(_)
        ));
        // Read only gets nothing, not even a snapshot.
        assert_eq!(
            browser_gate(
                &access(AccessLevel::ReadOnly, true, true),
                claude,
                true,
                &call(json!({"action": "snapshot"})),
                shop
            ),
            Gate::Deny("Computer needs Ask first or Full access.")
        );
        // A forge page asks at Ask, whatever the switches say.
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Ask, true, true),
                claude,
                true,
                &click,
                pull
            ),
            Gate::Approve
        );
        // The forge rule reads the page's host, not its port, case or login.
        for page in [
            "https://GitHub.com:443/o/r",
            "https://user:pw@github.com/o/r",
            "https://github.com./o/r",
        ] {
            assert_eq!(
                browser_gate(&full, claude, true, &click, Some(page)),
                Gate::Approve,
                "{page}"
            );
        }
    }

    #[test]
    fn input_on_an_unknown_page_follows_the_forge_rule() {
        use serde_json::json;
        let claude = AgentProvider::Claude;
        let click = call(json!({"action": "click", "selector": "#buy"}));
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, false, false),
                claude,
                true,
                &click,
                None
            ),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, true, true),
                claude,
                true,
                &click,
                None
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, false, false),
                claude,
                true,
                &click,
                Some("about:blank")
            ),
            Gate::Allow
        );
    }

    #[test]
    fn forge_hosts_are_the_hosts_and_their_subdomains() {
        assert!(is_forge_host("github.com:443") && is_forge_host("API.GitLab.com.:8443"));
        assert!(!is_forge_host("shop.test:443"));
        assert!(
            is_forge_host("github.com")
                && is_forge_host("api.github.com")
                && is_forge_host("GitLab.com")
        );
        assert!(!is_forge_host("notgithub.com") && !is_forge_host("github.com.evil.test"));
    }

    #[test]
    fn the_computer_tab_offers_what_the_state_allows() {
        let running = ComputerState {
            agent_id: "a".into(),
            session_id: Some("s".into()),
            url: Some("https://a.test/".into()),
            ..Default::default()
        };
        assert_eq!(
            computer_controls(None, true, true, false),
            ComputerControls {
                open: true,
                ..Default::default()
            }
        );
        assert_eq!(
            computer_controls(Some(&running), true, true, false),
            ComputerControls {
                close: true,
                take: true,
                ..Default::default()
            }
        );
        assert!(computer_controls(Some(&running), true, true, true).window);
        let held = ComputerState {
            control: Control::Owner,
            ..running.clone()
        };
        assert_eq!(
            computer_controls(Some(&held), true, true, false),
            ComputerControls {
                close: true,
                hand_back: true,
                navigate: true,
                input: true,
                ..Default::default()
            }
        );
        assert!(computer_controls(Some(&held), true, true, true).window);
        let headed = ComputerState {
            headed: true,
            ..held.clone()
        };
        assert_eq!(
            computer_controls(Some(&headed), true, true, true),
            ComputerControls {
                close: true,
                hand_back: true,
                ..Default::default()
            }
        );
        assert_eq!(
            computer_controls(Some(&running), false, true, true),
            ComputerControls::default()
        );
        assert_eq!(
            computer_controls(Some(&running), true, false, true),
            ComputerControls::default()
        );
        assert_eq!(
            computer_status_line(None, false, "Ada"),
            "Computer is off. Turn it on in Settings."
        );
        assert_eq!(
            computer_status_line(None, true, "Ada"),
            "The browser is closed. Ada opens it on its next step, or you can open it now."
        );
        assert_eq!(
            computer_status_line(Some(&running), true, "Ada"),
            "Ada has the browser."
        );
        assert_eq!(
            computer_status_line(Some(&held), true, "Ada"),
            "You have control. Ada waits until you hand it back."
        );
        assert_eq!(
            computer_status_line(Some(&headed), true, "Ada"),
            "The browser is open in a window on this computer. Hand back when you're done."
        );
        let failed = ComputerState {
            agent_id: "a".into(),
            error: Some("Install Chrome to give a bruh a browser.".into()),
            ..Default::default()
        };
        assert_eq!(
            computer_status_line(Some(&failed), true, "Ada"),
            "Install Chrome to give a bruh a browser."
        );
        assert_eq!(
            computer_controls(Some(&failed), true, true, true),
            ComputerControls {
                open: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn browser_cards_say_what_the_bruh_would_do_where() {
        use serde_json::json;
        let on = |command: serde_json::Value| json!({"command": command, "page_url": "https://shop.test/cart"});
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &json!({"command": {"action": "navigate", "url": "https://a.test/x"}})
            ),
            "Open https://a.test/x"
        );
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "click", "selector": "#buy"}))
            ),
            "Click #buy on shop.test"
        );
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "click_at", "x": 120, "y": 340}))
            ),
            "Click at 120, 340 on shop.test"
        );
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "type", "text": "hello"}))
            ),
            "Type on shop.test"
        );
        assert_eq!(
            approval_detail(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "type", "text": "hello"}))
            )
            .as_deref(),
            Some("hello")
        );
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "key", "key": "Enter"}))
            ),
            "Press Enter on shop.test"
        );
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "fill", "selector": "#email", "text": "a@b.test"}))
            ),
            "Fill #email on shop.test"
        );
        assert_eq!(
            approval_detail(
                BRUH_BROWSER_TOOL,
                &on(json!({"action": "fill", "selector": "#email", "text": "a@b.test"}))
            )
            .as_deref(),
            Some("a@b.test")
        );
        assert_eq!(
            approval_detail(
                BRUH_BROWSER_TOOL,
                &json!({"command": {"action": "navigate", "url": "https://a.test/x"}})
            ),
            None
        );
        assert_eq!(
            approval_summary(
                BRUH_BROWSER_TOOL,
                &json!({"command": {"action": "click", "selector": "#buy"}})
            ),
            "Click #buy"
        );
        let long = format!("https://a.test/?q={}", "x".repeat(300));
        let open = json!({"command": {"action": "navigate", "url": long}});
        assert!(approval_summary(BRUH_BROWSER_TOOL, &open).ends_with('…'));
        assert_eq!(
            approval_detail(BRUH_BROWSER_TOOL, &open).as_deref(),
            Some(long.as_str())
        );
    }

    #[test]
    fn deleting_a_bruh_says_what_goes_with_it() {
        assert_eq!(
            delete_bruh_confirm("Ada"),
            "Delete Ada? Its chat, memory, browser and private work folder go with it. A folder you chose stays."
        );
        assert_eq!(
            delete_agent_line("Ada"),
            "Delete Ada, its subscriptions and its private folder and browser."
        );
        assert_eq!(
            handed_back_note("https://a.test/"),
            "The owner used your computer and handed it back. It is now on https://a.test/. Take a snapshot before you act."
        );
    }

    #[test]
    fn the_computer_waits_for_a_current_service() {
        use crate::crew::{ServiceNotice, CREW_PROTOCOL};
        assert_eq!(computer_reason(None), None, "unknown: nothing to say yet");
        assert_eq!(computer_reason(Some(CREW_PROTOCOL)), None);
        assert_eq!(
            computer_reason(Some(2)),
            Some(ServiceNotice::ServingOld.message())
        );
    }

    #[test]
    fn the_computer_switch_says_why_it_is_off() {
        use crate::crew::{ServiceNotice, CREW_PROTOCOL};
        let current = Some(CREW_PROTOCOL);
        let claude = AgentProvider::Claude;
        assert_eq!(
            computer_switch_reason(AccessLevel::ReadOnly, claude, current),
            Some("Computer needs Ask first or Full access.")
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, AgentProvider::Codex, current),
            Some(NEEDS_CLAUDE)
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, claude, Some(2)),
            Some(ServiceNotice::ServingOld.message())
        );
        // The reason the owner can fix in Settings comes first.
        assert_eq!(
            computer_switch_reason(AccessLevel::ReadOnly, claude, Some(2)),
            Some("Computer needs Ask first or Full access.")
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, claude, current),
            None
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Ask, claude, current),
            None
        );
        assert_eq!(computer_switch_reason(AccessLevel::Ask, claude, None), None);
    }

    #[test]
    fn the_unknown_action_reply_names_every_bruh_action() {
        for action in crate::local_tools::BRUH_BROWSER_ACTIONS {
            assert!(UNKNOWN_BROWSER_ACTION.contains(action), "{action}");
        }
    }

    #[test]
    fn app_copy_has_no_dashes_bullets_or_middle_dots() {
        for copy in [
            COMPUTER_HINT,
            LIVE_VIEW_HINT,
            COMPUTERS_FULL,
            TURN_ON_IN_SETTINGS,
            TAKE_CONTROL_FIRST,
            USE_THE_WINDOW,
        ] {
            assert!(
                !copy.contains(['–', '—', '•', '·']) && !copy.contains(" - "),
                "{copy}"
            );
        }
    }
}
