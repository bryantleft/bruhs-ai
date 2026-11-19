//! Who gets a Computer, how each browser call is gated, and what the
//! Computer tab offers and says. Shared by both apps and the service.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    computer_ready, effective_level, gate, is_local_host, service_notice, settings_commands,
    vault_ready, AccessLevel, BruhAccess, BruhSettings, BruhState, Gate, GatedAction,
    FORGE_COMMAND_HOSTS, LOCAL_HOST_DENIAL,
};
use crate::agent::AgentProvider;
use crate::local_tools::{
    validate_action, validate_browser_url, BrowserAction, BrowserActionKind, ComputerState,
    Control, SecretRef, COMPUTER_VIEWPORT,
};
use crate::reactive::{ReactiveAgent, ReactiveCommand, ReactiveSnapshot};
use crate::vault::{same_origin, SecretField};

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
/// How often the Computer tab reads the state while no live view runs, so
/// a browser the bruh opens shows up, in ms.
pub const COMPUTER_POLL_MS: u64 = 5_000;

// Model-facing copy.

pub const COMPUTER_OFF_DENIAL: &str =
    "Your computer is off. The owner can turn it on in your Settings.";
pub const NOT_FOR_BRUHS: &str =
    "This browser action isn't available to bruhs. Take a snapshot to read the page.";
pub const UNKNOWN_BROWSER_ACTION: &str = "Use one of the local_browser actions: navigate, snapshot, screenshot, click, click_at, type, key, scroll, fill, logins.";
pub const OWNER_HAS_COMPUTER: &str =
    "The owner has taken control of your computer. Wait for them to hand it back.";
pub const CONTROL_WAIT_EXPIRED: &str =
    "The owner still has your computer. Stop here and say you are waiting for it.";
pub const CLOSED_BY_OWNER: &str =
    "The owner closed your browser. It opens again on your next step.";
pub const ACTION_STOPPED: &str = "Stopped by the owner.";
/// The page target ended while Chrome lived on: a crash, or a page that
/// closed itself.
pub const BROWSER_PAGE_STOPPED: &str =
    "Your browser's page stopped. It opens again on your next step, so try the step again.";
/// The page target ended during a step that acts (input or a navigation),
/// which may have landed first: a click on something that closes the page.
pub const BROWSER_PAGE_CLOSED_DURING_STEP: &str =
    "Your browser's page closed during this step, so the step may have run. It opens again on your next step. Take a snapshot to see where things stand before you do anything else.";
pub const PAGE_CHANGED: &str =
    "The page changed while the owner decided. Take a snapshot and try again.";
/// Input on a page the browser didn't name: nothing can say which site it
/// would act on.
pub const PAGE_UNKNOWN: &str =
    "Your browser didn't say which page it is on. Take a snapshot, then try again.";
/// The command route got a step the gate never let through, or one that
/// already ran.
pub const STEP_NOT_APPROVED: &str = "This browser step wasn't approved.";
/// The command route got a step from a web page, not the bruh's helper.
pub const STEP_FROM_A_PAGE: &str = "Only the bruh's own browser tool can send its steps.";

// Model-facing copy for vault fills.

/// A thread browser got a vault fill or `logins`.
pub const VAULT_FOR_BRUHS: &str = "Vault logins fill only in a bruh's own browser.";
pub const FILL_TEXT_OR_SECRET: &str = "Fill with text or with a vault secret, not both.";
pub const FILL_NEEDS_VALUE: &str = "Fill needs text or a vault secret.";
pub const SUBMIT_NEEDS_SECRET: &str =
    "submit goes with a vault secret. Press Enter with the key action.";
pub const LOGIN_NOT_GRANTED: &str =
    "That vault login isn't granted to you. The owner can grant it in your Settings.";
pub const LOGIN_NO_FIELD: &str = "That login has no such field. logins lists each login's fields.";
pub const LOGIN_WRONG_ORIGIN: &str =
    "That login fills only on its own site. Open its sign-in page first; logins lists it.";
pub const SECRET_FIELD_MISMATCH: &str =
    "Fill a password into a password field, and a username or one-time code into a text field.";
pub const SCREENSHOT_AFTER_FILL: &str =
    "Screenshots are off on this page after a vault fill. Take a snapshot, or go on to the next page.";
pub const FILL_FAILED: &str =
    "The vault value couldn't be typed into that field. Take a snapshot and try again.";
pub const ONE_PASSWORD_NEEDS_OWNER: &str =
    "1Password needs the owner to sign in on this computer. Stop here and say so.";
pub const ONE_PASSWORD_UNAVAILABLE: &str =
    "1Password isn't set up for that login on this computer. Stop here and tell the owner.";
pub const KEYCHAIN_NEEDS_OWNER: &str =
    "The keychain needs the owner to allow access on this computer. Stop here and say so.";
pub const VAULT_READ_FAILED: &str =
    "Bruhs couldn't read that vault login. Stop here and tell the owner.";
/// The run preamble's paragraph for a bruh whose Computer is on and that
/// has a usable login.
pub const VAULT_PREAMBLE: &str = "Your browser can sign in with vault logins. `logins` lists the ones you may use; `fill` with `secret: {login_id, field}` types one into a field on that login's own site, and `submit: true` presses Enter. You never see the values. If a fill says the owner is needed, stop and say so.";

/// A vault login a bruh may fill, as `logins` lists it and the gate checks
/// a fill against it. Never a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginTarget {
    pub login_id: String,
    pub name: String,
    /// The sign-in page.
    pub url: String,
    /// `scheme://host[:port]`, the only origin it fills on.
    pub origin: String,
    pub fields: Vec<SecretField>,
}

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
/// The desktop's live view speaks plain `ws://` to a local server only.
pub const LIVE_VIEW_NEEDS_HTTP: &str = "The live view needs an http:// server address.";
/// The live view's socket couldn't be opened.
pub const LIVE_VIEW_UNREACHABLE: &str = "The live view can't reach the server. Try again.";
pub const TURN_ON_IN_SETTINGS: &str = "Turn on Computer in the bruh's Settings.";
/// This system lacks what a bruh's browser needs: the network guard's read
/// of this computer's addresses and the recovery of a crashed browser's
/// profile. Both exist only on macOS and Linux so far.
pub const COMPUTER_UNSUPPORTED: &str = "Computer works only on macOS and Linux for now.";
/// A launch whose traffic didn't reach the network guard: a Chrome policy
/// (an organization's proxy settings) wins over the guard's proxy flag.
pub const PROXY_POLICY_OVERRIDE: &str =
    "Chrome's proxy is set by your organization, so Bruhs won't open a bruh's browser.";

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

/// Whether the Computer switch in a bruh's Settings should follow the
/// stored value: while it is `off`, while the owner hasn't changed it since
/// the form was filled (`switch == filled`), or once the owner's value
/// matches the stored one. `Some` is then the value to show and to count as
/// filled from. Save sends SetComputer only for a change made there, so it
/// never puts back a value another app replaced. Both apps use this rule.
#[must_use]
pub fn computer_switch_follows(
    switch: bool,
    filled: bool,
    stored: bool,
    off: bool,
) -> Option<bool> {
    (off || switch == filled || switch == stored).then_some(stored)
}

/// Why the Computer switch in a bruh's Settings can't work, if it can't:
/// the model or level first, since the owner can change those there, then
/// an older service, then a system that runs no bruh browser
/// (`unsupported`, from the last [`ComputerState`] read). Both apps show
/// this one reason.
#[must_use]
pub fn computer_switch_reason(
    level: AccessLevel,
    provider: AgentProvider,
    protocol: Option<u32>,
    unsupported: bool,
) -> Option<&'static str> {
    computer_unavailable(level, provider)
        .or_else(|| computer_reason(protocol))
        .or_else(|| unsupported.then_some(COMPUTER_UNSUPPORTED))
}

/// Whether the Computer switch in a bruh's Settings is off (disabled): the
/// model or level can't have a Computer, or the service doesn't run one
/// (an unknown version counts as not yet). An unsupported system leaves
/// it usable, so a Computer switched on there can still be switched off.
#[must_use]
pub fn computer_switch_off(
    level: AccessLevel,
    provider: AgentProvider,
    protocol: Option<u32>,
) -> bool {
    computer_unavailable(level, provider).is_some() || !protocol.is_some_and(computer_ready)
}

/// What Save sends for a bruh's Settings: [`settings_commands`], without
/// SetComputer or SetVault for a service that doesn't know it.
#[must_use]
pub fn settings_commands_for(
    protocol: Option<u32>,
    agent: &ReactiveAgent,
    state: &BruhState,
    edited: &BruhSettings,
) -> Vec<ReactiveCommand> {
    let computer = protocol.is_some_and(computer_ready);
    let vault = protocol.is_some_and(vault_ready);
    settings_commands(agent, state, edited)
        .into_iter()
        .filter(|command| match command {
            ReactiveCommand::SetComputer { .. } => computer,
            ReactiveCommand::SetVault { .. } => vault,
            _ => true,
        })
        .collect()
}

/// What the Computer tab knows of a bruh's Computer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerGate {
    pub name: String,
    /// The Settings switch as stored.
    pub switched_on: bool,
    /// Why the bruh's model or level keeps a switched-on Computer off.
    pub blocked: Option<&'static str>,
    /// The service runs Computers. An unknown version counts as not yet.
    pub ready: bool,
    /// Why Computers are off on an older service.
    pub reason: Option<&'static str>,
}

impl ComputerGate {
    /// Switched on, and available to the bruh.
    #[must_use]
    pub fn on(&self) -> bool {
        self.switched_on && self.blocked.is_none()
    }

    /// On, on a service that runs it: the tab offers it and streams it.
    #[must_use]
    pub fn works(&self) -> bool {
        self.on() && self.ready
    }
}

/// Bruh `agent_id`'s Computer as the snapshot and the service's `protocol`
/// have it. An agent the snapshot doesn't list counts as Claude; a bruh it
/// doesn't list has no name and its Computer off.
#[must_use]
pub fn computer_gate(
    snapshot: Option<&ReactiveSnapshot>,
    agent_id: &str,
    protocol: Option<u32>,
) -> ComputerGate {
    let bruh = snapshot.and_then(|snapshot| {
        let bruh = snapshot
            .bruhs
            .iter()
            .find(|bruh| bruh.agent_id == agent_id)?;
        let provider = snapshot
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .map_or(AgentProvider::Claude, |agent| {
                AgentProvider::for_model(&agent.model)
            });
        Some((
            bruh.name.clone(),
            bruh.computer,
            computer_unavailable(effective_level(bruh.access.level, provider), provider),
        ))
    });
    let (name, switched_on, blocked) = bruh.unwrap_or_default();
    ComputerGate {
        name,
        switched_on,
        blocked,
        ready: protocol.is_some_and(computer_ready),
        reason: computer_reason(protocol),
    }
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
pub(super) fn page_host(url: &str) -> Option<String> {
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
///
/// A vault fill is never asked about: it runs when `login` is the usable
/// login it names, has the field and the page is on its exact origin, and
/// is refused otherwise. The forge rule doesn't apply to it.
#[must_use]
pub fn browser_gate(
    access: &BruhAccess,
    provider: AgentProvider,
    computer: bool,
    input: &Value,
    page_url: Option<&str>,
    login: Option<&LoginTarget>,
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
    if let Some((_, secret, _)) = action.secret_fill() {
        return secret_fill_gate(secret, page_url, login);
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

/// Whether a vault fill may run: never asked, only allowed or refused.
fn secret_fill_gate(
    secret: &SecretRef,
    page_url: Option<&str>,
    login: Option<&LoginTarget>,
) -> Gate {
    if page_url.is_none() {
        return Gate::Deny(PAGE_UNKNOWN);
    }
    let Some(login) = login.filter(|login| login.login_id == secret.login_id) else {
        return Gate::Deny(LOGIN_NOT_GRANTED);
    };
    if !login.fields.contains(&secret.field) {
        return Gate::Deny(LOGIN_NO_FIELD);
    }
    if !same_origin(page_url, &login.origin) {
        return Gate::Deny(LOGIN_WRONG_ORIGIN);
    }
    Gate::Allow
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
        BrowserAction::Fill {
            selector,
            text: None,
            ..
        } => format!("Fill {selector} from the vault"),
        BrowserAction::Fill { selector, .. } => format!("Fill {selector}"),
        BrowserAction::Logins => return Some("List vault logins".to_string()),
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
        BrowserAction::Type { text, .. } => Some(text),
        // A vault fill has no text, and its value is never on the call.
        BrowserAction::Fill { text, .. } => text,
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
    // A system that runs no bruh browser: nothing to open.
    if !(on && ready) || state.is_some_and(|state| state.unsupported) {
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
        // A newer service's control: only Close is safe to offer.
        Control::Other => ComputerControls {
            close: true,
            ..ComputerControls::default()
        },
    }
}

/// The line at the top of the Computer tab: an older service's notice, a
/// switch that is off, the reason a switched-on Computer can't work (its
/// model or level), the browser's error, or what the browser is doing.
#[must_use]
pub fn computer_status_line(gate: &ComputerGate, state: Option<&ComputerState>) -> String {
    if !gate.ready {
        return gate.reason.unwrap_or_default().to_string();
    }
    if !gate.switched_on {
        return "Computer is off. Turn it on in Settings.".to_string();
    }
    if let Some(blocked) = gate.blocked {
        return blocked.to_string();
    }
    let name = gate.name.as_str();
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
            Control::Other => format!("{name}'s browser is open."),
        },
    }
}

/// The line under the live view: its keys while the owner's input goes to
/// a `focused` view, the window while the browser shows in one, or Take
/// control while the bruh has it.
#[must_use]
pub fn computer_hint(
    controls: ComputerControls,
    state: Option<&ComputerState>,
    focused: bool,
) -> Option<&'static str> {
    if controls.input {
        focused.then_some(LIVE_VIEW_HINT)
    } else if state.is_some_and(|state| state.headed) {
        Some(USE_THE_WINDOW)
    } else if controls.take {
        Some(TAKE_CONTROL_FIRST)
    } else {
        None
    }
}

/// What Delete asks before a bruh goes.
#[must_use]
pub fn delete_bruh_confirm(name: &str) -> String {
    format!(
        "Delete {name}? Its chat, memory, browser, own vault entries and private work folder go with it. A folder you chose stays."
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

/// How a proposal to delete agent `id` reads: by its name, or by its id
/// when the app hasn't loaded it.
#[must_use]
pub fn delete_agent_line_for(snapshot: Option<&ReactiveSnapshot>, id: &str) -> String {
    let name = snapshot
        .and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == id))
        .map_or(id, |agent| agent.name.as_str());
    delete_agent_line(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentProvider;
    use crate::crew::{
        approval_detail, approval_summary, AccessLevel, BruhAccess, Gate, LOCAL_HOST_DENIAL,
    };
    use crate::local_tools::{ComputerState, Control};
    use crate::reactive::{ReactiveAgent, ReactiveSnapshot};

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
                shop,
                None
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "https://a.test"})),
                None,
                None
            ),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(&ask, claude, true, &click, shop, None),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "https://github.com/o/r"})),
                None,
                None
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(&full, claude, true, &click, shop, None),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(&full, claude, true, &click, pull, None),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, true, false),
                claude,
                true,
                &click,
                Some("https://gist.github.com/x"),
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
                Some("https://gitlab.com/x"),
                None
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "navigate", "url": "http://127.1:3001/"})),
                None,
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
                None,
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
                shop,
                None
            ),
            Gate::Deny(COMPUTER_OFF_DENIAL)
        );
        assert_eq!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "evaluate", "expression": "document.cookie"})),
                shop,
                None
            ),
            Gate::Deny(NOT_FOR_BRUHS)
        );
        for action in ["console", "network"] {
            assert_eq!(
                browser_gate(
                    &full,
                    claude,
                    true,
                    &call(json!({"action": action})),
                    shop,
                    None
                ),
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
                shop,
                None
            ),
            Gate::Deny(UNKNOWN_BROWSER_ACTION)
        );
        assert_eq!(
            browser_gate(
                &full,
                AgentProvider::Codex,
                true,
                &call(json!({"action": "snapshot"})),
                shop,
                None
            ),
            Gate::Deny("Computer needs a Claude model.")
        );
        assert!(matches!(
            browser_gate(
                &full,
                claude,
                true,
                &call(json!({"action": "click_at", "x": 9000, "y": 1})),
                shop,
                None
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
                shop,
                None
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
                pull,
                None
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
                browser_gate(&full, claude, true, &click, Some(page), None),
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
                None,
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
                None,
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
                Some("about:blank"),
                None
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
        let on = ComputerGate {
            name: "Ada".into(),
            switched_on: true,
            ready: true,
            ..ComputerGate::default()
        };
        let off = ComputerGate {
            switched_on: false,
            ..on.clone()
        };
        assert_eq!(
            computer_status_line(&off, None),
            "Computer is off. Turn it on in Settings."
        );
        assert_eq!(
            computer_status_line(&on, None),
            "The browser is closed. Ada opens it on its next step, or you can open it now."
        );
        assert_eq!(
            computer_status_line(&on, Some(&running)),
            "Ada has the browser."
        );
        assert_eq!(
            computer_status_line(&on, Some(&held)),
            "You have control. Ada waits until you hand it back."
        );
        assert_eq!(
            computer_status_line(&on, Some(&headed)),
            "The browser is open in a window on this computer. Hand back when you're done."
        );
        // A newer service's control: the tab still reads, and offers Close.
        let newer = ComputerState {
            control: Control::Other,
            ..running
        };
        assert_eq!(
            computer_controls(Some(&newer), true, true, true),
            ComputerControls {
                close: true,
                ..Default::default()
            }
        );
        assert_eq!(
            computer_status_line(&on, Some(&newer)),
            "Ada's browser is open."
        );
        let failed = ComputerState {
            agent_id: "a".into(),
            error: Some("Install Chrome to give a bruh a browser.".into()),
            ..Default::default()
        };
        assert_eq!(
            computer_status_line(&on, Some(&failed)),
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

    fn crew(level: AccessLevel, model: &str, computer: bool) -> ReactiveSnapshot {
        use crate::crew::{BruhLook, BruhStatus};
        ReactiveSnapshot {
            agents: vec![ReactiveAgent {
                id: "a".into(),
                name: "Ada".into(),
                instructions: String::new(),
                memory: String::new(),
                model: model.into(),
                project_path: String::new(),
                enabled: true,
            }],
            bruhs: vec![BruhState {
                agent_id: "a".into(),
                name: "Ada".into(),
                look: BruhLook::default(),
                access: access(level, false, false),
                effort: None,
                status: BruhStatus::Asleep,
                next_wake_ms: None,
                unread: 0,
                computer,
                vault: Vec::new(),
            }],
            ..ReactiveSnapshot::default()
        }
    }

    #[test]
    fn the_gate_reads_the_bruh_and_the_service() {
        use crate::crew::CREW_PROTOCOL;
        let current = Some(CREW_PROTOCOL);
        let full = crew(AccessLevel::Full, "claude-sonnet-4-6", true);
        let gate = computer_gate(Some(&full), "a", current);
        assert_eq!(
            gate,
            ComputerGate {
                name: "Ada".into(),
                switched_on: true,
                blocked: None,
                ready: true,
                reason: None,
            }
        );
        assert!(gate.on() && gate.works());
        // An agent the snapshot doesn't list counts as Claude.
        let mut unlisted = full.clone();
        unlisted.agents.clear();
        assert!(computer_gate(Some(&unlisted), "a", current).on());
        let codex = crew(AccessLevel::Full, "gpt-5-codex", true);
        let gate = computer_gate(Some(&codex), "a", current);
        assert_eq!(gate.blocked, Some(NEEDS_CLAUDE));
        assert!(!gate.on());
        let old = computer_gate(Some(&full), "a", Some(2));
        assert!(old.on() && !old.works());
        assert_eq!(old.reason, computer_reason(Some(2)));
        assert_eq!(
            computer_gate(None, "a", current),
            ComputerGate {
                ready: true,
                ..ComputerGate::default()
            }
        );
    }

    /// Computer is on, but the bruh's level or model can't have one: the
    /// tab says why, not to turn on a switch that is already on.
    #[test]
    fn a_switched_on_but_blocked_computer_says_why() {
        use crate::crew::CREW_PROTOCOL;
        let current = Some(CREW_PROTOCOL);
        let read_only = crew(AccessLevel::ReadOnly, "claude-sonnet-4-6", true);
        assert_eq!(
            computer_status_line(&computer_gate(Some(&read_only), "a", current), None),
            "Computer needs Ask first or Full access."
        );
        let codex = crew(AccessLevel::Full, "gpt-5-codex", true);
        assert_eq!(
            computer_status_line(&computer_gate(Some(&codex), "a", current), None),
            NEEDS_CLAUDE
        );
        // Stored off: turn it on in Settings, whatever else blocks it.
        let off = crew(AccessLevel::ReadOnly, "claude-sonnet-4-6", false);
        assert_eq!(
            computer_status_line(&computer_gate(Some(&off), "a", current), None),
            "Computer is off. Turn it on in Settings."
        );
        // An older service's notice comes first.
        assert_eq!(
            computer_status_line(&computer_gate(Some(&read_only), "a", Some(2)), None),
            computer_reason(Some(2)).unwrap()
        );
        assert_eq!(
            computer_status_line(&computer_gate(Some(&read_only), "a", None), None),
            "",
            "an unknown service says nothing yet"
        );
    }

    #[test]
    fn the_hint_follows_the_controls_and_focus() {
        let input = ComputerControls {
            input: true,
            ..ComputerControls::default()
        };
        let take = ComputerControls {
            take: true,
            ..ComputerControls::default()
        };
        let headed = ComputerState {
            headed: true,
            ..ComputerState::default()
        };
        assert_eq!(computer_hint(input, None, true), Some(LIVE_VIEW_HINT));
        assert_eq!(computer_hint(input, None, false), None);
        assert_eq!(
            computer_hint(ComputerControls::default(), Some(&headed), false),
            Some(USE_THE_WINDOW)
        );
        assert_eq!(computer_hint(take, None, false), Some(TAKE_CONTROL_FIRST));
        assert_eq!(computer_hint(ComputerControls::default(), None, true), None);
    }

    /// A system that runs no bruh browser offers nothing to open.
    #[test]
    fn an_unsupported_computer_offers_nothing() {
        let unsupported = ComputerState {
            agent_id: "a".into(),
            error: Some(COMPUTER_UNSUPPORTED.into()),
            unsupported: true,
            ..ComputerState::default()
        };
        assert_eq!(
            computer_controls(Some(&unsupported), true, true, true),
            ComputerControls::default()
        );
        let gate = ComputerGate {
            name: "Ada".into(),
            switched_on: true,
            ready: true,
            ..ComputerGate::default()
        };
        assert_eq!(
            computer_status_line(&gate, Some(&unsupported)),
            COMPUTER_UNSUPPORTED
        );
    }

    #[test]
    fn save_drops_set_computer_for_an_older_service() {
        use crate::crew::{current_settings, CREW_PROTOCOL};
        let snapshot = crew(AccessLevel::Full, "claude-sonnet-4-6", false);
        let (agent, state) = (&snapshot.agents[0], &snapshot.bruhs[0]);
        let mut edited = current_settings(&snapshot, "a").unwrap();
        edited.computer = true;
        let current = settings_commands_for(Some(CREW_PROTOCOL), agent, state, &edited);
        assert_eq!(current, settings_commands(agent, state, &edited));
        assert!(current
            .iter()
            .any(|command| matches!(command, ReactiveCommand::SetComputer { on: true, .. })));
        for protocol in [Some(2), None] {
            assert!(settings_commands_for(protocol, agent, state, &edited).is_empty());
        }
        edited.name = "Bea".into();
        assert_eq!(
            settings_commands_for(Some(2), agent, state, &edited).len(),
            1,
            "other edits still go"
        );
    }

    #[test]
    fn the_poll_matches_on_both_apps() {
        assert_eq!(COMPUTER_POLL_MS, 5_000);
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
            "Delete Ada? Its chat, memory, browser, own vault entries and private work folder go with it. A folder you chose stays."
        );
        assert_eq!(
            delete_agent_line("Ada"),
            "Delete Ada, its subscriptions and its private folder and browser."
        );
        let snapshot = crew(AccessLevel::Full, "claude-sonnet-4-6", false);
        assert_eq!(
            delete_agent_line_for(Some(&snapshot), "a"),
            delete_agent_line("Ada")
        );
        // An agent the app hasn't loaded is named by its id.
        assert_eq!(delete_agent_line_for(None, "zz"), delete_agent_line("zz"));
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

    /// Another app turned the Computer on while the owner had other edits:
    /// the switch follows it, so Save can't turn it off again.
    #[test]
    fn a_computer_switch_left_alone_never_puts_back_an_older_value() {
        let (on, off) = (true, false);
        // Left alone: it follows the stored value.
        assert_eq!(computer_switch_follows(off, off, on, false), Some(on));
        // Changed here: the owner's value stays.
        assert_eq!(computer_switch_follows(on, off, off, false), None);
        // Off: always the stored value.
        assert_eq!(computer_switch_follows(on, off, off, true), Some(off));
        // The owner turned it on, then another app did too: that becomes
        // what the form was filled from, so one click turns it off again
        // and stays off.
        assert_eq!(computer_switch_follows(on, off, on, false), Some(on));
        assert_eq!(computer_switch_follows(off, on, on, false), None);
    }

    #[test]
    fn the_computer_switch_says_why_it_is_off() {
        use crate::crew::{ServiceNotice, CREW_PROTOCOL};
        let current = Some(CREW_PROTOCOL);
        let claude = AgentProvider::Claude;
        assert_eq!(
            computer_switch_reason(AccessLevel::ReadOnly, claude, current, false),
            Some("Computer needs Ask first or Full access.")
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, AgentProvider::Codex, current, false),
            Some(NEEDS_CLAUDE)
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, claude, Some(2), false),
            Some(ServiceNotice::ServingOld.message())
        );
        // The reason the owner can fix in Settings comes first.
        assert_eq!(
            computer_switch_reason(AccessLevel::ReadOnly, claude, Some(2), true),
            Some("Computer needs Ask first or Full access.")
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, claude, current, false),
            None
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Ask, claude, current, false),
            None
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Ask, claude, None, false),
            None
        );
        // A system that runs no bruh browser comes last.
        assert_eq!(
            computer_switch_reason(AccessLevel::Ask, claude, current, true),
            Some(COMPUTER_UNSUPPORTED)
        );
        assert_eq!(
            computer_switch_reason(AccessLevel::Full, claude, Some(2), true),
            Some(ServiceNotice::ServingOld.message())
        );
    }

    #[test]
    fn the_computer_switch_is_off_for_the_model_level_or_service_only() {
        use crate::crew::CREW_PROTOCOL;
        let current = Some(CREW_PROTOCOL);
        let claude = AgentProvider::Claude;
        assert!(!computer_switch_off(AccessLevel::Full, claude, current));
        assert!(computer_switch_off(AccessLevel::ReadOnly, claude, current));
        assert!(computer_switch_off(
            AccessLevel::Full,
            AgentProvider::Codex,
            current
        ));
        assert!(computer_switch_off(AccessLevel::Full, claude, Some(2)));
        assert!(
            computer_switch_off(AccessLevel::Full, claude, None),
            "an unknown service counts as not yet"
        );
    }

    #[test]
    fn a_granted_login_fills_on_its_own_origin_without_asking() {
        use crate::vault::SecretField;
        use serde_json::json;
        let target = LoginTarget {
            login_id: "v1-a".into(),
            name: "Shop".into(),
            url: "https://shop.test/in".into(),
            origin: "https://shop.test".into(),
            fields: vec![SecretField::Username, SecretField::Password],
        };
        let fill = |field: &str| json!({"command": {"action": "fill", "selector": "#f", "secret": {"login_id": "v1-a", "field": field}}});
        let ask = access(AccessLevel::Ask, false, false);
        let claude = AgentProvider::Claude;
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &fill("password"),
                Some("https://shop.test/in"),
                Some(&target)
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &fill("password"),
                Some("https://evil.test/in"),
                Some(&target)
            ),
            Gate::Deny(LOGIN_WRONG_ORIGIN)
        );
        // Same host, another scheme or port: another origin.
        for page in ["http://shop.test/in", "https://shop.test:8443/in"] {
            assert_eq!(
                browser_gate(
                    &ask,
                    claude,
                    true,
                    &fill("password"),
                    Some(page),
                    Some(&target)
                ),
                Gate::Deny(LOGIN_WRONG_ORIGIN),
                "{page}"
            );
        }
        assert_eq!(
            browser_gate(&ask, claude, true, &fill("password"), None, Some(&target)),
            Gate::Deny(PAGE_UNKNOWN)
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &fill("totp"),
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Deny(LOGIN_NO_FIELD)
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &fill("password"),
                Some("https://shop.test/"),
                None
            ),
            Gate::Deny(LOGIN_NOT_GRANTED)
        );
        let other = LoginTarget {
            login_id: "v1-b".into(),
            ..target.clone()
        };
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &fill("password"),
                Some("https://shop.test/"),
                Some(&other)
            ),
            Gate::Deny(LOGIN_NOT_GRANTED)
        );
        // The forge rule doesn't apply to the fill.
        let github = LoginTarget {
            origin: "https://github.com".into(),
            ..target.clone()
        };
        assert_eq!(
            browser_gate(
                &access(AccessLevel::Full, false, false),
                claude,
                true,
                &fill("password"),
                Some("https://github.com/login"),
                Some(&github)
            ),
            Gate::Allow
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                false,
                &fill("password"),
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Deny(COMPUTER_OFF_DENIAL)
        );
        // Codex bruhs and read only bruhs get nothing.
        assert_eq!(
            browser_gate(
                &ask,
                AgentProvider::Codex,
                true,
                &fill("password"),
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Deny(NEEDS_CLAUDE)
        );
        assert_eq!(
            browser_gate(
                &ask,
                AgentProvider::Codex,
                true,
                &json!({"command": {"action": "logins"}}),
                None,
                None
            ),
            Gate::Deny(NEEDS_CLAUDE)
        );
        assert_eq!(
            browser_gate(
                &access(AccessLevel::ReadOnly, false, false),
                claude,
                true,
                &fill("password"),
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Deny(NEEDS_ASK_OR_FULL)
        );
        // Text with a secret, or submit with text, never reaches the kind rules.
        let both = json!({"command": {"action": "fill", "selector": "#f", "text": "x",
            "secret": {"login_id": "v1-a", "field": "password"}}});
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &both,
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Deny(FILL_TEXT_OR_SECRET)
        );
        let submit =
            json!({"command": {"action": "fill", "selector": "#f", "text": "x", "submit": true}});
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &submit,
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Deny(SUBMIT_NEEDS_SECRET)
        );
        // A text fill keeps the input rules.
        let text = json!({"command": {"action": "fill", "selector": "#f", "text": "x"}});
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &text,
                Some("https://shop.test/"),
                Some(&target)
            ),
            Gate::Approve
        );
        assert_eq!(
            browser_gate(
                &ask,
                claude,
                true,
                &json!({"command": {"action": "logins"}}),
                None,
                None
            ),
            Gate::Allow
        );
        assert_eq!(approval_detail(BRUH_BROWSER_TOOL, &fill("password")), None);
        let mut on_shop = fill("password");
        on_shop["page_url"] = json!("https://shop.test/in");
        assert_eq!(
            approval_summary(BRUH_BROWSER_TOOL, &on_shop),
            "Fill #f from the vault on shop.test"
        );
        assert_eq!(
            approval_summary(BRUH_BROWSER_TOOL, &json!({"command": {"action": "logins"}})),
            "List vault logins"
        );
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
