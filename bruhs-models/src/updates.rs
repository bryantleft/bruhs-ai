//! Update checks for the provider CLIs and the app: the report the server
//! sends to the web, the desktop builds in process, and the pure rules both
//! follow: which installer owns a CLI, the command that updates it, the compatibility ranges, and
//! the steps and messages of a run.
//!
//! The I/O (running `--version`, asking a registry, `git ls-remote`, running
//! an installer) lives in `bruhs_core::updates`. Nothing here touches the
//! filesystem: install detection reads paths as text.

use serde::{Deserialize, Serialize};

use crate::agent::AgentProvider;
use crate::time::EpochMs;

/// A report whose lookups all answered stays fresh this long (T3 caches a
/// found latest version for an hour).
pub const CHECK_TTL_MS: i64 = 60 * 60 * 1000;
/// A report with a failed lookup is retried after a minute (T3: one minute
/// for a version it could not find).
pub const FAILED_CHECK_TTL_MS: i64 = 60 * 1000;
/// How often an open client asks (T3's provider health refresh).
pub const POLL_INTERVAL_MS: u32 = 5 * 60 * 1000;
/// How often a client asks while an update runs.
pub const RUN_POLL_INTERVAL_MS: u32 = 1000;
/// An installer may run this long before it is stopped.
pub const UPDATE_TIMEOUT_MS: u64 = 30 * 60 * 1000;
/// Bytes of installer output kept with a run.
pub const RUN_OUTPUT_LIMIT: usize = 10_000;
/// Characters of the installer's latest line shown while it runs.
pub const LAST_LINE_LIMIT: usize = 200;
/// How long a successful run stays in the sidebar.
pub const SUCCESS_VISIBLE_MS: i64 = 3_000;

/// The npm package that ships `provider`'s CLI.
pub fn npm_package(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "@anthropic-ai/claude-code",
        AgentProvider::Codex => "@openai/codex",
    }
}

/// The executable on `PATH` (also the mise tool name).
pub fn binary_name(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "claude",
        AgentProvider::Codex => "codex",
    }
}

/// The name people know the CLI by.
pub fn provider_cli_label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Claude Code",
        AgentProvider::Codex => "Codex",
    }
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

/// A dotted release number with an optional pre-release tag, as CLIs print
/// them (`2.1.319`, `0.167.0-alpha.2`). Missing parts read as zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    parts: Vec<u64>,
    pre: Option<String>,
}

impl Version {
    /// The first version-looking token in `text` (`2.1.319 (Claude Code)`,
    /// `codex-cli 0.166.0`, a cask's `0.167.0,abc`). `None` when there is no
    /// `N.N` anywhere.
    pub fn find(text: &str) -> Option<Self> {
        text.split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '"' | '@'))
            .find_map(|token| Self::parse(token.trim_start_matches('v')))
    }

    /// Parse exactly one `N.N[.N…][-pre][+build]` token.
    pub fn parse(token: &str) -> Option<Self> {
        let token = token.split('+').next().unwrap_or(token);
        let (core, pre) = match token.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_string())),
            Some((core, _)) => (core, None),
            None => (token, None),
        };
        let parts = core
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()?;
        (parts.len() >= 2).then_some(Self { parts, pre })
    }

    fn part(&self, index: usize) -> u64 {
        self.parts.get(index).copied().unwrap_or(0)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let core = self
            .parts
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".");
        match &self.pre {
            Some(pre) => write!(f, "{core}-{pre}"),
            None => f.write_str(&core),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    /// Numeric parts first; a release sorts after its own pre-releases.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let len = self.parts.len().max(other.parts.len());
        (0..len)
            .map(|i| self.part(i).cmp(&other.part(i)))
            .find(|order| order.is_ne())
            .unwrap_or_else(|| match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

/// Whether `latest` is newer than `installed`. Unparseable text is never
/// "newer", so a registry hiccup cannot invent an update.
pub fn is_newer(latest: &str, installed: &str) -> bool {
    match (Version::find(latest), Version::find(installed)) {
        (Some(latest), Some(installed)) => latest > installed,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Compatibility
// ---------------------------------------------------------------------------

/// How well a CLI version works with this bruhs build (T3's manifest
/// statuses; its `graceful` reads as limited).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Compat {
    Supported,
    Limited,
    Unsupported,
    Broken,
}

impl Compat {
    /// Whether the card warns about this version.
    pub fn warns(self) -> bool {
        self != Self::Supported
    }

    /// Whether this version may be offered as an update.
    pub fn installable(self) -> bool {
        matches!(self, Self::Supported | Self::Limited)
    }

    /// The card's warning title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Supported => "Supported",
            Self::Limited => "Limited support",
            Self::Unsupported => "Unsupported version",
            Self::Broken => "Known broken version",
        }
    }
}

/// `[from, until)`; `None` is open.
struct CompatRange {
    from: Option<&'static str>,
    until: Option<&'static str>,
    compat: Compat,
}

struct CompatPolicy {
    recommended: &'static str,
    ranges: &'static [CompatRange],
}

/// The CLI versions whose interfaces bruhs drives (stream-json Claude Code
/// and `codex exec --json`).
fn policy(provider: AgentProvider) -> &'static CompatPolicy {
    const CLAUDE: CompatPolicy = CompatPolicy {
        recommended: ">=2.1.301",
        ranges: &[
            CompatRange {
                from: Some("2.1.280"),
                until: None,
                compat: Compat::Supported,
            },
            CompatRange {
                from: Some("2.1.111"),
                until: Some("2.1.280"),
                compat: Compat::Limited,
            },
            CompatRange {
                from: None,
                until: Some("2.1.111"),
                compat: Compat::Unsupported,
            },
        ],
    };
    const CODEX: CompatPolicy = CompatPolicy {
        recommended: ">=0.159.0",
        ranges: &[
            CompatRange {
                from: Some("0.156.0"),
                until: None,
                compat: Compat::Supported,
            },
            CompatRange {
                from: Some("0.149.0"),
                until: Some("0.156.0"),
                compat: Compat::Unsupported,
            },
            CompatRange {
                from: None,
                until: Some("0.149.0"),
                compat: Compat::Broken,
            },
        ],
    };
    match provider {
        AgentProvider::Claude => &CLAUDE,
        AgentProvider::Codex => &CODEX,
    }
}

/// Where `version` falls in `provider`'s ranges; `None` when it does not
/// parse.
pub fn compat(provider: AgentProvider, version: &str) -> Option<Compat> {
    let version = Version::find(version)?;
    let at_least =
        |bound: Option<&str>| bound.and_then(Version::parse).is_none_or(|b| version >= b);
    let below = |bound: Option<&str>| bound.and_then(Version::parse).is_none_or(|b| version < b);
    policy(provider)
        .ranges
        .iter()
        .find(|range| at_least(range.from) && below(range.until))
        .map(|range| range.compat)
}

/// The version range bruhs recommends (`>=2.1.301`).
pub fn recommended_range(provider: AgentProvider) -> &'static str {
    policy(provider).recommended
}

/// The card's warning message for an installed version.
pub fn compat_message(provider: AgentProvider, compat: Compat) -> String {
    let what = match compat {
        Compat::Supported => return String::new(),
        Compat::Limited => "has limited compatibility with",
        Compat::Unsupported => "is not supported by",
        Compat::Broken => "is known to be broken with",
    };
    format!(
        "This provider version {what} this bruhs release. Use {}.",
        recommended_range(provider)
    )
}

// ---------------------------------------------------------------------------
// Who installed the CLI
// ---------------------------------------------------------------------------

/// The tool that owns a CLI install, read from where its binary lives.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum InstallManager {
    Mise,
    /// The CLI's own installer: Claude's `~/.local/share/claude`, Codex's
    /// standalone `packages/standalone`.
    Native,
    VitePlus,
    Bun,
    Pnpm,
    Yarn,
    Volta,
    Npm,
    Homebrew,
    HomebrewCask,
    /// Not recognized.
    Unknown,
}

impl InstallManager {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mise => "mise",
            Self::Native => "native installer",
            Self::VitePlus => "Vite+",
            Self::Bun => "Bun",
            Self::Pnpm => "pnpm",
            Self::Yarn => "Yarn",
            Self::Volta => "Volta",
            Self::Npm => "npm",
            Self::Homebrew | Self::HomebrewCask => "Homebrew",
            Self::Unknown => "unknown installer",
        }
    }

    /// A stable key for lock files and queues: updates through the same
    /// installer wait for each other.
    pub fn key(self) -> &'static str {
        match self {
            Self::Mise => "mise",
            Self::Native => "native",
            Self::VitePlus => "vite-plus",
            Self::Bun => "bun",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Volta => "volta",
            Self::Npm => "npm",
            Self::Homebrew | Self::HomebrewCask => "homebrew",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the latest version comes from Homebrew rather than npm.
    /// Homebrew compares against what it can install, which can trail npm.
    pub fn uses_homebrew(self) -> bool {
        matches!(self, Self::Homebrew | Self::HomebrewCask)
    }
}

/// Where a CLI came from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub manager: InstallManager,
    /// Homebrew formula/cask, mise tool, or the npm package.
    pub package: String,
    /// npm `--prefix`, the Homebrew prefix, or Codex's standalone home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// mise only: the config requests `latest`, so `mise upgrade` moves it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tracks_latest: bool,
}

/// Classify a CLI from where it was found on `PATH` (`on_path`, not
/// resolved) and where that resolves (`resolved`, symlinks followed). Shims
/// (mise, Volta, Vite+) are read from `on_path`, because they resolve to the
/// shim tool's own binary. T3's order: the first match wins.
pub fn detect_install(provider: AgentProvider, on_path: &str, resolved: &str) -> Install {
    let on_path = on_path.replace('\\', "/");
    let resolved = resolved.replace('\\', "/");
    let lower_on = on_path.to_lowercase();
    let lower = resolved.to_lowercase();
    let either = |needle: &str| lower_on.contains(needle) || lower.contains(needle);
    let npm = npm_package(provider);
    let tool = binary_name(provider);
    let install = |manager, package: &str, prefix: Option<String>| Install {
        manager,
        package: package.to_string(),
        prefix,
        tracks_latest: false,
    };

    let mise_install = lower.contains("/mise/installs/") && !lower.contains("/mise/installs/node/");
    if either("/mise/shims/") || either("/mise/command-wrappers/") || mise_install {
        return install(InstallManager::Mise, tool, None);
    }
    let in_node_modules = lower.contains("/node_modules/");
    match provider {
        AgentProvider::Claude
            if !in_node_modules
                && (lower.contains("/.local/share/claude/")
                    || lower_on.ends_with("/.local/bin/claude")) =>
        {
            return install(InstallManager::Native, npm, None);
        }
        AgentProvider::Codex => {
            if let Some((home, _)) = resolved.split_once("/packages/standalone/") {
                return install(InstallManager::Native, npm, Some(home.to_string()));
            }
        }
        AgentProvider::Claude => {}
    }
    if either("/.vite-plus/bin/") || either("/.local/share/vite-plus/bin/") {
        return install(InstallManager::VitePlus, npm, None);
    }
    if either("/.bun/bin/") || either("/.bun/install/global/") {
        return install(InstallManager::Bun, npm, None);
    }
    if either("/.local/share/pnpm/") || either("/library/pnpm/") || either("/pnpm/global/") {
        return install(InstallManager::Pnpm, npm, None);
    }
    if either("/yarn/global/node_modules/")
        || either("/yarn/data/global/node_modules/")
        || either("/.yarn/bin/")
    {
        return install(InstallManager::Yarn, npm, None);
    }
    if either("/.volta/bin/")
        || either("/.volta/tools/image/packages/")
        || lower.ends_with("/volta-shim")
    {
        return install(InstallManager::Volta, npm, None);
    }
    let modules = format!("/lib/node_modules/{npm}/");
    if let Some(at) = resolved.find(&modules) {
        return install(InstallManager::Npm, npm, Some(resolved[..at].to_string()));
    }
    for (marker, manager) in [
        ("/Caskroom/", InstallManager::HomebrewCask),
        ("/Cellar/", InstallManager::Homebrew),
    ] {
        if let Some((prefix, rest)) = resolved.split_once(marker) {
            if let Some(name) = rest.split('/').next().filter(|name| !name.is_empty()) {
                return install(manager, name, Some(prefix.to_string()));
            }
        }
    }
    // Claude's own installer can live anywhere. Codex needs its command
    // path, and anything under node_modules belongs to a package manager
    // that was not recognized: both are updated by hand.
    if provider == AgentProvider::Claude && !in_node_modules {
        return install(InstallManager::Native, npm, None);
    }
    install(InstallManager::Unknown, npm, None)
}

/// What the update command needs beyond the install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandContext<'a> {
    /// The resolved binary (native installs run `<binary> update`).
    pub resolved: &'a str,
    /// The latest version, when known (pnpm names it exactly).
    pub latest: Option<&'a str>,
    /// `pnpm --version`; 10.4.0 and later get `--allow-build`.
    pub pnpm_version: Option<&'a str>,
}

/// The command that updates `install` in place, as argv (T3's per-installer
/// commands). `None` when it must be updated by hand.
pub fn update_command(install: &Install, context: &CommandContext<'_>) -> Option<Vec<String>> {
    let pkg = install.package.as_str();
    let at_latest = format!("{pkg}@latest");
    let owned = |argv: &[&str]| argv.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let argv = match install.manager {
        InstallManager::Mise if install.tracks_latest => owned(&["mise", "upgrade", pkg]),
        InstallManager::Mise | InstallManager::Unknown => return None,
        InstallManager::Native => owned(&[context.resolved, "update"]),
        InstallManager::VitePlus => owned(&["vp", "i", "-g", pkg]),
        InstallManager::Bun => owned(&["bun", "i", "-g", &at_latest]),
        InstallManager::Pnpm => {
            // pnpm 11 holds back fresh releases (minimumReleaseAge) unless
            // the version is named exactly.
            let target = context
                .latest
                .and_then(Version::find)
                .map_or_else(|| at_latest.clone(), |v| format!("{pkg}@{v}"));
            let allow_build = context
                .pnpm_version
                .and_then(Version::find)
                .zip(Version::parse("10.4.0"))
                .is_some_and(|(have, need)| have >= need);
            let mut argv = owned(&["pnpm", "add", "-g"]);
            if allow_build {
                argv.push(format!("--allow-build={pkg}"));
            }
            argv.push(target);
            argv
        }
        InstallManager::Yarn => owned(&["yarn", "global", "add", &at_latest]),
        InstallManager::Volta => owned(&["volta", "install", &at_latest]),
        InstallManager::Npm => {
            let mut argv = owned(&["npm", "install", "-g"]);
            if let Some(prefix) = &install.prefix {
                argv.extend(["--prefix".to_string(), prefix.clone()]);
            }
            argv.push(format!("--allow-scripts={pkg}"));
            argv.push(at_latest);
            argv
        }
        InstallManager::HomebrewCask => owned(&["brew", "upgrade", "--cask", pkg]),
        InstallManager::Homebrew => owned(&["brew", "upgrade", pkg]),
    };
    Some(argv)
}

/// What a manual-only card says ("Version X is out. bruhs runs `<path>`;
/// update it with the app or tool that installed it.").
pub fn manual_hint(install: &Install, latest: Option<&str>, resolved: &str) -> String {
    let out = latest.map_or_else(
        || "A newer version may be out.".to_string(),
        |v| format!("Version {v} is out."),
    );
    match install.manager {
        InstallManager::Mise => format!(
            "{out} bruhs runs {resolved} through mise, which pins its version. Run `mise upgrade {}` or change the version your mise config requests.",
            install.package
        ),
        _ => format!(
            "{out} bruhs runs {resolved}; update it with the app or tool that installed it."
        ),
    }
}

/// argv as one line to show or copy; arguments with spaces are quoted.
pub fn display_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '\'') {
                format!("'{}'", arg.replace('\'', r"'\''"))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The last non-empty line of installer output, without terminal control
/// codes, cut to [`LAST_LINE_LIMIT`] characters.
pub fn last_output_line(output: &str) -> Option<String> {
    let line = output
        .split(['\n', '\r'])
        .map(strip_ansi)
        .map(|line| line.trim().to_string())
        .rfind(|line| !line.is_empty())?;
    Some(line.chars().take(LAST_LINE_LIMIT).collect())
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if !c.is_control() || c == '\t' {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Provider status
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderUpdateState {
    /// The CLI is not on `PATH`.
    NotInstalled,
    UpToDate,
    Outdated,
    /// Installed, but the latest version is unknown (a failed lookup, or
    /// checks are off).
    Unknown,
}

/// One provider CLI: what is installed, what is current, and how to update.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProviderUpdate {
    pub provider: AgentProvider,
    pub state: ProviderUpdateState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// The binary after following symlinks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Install>,
    /// argv that updates it; `None` when it updates by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_command: Option<Vec<String>>,
    /// What to do by hand, when there is no command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_hint: Option<String>,
    /// The installed version against bruhs's ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<Compat>,
    /// Why a version could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProviderUpdate {
    /// Fill in the state and compatibility from the two versions. A latest
    /// version that is itself unsupported or broken is not offered.
    pub fn settle(mut self) -> Self {
        self.compat = self
            .installed
            .as_deref()
            .and_then(|v| compat(self.provider, v));
        let latest_installable = self
            .latest
            .as_deref()
            .and_then(|v| compat(self.provider, v))
            .is_none_or(Compat::installable);
        self.state = match (&self.installed, &self.latest) {
            _ if self.binary_path.is_none() && self.installed.is_none() => {
                ProviderUpdateState::NotInstalled
            }
            (Some(installed), Some(latest))
                if is_newer(latest, installed) && latest_installable =>
            {
                ProviderUpdateState::Outdated
            }
            (Some(installed), Some(latest))
                if Version::find(installed).is_some() && Version::find(latest).is_some() =>
            {
                ProviderUpdateState::UpToDate
            }
            _ => ProviderUpdateState::Unknown,
        };
        self
    }

    pub fn is_outdated(&self) -> bool {
        self.state == ProviderUpdateState::Outdated
    }

    /// Outdated and updatable without leaving bruhs.
    pub fn can_update_here(&self) -> bool {
        self.is_outdated() && self.update_command.is_some()
    }

    /// The command as one line.
    pub fn command_line(&self) -> Option<String> {
        self.update_command.as_deref().map(display_command)
    }

    /// The compatibility warning (title, message), when the installed
    /// version needs one. A Homebrew install whose latest is no better
    /// hints at stale metadata, as T3 does.
    pub fn compat_warning(&self) -> Option<(&'static str, String)> {
        let compat = self.compat.filter(|c| c.warns())?;
        let mut message = compat_message(self.provider, compat);
        let latest_bad = self
            .latest
            .as_deref()
            .and_then(|v| self::compat(self.provider, v))
            .is_some_and(|c| !c.installable());
        if let Some(install) = self.install.as_ref().filter(|i| i.manager.uses_homebrew()) {
            if latest_bad {
                message.push_str(&format!(
                    " Homebrew's local package metadata may be stale. Run `brew update && brew upgrade {}`, then check again.",
                    install.package
                ));
            }
        }
        Some((compat.title(), message))
    }

    /// One status line: `0.166.0 → 0.167.0 via Homebrew`.
    pub fn status_line(&self) -> String {
        let via = self
            .install
            .as_ref()
            .map(|install| format!(" via {}", install.manager.label()))
            .unwrap_or_default();
        match self.state {
            ProviderUpdateState::NotInstalled => format!(
                "Not installed. Install {} and make sure `{}` is on PATH.",
                provider_cli_label(self.provider),
                binary_name(self.provider)
            ),
            ProviderUpdateState::Outdated => format!(
                "{} → {}{via}",
                self.installed.as_deref().unwrap_or("?"),
                self.latest.as_deref().unwrap_or("?")
            ),
            ProviderUpdateState::UpToDate => format!(
                "{} is the latest{via}",
                self.installed.as_deref().unwrap_or("?")
            ),
            ProviderUpdateState::Unknown => match (&self.installed, &self.error) {
                (Some(installed), Some(error)) => format!("{installed}{via}. {error}"),
                (Some(installed), None) => format!("{installed}{via}"),
                (None, Some(error)) => error.clone(),
                (None, None) => "Version unknown.".into(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Provider update runs
// ---------------------------------------------------------------------------

/// Where a provider update is (T3's runner steps).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Checking,
    /// Behind another update through the same installer.
    Queued,
    Running,
    Verifying,
    Done,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Updated,
    AlreadyCurrent,
    /// The installer finished but the version did not move.
    Unchanged,
    Failed,
}

/// One provider update, from click to verification.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProviderRun {
    /// Distinct per run, so a dismissal hides only this one.
    pub id: u64,
    pub provider: AgentProvider,
    pub phase: RunPhase,
    pub started_at: EpochMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<EpochMs>,
    /// The command line, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The installer's latest line while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    /// The version it should reach, then the one it reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunOutcome>,
    /// The outcome in words (`Update command exited with code 1: …`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The tail of the installer's output.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
}

impl ProviderRun {
    pub fn is_active(&self) -> bool {
        self.phase != RunPhase::Done
    }

    /// The step in words, as the settings card shows it.
    pub fn phase_line(&self) -> String {
        match self.phase {
            RunPhase::Queued => "Waiting for another provider update to finish.".into(),
            RunPhase::Checking => "Checking for the latest version".into(),
            RunPhase::Running => match (&self.last_line, &self.command) {
                (Some(line), _) => line.clone(),
                (None, Some(command)) => format!("Running {command}"),
                (None, None) => "Running the update".into(),
            },
            RunPhase::Verifying => "Verifying the installed version".into(),
            RunPhase::Done => self.message.clone().unwrap_or_default(),
        }
    }

    /// The sidebar pill's text for this run (T3's provider pill).
    pub fn pill_label(&self) -> String {
        let label = provider_cli_label(self.provider);
        let version = self
            .to_version
            .as_deref()
            .map(|v| format!(" v{v}"))
            .unwrap_or_default();
        match self.outcome {
            Some(RunOutcome::Updated) => format!("{label} updated:{version}"),
            Some(RunOutcome::AlreadyCurrent) => format!("{label} is up to date"),
            Some(RunOutcome::Unchanged) => format!("{label} still needs an update"),
            Some(RunOutcome::Failed) => format!("{label}{version} update failed"),
            None => format!("Updating {label}"),
        }
    }

    /// Failed or unchanged: the pill stays until dismissed.
    pub fn needs_attention(&self) -> bool {
        matches!(
            self.outcome,
            Some(RunOutcome::Failed | RunOutcome::Unchanged)
        )
    }
}

/// The sidebar pill's text while runs are active (T3: "Updating Claude
/// Code", "Updating 2 providers").
pub fn running_label(runs: &[ProviderRun]) -> Option<String> {
    let active: Vec<_> = runs.iter().filter(|r| r.is_active()).collect();
    match active.as_slice() {
        [] => None,
        [one] => Some(format!("Updating {}", provider_cli_label(one.provider))),
        many => Some(format!("Updating {} providers", many.len())),
    }
}

// ---------------------------------------------------------------------------
// App (source checkout)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppUpdateState {
    UpToDate,
    /// The remote branch has commits this build does not.
    Behind,
    /// This checkout already has them; the running build is older.
    RebuildNeeded,
    /// The check could not finish (offline, no remote, no git), or checks
    /// are off.
    Unknown,
    /// Built without git metadata, so there is nothing to compare.
    Unavailable,
}

/// The GitHub repository releases come from.
pub const RELEASE_REPO: &str = "bryantleft/bruhs-ai";

/// The release assets for `arch` (`arm64`, `x86_64`): the disk image people
/// download, and the zip an installed app updates itself from. Their names
/// carry no version, so `releases/latest/download/<name>` always works.
pub fn release_asset_names(arch: &str) -> (String, String) {
    (
        format!("Bruhs-macOS-{arch}.dmg"),
        format!("Bruhs-macOS-{arch}.zip"),
    )
}

/// The checksums file published with every release.
pub const RELEASE_CHECKSUMS: &str = "SHA256SUMS";

/// The public changelog; each release has an anchor (`#v0.1.1`).
pub const CHANGELOG_URL: &str = "https://bruhs.ai/changelog";

/// The changelog entry for `version` (`0.1.1` or `v0.1.1`).
pub fn release_notes_url(version: &str) -> String {
    format!("{CHANGELOG_URL}#v{}", version.trim_start_matches('v'))
}

/// The hex SHA-256 for `name` in a `shasum -a 256` listing.
pub fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// A packaged build against the latest GitHub release.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReleaseUpdate {
    /// This app's version (`0.1.0`).
    pub current_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// The release page (notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    /// The update zip for this architecture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    /// Its SHA-256 from the release's `SHA256SUMS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl ReleaseUpdate {
    /// The changelog entry for the latest version, when there is one.
    pub fn notes_url(&self) -> Option<String> {
        self.latest_version.as_deref().map(release_notes_url)
    }
}

/// The running build against the checkout's remote default branch, or, for
/// a packaged build (`release` set), against the latest GitHub release.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppUpdate {
    pub state: AppUpdateState,
    /// The commit this binary was built from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_commit: Option<String>,
    /// `origin/main`, or whatever the remote's default branch is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_branch: Option<String>,
    /// The branch the checkout has out now, when it is not the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_branch: Option<String>,
    /// The checkout the binary was built from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    /// Shell line that pulls and rebuilds; shown to copy, never run by bruhs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Set for a packaged build: it updates from GitHub releases, not git.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseUpdate>,
}

impl AppUpdate {
    pub fn needs_action(&self) -> bool {
        matches!(
            self.state,
            AppUpdateState::Behind | AppUpdateState::RebuildNeeded
        )
    }

    /// Whether a packaged update can be downloaded and installed in place.
    pub fn installable_release(&self) -> Option<&ReleaseUpdate> {
        self.release.as_ref().filter(|r| {
            self.state == AppUpdateState::Behind && r.download_url.is_some() && r.sha256.is_some()
        })
    }

    pub fn status_line(&self) -> String {
        if let Some(release) = &self.release {
            let current = &release.current_version;
            return match self.state {
                AppUpdateState::UpToDate => format!("Bruhs v{current} is the latest release."),
                AppUpdateState::Behind => format!(
                    "Bruhs v{current} → v{}",
                    release.latest_version.as_deref().unwrap_or("?")
                ),
                _ => match &self.error {
                    Some(error) => format!("Bruhs v{current}. {error}"),
                    None => format!("Bruhs v{current}."),
                },
            };
        }
        let built = self.built_commit.as_deref().map_or("?", short_commit);
        let remote = self.remote_commit.as_deref().map_or("?", short_commit);
        let branch = self.remote_branch.as_deref().unwrap_or("origin");
        match (self.state, &self.local_branch) {
            (AppUpdateState::UpToDate, _) => {
                format!("Built from {built}, up to date with {branch}.")
            }
            (AppUpdateState::Behind, Some(local)) => format!(
                "Built from {built} on {local}. {branch} is at {remote}: merge or rebase when you are ready, then rebuild."
            ),
            (AppUpdateState::Behind, None) => {
                format!("Built from {built}. {branch} is at {remote}: pull and rebuild.")
            }
            (AppUpdateState::RebuildNeeded, _) => format!(
                "Built from {built}. This checkout already has {branch} at {remote}: rebuild to use it."
            ),
            (AppUpdateState::Unknown, _) => match &self.error {
                Some(error) => format!("Built from {built}. {error}"),
                None => format!("Built from {built}."),
            },
            (AppUpdateState::Unavailable, _) => {
                "Built without git metadata, so updates cannot be checked.".into()
            }
        }
    }
}

/// The first 7 characters of a commit id.
pub fn short_commit(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// The shell line that brings a source checkout up to date. A checkout on
/// another branch gets no pull: merging or rebasing is its owner's call.
pub fn app_update_command(
    repo_root: &str,
    state: AppUpdateState,
    on_default_branch: bool,
    rebuild: &str,
) -> Option<String> {
    let root = display_command(&[repo_root.to_string()]);
    match state {
        AppUpdateState::Behind if on_default_branch => {
            Some(format!("cd {root} && git pull --ff-only && {rebuild}"))
        }
        AppUpdateState::RebuildNeeded => Some(format!("cd {root} && {rebuild}")),
        _ => None,
    }
}

/// The default branch and its commit from `git ls-remote --symref <remote>
/// HEAD`: `ref: refs/heads/main\tHEAD` then `<sha>\tHEAD`.
pub fn parse_ls_remote_head(stdout: &str) -> Option<(String, String)> {
    let mut branch = None;
    let mut commit = None;
    for line in stdout.lines() {
        let (left, right) = line.split_once('\t')?;
        if right.trim() != "HEAD" {
            continue;
        }
        if let Some(target) = left.strip_prefix("ref: ") {
            branch = target.strip_prefix("refs/heads/").map(String::from);
        } else if left.len() >= 40 && left.chars().all(|c| c.is_ascii_hexdigit()) {
            commit = Some(left.to_string());
        }
    }
    Some((branch?, commit?))
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// Everything one check found, plus the update runs this process knows.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdatesReport {
    pub checked_at: EpochMs,
    /// Whether this check asked registries and the remote. Off when update
    /// checks are switched off: installed versions only.
    #[serde(default = "online_default")]
    pub online: bool,
    pub providers: Vec<ProviderUpdate>,
    pub app: AppUpdate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<ProviderRun>,
}

fn online_default() -> bool {
    true
}

impl UpdatesReport {
    pub fn provider(&self, provider: AgentProvider) -> Option<&ProviderUpdate> {
        self.providers.iter().find(|p| p.provider == provider)
    }

    /// The latest run for `provider`.
    pub fn run(&self, provider: AgentProvider) -> Option<&ProviderRun> {
        self.runs
            .iter()
            .filter(|r| r.provider == provider)
            .max_by_key(|r| r.id)
    }

    pub fn is_running(&self, provider: AgentProvider) -> bool {
        self.run(provider).is_some_and(ProviderRun::is_active)
    }

    pub fn any_running(&self) -> bool {
        self.runs.iter().any(ProviderRun::is_active)
    }

    /// Providers "Update all" and the sidebar's Update can run now.
    pub fn updatable_providers(&self) -> Vec<AgentProvider> {
        self.providers
            .iter()
            .filter(|p| p.can_update_here() && !self.is_running(p.provider))
            .map(|p| p.provider)
            .collect()
    }

    /// Outdated providers the confirmation skips, with why (T3's reasons).
    pub fn skipped_providers(&self) -> Vec<(AgentProvider, &'static str)> {
        self.providers
            .iter()
            .filter(|p| p.is_outdated())
            .filter_map(|p| {
                if self.is_running(p.provider) {
                    Some((p.provider, "already updating"))
                } else if p.update_command.is_none() {
                    Some((p.provider, "needs a manual update"))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Whether the sidebar shows its Update pill.
    pub fn has_updates(&self) -> bool {
        self.providers.iter().any(ProviderUpdate::is_outdated) || self.app.needs_action()
    }

    /// The pill's tooltip (T3: "Update 2 providers"), then what is behind.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        match self.updatable_providers().len() {
            0 => {}
            1 => lines.push("Update 1 provider".to_string()),
            n => lines.push(format!("Update {n} providers")),
        }
        lines.extend(
            self.providers
                .iter()
                .filter(|p| p.is_outdated())
                .map(|p| format!("{} {}", provider_cli_label(p.provider), p.status_line())),
        );
        match (self.app.state, &self.app.release) {
            (AppUpdateState::Behind, Some(release)) => lines.push(format!(
                "Bruhs v{} is out",
                release.latest_version.as_deref().unwrap_or("?")
            )),
            (AppUpdateState::Behind, None) => lines.push("bruhs: new commits on the remote".into()),
            (AppUpdateState::RebuildNeeded, _) => {
                lines.push("bruhs: rebuild to use this checkout".into());
            }
            _ => {}
        }
        lines.join("\n")
    }

    /// Whether this report can be served at `now` without checking again: an
    /// hour, or a minute when a lookup failed.
    pub fn is_fresh(&self, now: EpochMs) -> bool {
        let failed = self.providers.iter().any(|p| p.error.is_some())
            || self.app.state == AppUpdateState::Unknown;
        let ttl = if failed {
            FAILED_CHECK_TTL_MS
        } else {
            CHECK_TTL_MS
        };
        now.millis_since(self.checked_at) < ttl
    }

    /// Replace one provider's entry (after it was updated).
    pub fn with_provider(mut self, update: ProviderUpdate) -> Self {
        match self
            .providers
            .iter_mut()
            .find(|p| p.provider == update.provider)
        {
            Some(slot) => *slot = update,
            None => self.providers.push(update),
        }
        self
    }
}

/// The sidebar Update pill's label while nothing runs.
pub fn sidebar_label(report: &UpdatesReport) -> &'static str {
    if !report.updatable_providers().is_empty() {
        "Update"
    } else if report.app.needs_action() && report.app.release.is_some() {
        "Update Bruhs"
    } else if report.app.needs_action() {
        "Rebuild bruhs"
    } else {
        "Update available"
    }
}

/// The confirmation's title (T3: "Update N machine(s)?"; here providers).
pub fn confirm_title(count: usize) -> String {
    match count {
        1 => "Update 1 provider?".into(),
        n => format!("Update {n} providers?"),
    }
}

#[cfg(test)]
mod tests;
