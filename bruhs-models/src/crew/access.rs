//! Access gates: what a bruh may do at its access level, checked on every
//! tool call of an unattended run.

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::computer::{browser_detail, browser_summary, VAULT_PREAMBLE};
use super::{AccessLevel, BruhAccess, BRUH_BROWSER_TOOL};
use crate::agent::AgentProvider;
use crate::secrets::review_text;

/// How long a run waits for the owner to answer an approval.
pub const APPROVAL_WAIT_MS: u64 = 15 * 60 * 1000;

/// What happens to a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Allow,
    Approve,
    Deny(&'static str),
}

/// The kinds of action the gate table covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatedAction {
    Read,
    Write,
    Delivery,
    SendAsOwner,
}

pub const READ_ONLY_DENIAL: &str = "This bruh is read only. Return a reviewable draft instead.";
pub const APP_TOOL_DENIAL: &str = "App tools aren't available to bruh runs.";
pub const DELIVERY_DENIAL: &str =
    "Pushing, merging and opening pull requests need Full access with Can deliver on.";
pub const LOCAL_HOST_DENIAL: &str = "Bruhs can't reach services on this computer.";
/// A file tool call whose path, once resolved, is in a bruh's browser
/// profile, which holds the owner's logins.
pub const PROFILE_DENIAL: &str =
    "That path leads into a bruh's browser profile, which holds the owner's logins. Leave it alone.";
/// A file tool call that opens a file in the bruhs' folders with a second
/// name: a hard link, which could be a profile file under another path.
pub const LINKED_FILE_DENIAL: &str =
    "That file is also linked somewhere else, so it could be a private file. Bruhs doesn't open linked files in a bruh's folder.";

/// A shell line that runs `security` or `op`, at any level.
pub const VAULT_COMMAND_DENIAL: &str =
    "Bruhs can't read the keychain or 1Password from a shell. Fill a vault login in your browser instead.";
/// A file tool call whose path, once resolved, is in a folder that holds
/// the owner's saved logins: the vault catalog, the keychains or 1Password's.
pub const VAULT_FILE_DENIAL: &str = "That path holds the owner's saved logins. Leave it alone.";

/// The one credential tool a Claude bruh at Ask or Full gets when it has a
/// usable vault key (D18): run one exact command with a key's value in its
/// environment. Every use asks the owner, at every level.
pub const BRUH_SECRET_TOOL: &str = "mcp__bruhs_secrets__request_secret";
/// The run preamble's line after the key names, when [`BRUH_SECRET_TOOL`]
/// is offered (D26, ruling F28).
pub const SECRET_PREAMBLE: &str =
    "For a command that needs one, use request_secret with the key's name; the owner approves each use.";
/// A tool call that came while an approved vault command still ran, after
/// the approval wait: its environment holds a key's value.
pub const VAULT_COMMAND_RUNNING: &str = "A vault command is still running. Try again when it ends.";

/// Claude `--disallowedTools` rules every bruh run gets. `Bash(op:*)` is a
/// word prefix, the same as `Bash(op *)`: it matches `op` and `op read`,
/// never `open` or `openssl`. Claude matches them after splitting a line at
/// `&&`, `|` and `;` and stripping plain wrappers such as `timeout` or
/// `nohup`, not behind `sh -c` or a full path; [`is_vault_command`] covers
/// those. `/proc` holds other processes' environments and memory.
pub const VAULT_DENY_RULES: [&str; 3] = ["Bash(security:*)", "Bash(op:*)", "Read(//proc/**)"];

/// Forge hosts a shell command can act on as the owner. Plain `gh`, `glab`
/// and commands naming one of these ask first at Full; see
/// [`is_forge_command`] for what the check misses.
pub const FORGE_COMMAND_HOSTS: [&str; 2] = ["github.com", "gitlab.com"];

/// The line under Send as me in a bruh's Settings, on both apps.
pub const SEND_AS_ME_HINT: &str =
    "At Full access, a Claude bruh also comments, files issues and calls GitHub or GitLab as you without asking.";

/// Claude `--disallowedTools` rules that keep a run from delivering code.
pub const DELIVERY_DENY_RULES: [&str; 5] = [
    "Bash(git push:*)",
    "Bash(gh pr create:*)",
    "Bash(gh pr merge:*)",
    "Bash(glab mr create:*)",
    "Bash(glab mr merge:*)",
];

const SUMMARY_MAX_CHARS: usize = 200;

/// The level a run actually gets on `provider`. Codex can't ask, so Ask runs
/// read only; runtimes other than Claude and Codex always run read only.
#[must_use]
pub fn effective_level(level: AccessLevel, provider: AgentProvider) -> AccessLevel {
    match (provider, level) {
        (AgentProvider::Claude, level) => level,
        (AgentProvider::Codex, AccessLevel::Ask) => AccessLevel::ReadOnly,
        (AgentProvider::Codex, level) => level,
        _ => AccessLevel::ReadOnly,
    }
}

/// Why `level` can't be offered on `provider`, if it can't.
#[must_use]
pub fn level_unavailable(level: AccessLevel, provider: AgentProvider) -> Option<&'static str> {
    match (provider, level) {
        (AgentProvider::Claude, _) | (_, AccessLevel::ReadOnly) => None,
        (AgentProvider::Codex, AccessLevel::Ask) => {
            Some("Codex can't ask for approval, so a Codex bruh runs Read only or Full.")
        }
        (AgentProvider::Codex, AccessLevel::Full) => None,
        _ => Some("Only Claude and Codex bruhs can do more than read."),
    }
}

/// The gate table. `level` is the effective level for the run's provider.
#[must_use]
pub fn gate(access: &BruhAccess, level: AccessLevel, action: GatedAction) -> Gate {
    match (action, level) {
        (GatedAction::Read, _) => Gate::Allow,
        (GatedAction::Write | GatedAction::SendAsOwner, AccessLevel::ReadOnly) => {
            Gate::Deny(READ_ONLY_DENIAL)
        }
        (GatedAction::Write | GatedAction::SendAsOwner, AccessLevel::Ask) => Gate::Approve,
        (GatedAction::Write, AccessLevel::Full) => Gate::Allow,
        (GatedAction::SendAsOwner, AccessLevel::Full) => {
            if access.send_as_me {
                Gate::Allow
            } else {
                Gate::Approve
            }
        }
        (GatedAction::Delivery, AccessLevel::Full) if access.can_deliver => Gate::Allow,
        (GatedAction::Delivery, _) => Gate::Deny(DELIVERY_DENIAL),
    }
}

/// The gate for one tool call of a bruh run. `project_path` is the bruh's
/// working folder; `shell_confined` is true only when the run's shell is
/// sandboxed to that folder.
///
/// The folder check is lexical (see [`inside_folder`]): the caller must pass
/// a canonical `project_path` and canonical `file_path`/`notebook_path` in
/// `input` (symlinks resolved; for a file that doesn't exist yet, its
/// nearest existing ancestor resolved), or a symlink inside the folder can
/// point a write outside it.
#[must_use]
pub fn permission_gate(
    access: &BruhAccess,
    provider: AgentProvider,
    tool: &str,
    input: &Value,
    project_path: &str,
    shell_confined: bool,
) -> Gate {
    let level = effective_level(access.level, provider);
    if tool.starts_with("mcp__") {
        // Every use still asks the owner, in the worker, not here.
        if tool == BRUH_SECRET_TOOL
            && provider == AgentProvider::Claude
            && level != AccessLevel::ReadOnly
        {
            return Gate::Allow;
        }
        return Gate::Deny(if level == AccessLevel::ReadOnly {
            READ_ONLY_DENIAL
        } else {
            APP_TOOL_DENIAL
        });
    }
    if matches!(tool, "Read" | "Grep" | "Glob" | "LS") {
        return gate(access, level, GatedAction::Read);
    }
    // At every level, as the browser: a fetch of the local Bruhs API could
    // read the bruh's own live view session or approve its own steps.
    if tool == "WebFetch" && input["url"].as_str().is_some_and(is_local_url) {
        return Gate::Deny(LOCAL_HOST_DENIAL);
    }
    // At every level: a vault read would hand the bruh a value.
    if tool == "Bash" && input["command"].as_str().is_some_and(is_vault_command) {
        return Gate::Deny(VAULT_COMMAND_DENIAL);
    }
    if level == AccessLevel::ReadOnly {
        return Gate::Deny(READ_ONLY_DENIAL);
    }
    match tool {
        "Bash" => {
            let command = input["command"].as_str().unwrap_or_default();
            let delivery =
                is_delivery_command(command).then(|| gate(access, level, GatedAction::Delivery));
            if let Some(denied @ Gate::Deny(_)) = delivery {
                // An allowed delivery still runs the whole line, so the rest
                // of it follows the normal shell rule below.
                denied
            } else if is_forge_command(command) {
                // Commenting, filing issues or calling a forge API acts in
                // the owner's name: Can deliver only opens delivery.
                gate(access, level, GatedAction::SendAsOwner)
                    .min_approve(level == AccessLevel::Full && shell_confined)
            } else if level == AccessLevel::Full && shell_confined {
                Gate::Allow
            } else {
                Gate::Approve
            }
        }
        "SandboxNetworkAccess" if input["host"].as_str().is_some_and(is_local_host) => {
            Gate::Deny(LOCAL_HOST_DENIAL)
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = input["file_path"]
                .as_str()
                .or_else(|| input["notebook_path"].as_str())
                .unwrap_or_default();
            if inside_folder(path, project_path) {
                gate(access, level, GatedAction::Write)
            } else {
                Gate::Approve
            }
        }
        _ => Gate::Approve,
    }
}

impl Gate {
    /// An Allow that only holds when `allowed` is true; otherwise Approve.
    fn min_approve(self, allowed: bool) -> Gate {
        match self {
            Gate::Allow if allowed => Gate::Allow,
            Gate::Allow => Gate::Approve,
            other => other,
        }
    }
}

/// The words of each command in a shell line, after leading `NAME=value`
/// assignments.
fn shell_commands(command: &str) -> impl Iterator<Item = Vec<&str>> {
    command
        .split(['&', '|', ';', '\n', '\r', '(', ')', '`'])
        .map(|segment| {
            segment
                .split_whitespace()
                .skip_while(|word| is_env_assignment(word))
                .collect()
        })
}

fn is_delivery_words(words: &[&str]) -> bool {
    match words {
        ["git", rest @ ..] => git_subcommand(rest) == Some("push"),
        ["gh", "pr", "create" | "merge", ..] | ["glab", "mr", "create" | "merge", ..] => true,
        _ => false,
    }
}

/// Whether any command in a shell line pushes, merges or opens a pull
/// request. A lexical check: the sandbox and deny rules back it up.
#[must_use]
pub fn is_delivery_command(command: &str) -> bool {
    shell_commands(command).any(|words| is_delivery_words(&words))
}

/// Whether a shell line acts on a forge other than by delivering: a `gh` or
/// `glab` word (also behind `sudo`, `env` or `sh -c`), a command naming a
/// forge host, such as `curl` to its API, or a `git` command that defines an
/// alias or runs a `!` shell command.
///
/// This is a speed bump for the plain forms, not a boundary. It reads the
/// line's words, so a script, an interpreter (`python -c`), a variable, an
/// escape (`g\h`) or a split host name gets past it. With Can deliver on,
/// the sandbox lets every command reach GitHub and GitLab, and `gh` runs
/// with the owner's login: such a Full bruh can act there as the owner.
#[must_use]
pub fn is_forge_command(command: &str) -> bool {
    shell_commands(command).any(|words| {
        if is_delivery_words(&words) {
            return false;
        }
        let program = |word: &str| {
            let word = word.trim_matches(['"', '\'', '!']);
            word.rsplit('/').next().unwrap_or(word).to_owned()
        };
        // A git alias or `!` command runs whatever it names.
        if words.first().is_some_and(|first| program(first) == "git")
            && words.iter().any(|word| {
                let word = word.trim_start_matches(['"', '\'']);
                word.starts_with('!') || word.contains("alias.")
            })
        {
            return true;
        }
        if words
            .iter()
            .any(|word| matches!(program(word).as_str(), "gh" | "glab"))
        {
            return true;
        }
        words.first().is_some_and(|first| program(first) != "git")
            && words.iter().any(|word| {
                let word = word.to_ascii_lowercase();
                FORGE_COMMAND_HOSTS.iter().any(|host| word.contains(host))
            })
    })
}

/// Whether a shell line runs the keychain's `security` tool or 1Password's
/// `op`: the program of any command in it, compared without case and with
/// its folder taken off (`/usr/bin/security`). A program with `*`, `?` or
/// `[…]` counts when it could expand to one of them.
///
/// The line is read the way a shell reads it ([`shell_lines`]): quoted text
/// and quoted here-documents are words, never commands, so a commit
/// message or a search pattern that names `security` stays allowed, while
/// `$(…)`, backticks, `<(…)` and `>(…)` (also inside double quotes and
/// unquoted here-documents), subshells and pipelines are commands. The
/// program is found behind `NAME=value` words, redirections, shell words
/// such as `if` and `!`, and the wrappers `sudo`, `doas`, `env`, `command`,
/// `builtin`, `exec`, `nohup`, `noglob`, `time`, `nice`, `timeout`,
/// `stdbuf` and `xargs` with their flags. The string of `sh`, `bash`,
/// `zsh`, `dash` or `ksh -c`, of `eval` and of `env -S` is read as a line
/// of its own. A line nested deeper than [`SHELL_NESTING`] is refused.
///
/// Like [`is_forge_command`], a speed bump, not a boundary: a variable
/// (`$tool`), a name built by a substitution, a script file, a line piped
/// into a shell, an interpreter (`python -c`), a function or alias, brace
/// expansion, `find -exec` or `ssh` gets past it.
#[must_use]
pub fn is_vault_command(command: &str) -> bool {
    runs_vault(command, 0)
}

/// How deep substitutions and `sh -c` strings may nest in a line the vault
/// check reads; a deeper line is refused.
const SHELL_NESTING: usize = 16;

fn runs_vault(line: &str, depth: usize) -> bool {
    let Some(commands) = shell_lines(line, depth) else {
        return true;
    };
    commands.iter().any(|words| match command_program(words) {
        Runs::Program(program) => ["security", "op"]
            .iter()
            .any(|name| glob_matches(program.as_bytes(), name.as_bytes())),
        Runs::Script(script) => runs_vault(&script, depth + 1),
        Runs::Nothing => false,
    })
}

/// What a command runs.
enum Runs {
    /// A program, lowercased, its folder taken off.
    Program(String),
    /// A line a shell reads again: `sh -c`, `eval` or `env -S`.
    Script(String),
    Nothing,
}

/// Every command a shell line runs, as its words with quotes and escapes
/// resolved, including the commands inside substitutions. A word that held
/// a substitution keeps a `$` in its place. `None` when the line nests
/// deeper than [`SHELL_NESTING`].
fn shell_lines(line: &str, depth: usize) -> Option<Vec<Vec<String>>> {
    let chars: Vec<char> = line.chars().collect();
    let mut lexer = Lexer {
        chars: &chars,
        pos: 0,
        commands: Vec::new(),
    };
    lexer.commands_until(None, depth)?;
    Some(lexer.commands)
}

struct Lexer<'a> {
    chars: &'a [char],
    pos: usize,
    commands: Vec<Vec<String>>,
}

/// A here-document waiting for its body after the line's next newline.
struct HereDoc {
    delimiter: String,
    /// A quoted delimiter makes the body literal; otherwise its
    /// substitutions run.
    literal: bool,
    strip_tabs: bool,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    /// Read commands until `end` (a substitution's `)` or backtick) or
    /// the end of the line.
    fn commands_until(&mut self, end: Option<char>, depth: usize) -> Option<()> {
        if depth > SHELL_NESTING {
            return None;
        }
        let mut command: Vec<String> = Vec::new();
        let mut word = String::new();
        let mut in_word = false;
        let mut heredocs: Vec<HereDoc> = Vec::new();
        let end_word = |command: &mut Vec<String>, word: &mut String, in_word: &mut bool| {
            if *in_word || !word.is_empty() {
                command.push(std::mem::take(word));
            }
            *in_word = false;
        };
        while let Some(c) = self.next() {
            if Some(c) == end {
                break;
            }
            match c {
                ' ' | '\t' => end_word(&mut command, &mut word, &mut in_word),
                '\n' | ';' | '|' | ')' => {
                    end_word(&mut command, &mut word, &mut in_word);
                    self.end_command(&mut command);
                    if c == '\n' {
                        for heredoc in heredocs.drain(..) {
                            self.here_body(&heredoc, depth)?;
                        }
                    }
                }
                // `2>&1` and `<&3` keep their `&`.
                '&' if word.ends_with(['<', '>']) => word.push('&'),
                '&' => {
                    end_word(&mut command, &mut word, &mut in_word);
                    self.end_command(&mut command);
                }
                '(' if word.ends_with(['$', '<', '>']) => {
                    self.commands_until(Some(')'), depth + 1)?;
                    word.push('$');
                }
                // A subshell or a function's parentheses.
                '(' => {
                    end_word(&mut command, &mut word, &mut in_word);
                    self.end_command(&mut command);
                }
                '`' => {
                    self.commands_until(Some('`'), depth + 1)?;
                    word.push('$');
                }
                '\'' => {
                    in_word = true;
                    while let Some(quoted) = self.next() {
                        if quoted == '\'' {
                            break;
                        }
                        word.push(quoted);
                    }
                }
                '"' => {
                    in_word = true;
                    self.double_quoted(&mut word, depth)?;
                }
                '\\' => match self.next() {
                    Some('\n') | None => {}
                    Some(escaped) => {
                        word.push(escaped);
                        in_word = true;
                    }
                },
                '#' if !in_word && word.is_empty() => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                }
                // A here-string: its word is data.
                '<' if self.peek() == Some('<') && self.chars.get(self.pos + 1) == Some(&'<') => {
                    self.pos += 2;
                    word.push_str("<<<");
                    in_word = true;
                }
                '<' if self.peek() == Some('<') => {
                    self.pos += 1;
                    end_word(&mut command, &mut word, &mut in_word);
                    heredocs.push(self.here_delimiter());
                }
                other => {
                    word.push(other);
                    in_word = true;
                }
            }
        }
        end_word(&mut command, &mut word, &mut in_word);
        self.end_command(&mut command);
        Some(())
    }

    fn end_command(&mut self, command: &mut Vec<String>) {
        if !command.is_empty() {
            self.commands.push(std::mem::take(command));
        }
    }

    /// The rest of a double-quoted word: only `$(…)` and backticks run.
    fn double_quoted(&mut self, word: &mut String, depth: usize) -> Option<()> {
        while let Some(c) = self.next() {
            match c {
                '"' => break,
                '\\' => match self.peek() {
                    Some(escaped @ ('$' | '`' | '"' | '\\')) => {
                        self.pos += 1;
                        word.push(escaped);
                    }
                    Some('\n') => self.pos += 1,
                    _ => word.push('\\'),
                },
                '`' => {
                    self.commands_until(Some('`'), depth + 1)?;
                    word.push('$');
                }
                '$' if self.peek() == Some('(') => {
                    self.pos += 1;
                    self.commands_until(Some(')'), depth + 1)?;
                    word.push('$');
                }
                other => word.push(other),
            }
        }
        Some(())
    }

    /// The delimiter after `<<` or `<<-`, quotes removed.
    fn here_delimiter(&mut self) -> HereDoc {
        let strip_tabs = self.peek() == Some('-');
        if strip_tabs {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.pos += 1;
        }
        let (mut delimiter, mut literal) = (String::new(), false);
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')') {
                break;
            }
            self.pos += 1;
            match c {
                '\'' | '"' => literal = true,
                '\\' => literal = true,
                other => delimiter.push(other),
            }
        }
        HereDoc {
            delimiter,
            literal,
            strip_tabs,
        }
    }

    /// A here-document's body, up to its delimiter line: the commands of
    /// its substitutions unless it is literal.
    fn here_body(&mut self, heredoc: &HereDoc, depth: usize) -> Option<()> {
        while self.pos < self.chars.len() {
            let start = self.pos;
            while self.peek().is_some_and(|c| c != '\n') {
                self.pos += 1;
            }
            let line = &self.chars[start..self.pos];
            self.pos += 1;
            let text: String = line.iter().collect();
            let text = if heredoc.strip_tabs {
                text.trim_start_matches('\t')
            } else {
                &text
            };
            if text == heredoc.delimiter {
                break;
            }
            if !heredoc.literal {
                let mut body = Lexer {
                    chars: line,
                    pos: 0,
                    commands: Vec::new(),
                };
                body.substitutions(depth)?;
                self.commands.append(&mut body.commands);
            }
        }
        Some(())
    }

    /// The commands of the substitutions in text that is otherwise literal.
    fn substitutions(&mut self, depth: usize) -> Option<()> {
        while let Some(c) = self.next() {
            match c {
                '\\' => self.pos += 1,
                '`' => self.commands_until(Some('`'), depth + 1)?,
                '$' if self.peek() == Some('(') => {
                    self.pos += 1;
                    self.commands_until(Some(')'), depth + 1)?;
                }
                _ => {}
            }
        }
        Some(())
    }
}

/// What a command's words run, behind wrappers, assignments and
/// redirections; nothing for no program or a `command -v` lookup.
fn command_program(words: &[String]) -> Runs {
    let mut rest = words;
    loop {
        let Some((word, tail)) = rest.split_first() else {
            return Runs::Nothing;
        };
        let redirect = word.trim_start_matches(|c: char| c.is_ascii_digit());
        if redirect.starts_with(['<', '>']) {
            // A redirection: its target is the next word when not attached.
            let bare = redirect.trim_start_matches(['<', '>', '&']).is_empty();
            rest = if bare {
                tail.get(1..).unwrap_or_default()
            } else {
                tail
            };
            continue;
        }
        if word.is_empty() || is_env_assignment(word) {
            rest = tail;
            continue;
        }
        let program = word.rsplit('/').next().unwrap_or(word).to_ascii_lowercase();
        let flags_and_rest = |with_value: &[&str]| {
            let after = skip_flags(tail, with_value);
            (&tail[..tail.len() - after.len()], after)
        };
        rest = match program.as_str() {
            "sudo" | "doas" => skip_flags(
                tail,
                &[
                    "-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U", "-r", "-t",
                ],
            ),
            "env" => {
                let (flags, after) = flags_and_rest(&[
                    "-u",
                    "-C",
                    "-P",
                    "-S",
                    "--unset",
                    "--chdir",
                    "--split-string",
                ]);
                let split = flags
                    .iter()
                    .position(|flag| flag == "-S" || flag == "--split-string");
                if let Some(script) = split.and_then(|index| flags.get(index + 1)) {
                    return Runs::Script(script.clone());
                }
                after
            }
            "command" => {
                let (flags, after) = flags_and_rest(&[]);
                if flags.iter().any(|flag| flag.contains(['v', 'V'])) {
                    return Runs::Nothing;
                }
                after
            }
            "exec" => skip_flags(tail, &["-a"]),
            "nice" => skip_flags(tail, &["-n", "--adjustment"]),
            "stdbuf" => skip_flags(tail, &["-i", "-o", "-e"]),
            "xargs" => skip_flags(
                tail,
                &[
                    "-I", "-J", "-L", "-n", "-P", "-R", "-S", "-s", "-E", "-d", "-a",
                ],
            ),
            // Its duration comes before the command.
            "timeout" => skip_flags(tail, &["-s", "-k", "--signal", "--kill-after"])
                .get(1..)
                .unwrap_or_default(),
            "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                let (flags, after) = flags_and_rest(&["-o", "+o"]);
                let reads_string = flags.iter().any(|flag| {
                    flag.starts_with('-') && !flag.starts_with("--") && flag.contains('c')
                });
                return match after.first() {
                    Some(script) if reads_string => Runs::Script(script.clone()),
                    _ if reads_string => Runs::Nothing,
                    _ => Runs::Program(program),
                };
            }
            "eval" => return Runs::Script(tail.join(" ")),
            "builtin" | "nohup" | "noglob" | "time" | "!" | "{" | "if" | "then" | "else"
            | "elif" | "do" | "while" | "until" => skip_flags(tail, &[]),
            _ => return Runs::Program(program),
        };
    }
}

/// `words` after leading flags (`-x`, and the value of each flag in
/// `with_value`), and after a `--` that ends them.
fn skip_flags<'a>(words: &'a [String], with_value: &[&str]) -> &'a [String] {
    let mut rest = words;
    while let Some((flag, tail)) = rest.split_first() {
        if flag == "--" {
            return tail;
        }
        if !(flag.starts_with('-') || flag.starts_with('+')) || flag.len() < 2 {
            return rest;
        }
        rest = if with_value.contains(&flag.as_str()) {
            tail.get(1..).unwrap_or_default()
        } else {
            tail
        };
    }
    rest
}

/// One piece of a shell glob.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobToken {
    Literal(u8),
    /// `?`, or a `[…]` class taken as any one character.
    One,
    Any,
}

/// Whether a shell glob (`*`, `?`, and `[…]` taken as any one character)
/// could expand to `name`. A word without them must equal it. Iterative,
/// O(pattern × name): the pattern is the model's.
fn glob_matches(pattern: &[u8], name: &[u8]) -> bool {
    let mut tokens = Vec::with_capacity(pattern.len());
    let mut index = 0;
    while let Some(&byte) = pattern.get(index) {
        index += 1;
        tokens.push(match byte {
            b'*' => GlobToken::Any,
            b'?' => GlobToken::One,
            b'[' => match pattern[index..].iter().position(|&byte| byte == b']') {
                Some(close) => {
                    index += close + 1;
                    GlobToken::One
                }
                None => GlobToken::Literal(b'['),
            },
            other => GlobToken::Literal(other),
        });
    }
    let (mut token, mut at) = (0, 0);
    // The last `*` and the name position it was tried at.
    let mut star: Option<(usize, usize)> = None;
    while at < name.len() {
        match tokens.get(token) {
            Some(GlobToken::One) => {
                token += 1;
                at += 1;
            }
            Some(GlobToken::Literal(byte)) if *byte == name[at] => {
                token += 1;
                at += 1;
            }
            Some(GlobToken::Any) => {
                star = Some((token, at));
                token += 1;
            }
            _ => match star {
                Some((star_token, star_at)) => {
                    star = Some((star_token, star_at + 1));
                    token = star_token + 1;
                    at = star_at + 1;
                }
                None => return false,
            },
        }
    }
    tokens[token..].iter().all(|token| *token == GlobToken::Any)
}

/// Whether a host a bruh asks to reach is this computer: loopback, the
/// unspecified address, `localhost` or a `.localhost` name, in any form a
/// resolver accepts (`127.1`, `0x7f000001`, `[::1]:3001`). A bruh that
/// reached the local Bruhs API could approve its own requests.
#[must_use]
pub fn is_local_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or_default()
    } else {
        host.as_str()
    };
    let bare = bare.trim_end_matches('.');
    if bare == "localhost" || bare.ends_with(".localhost") {
        return true;
    }
    if let Ok(ip) = bare.parse::<std::net::Ipv6Addr>() {
        return is_local_ip(ip.into());
    }
    loose_ipv4(bare).is_some_and(|ip| is_local_ip(ip.into()))
}

/// The host a network tool call of a bruh run reaches, lowercased and
/// without its port or brackets: SandboxNetworkAccess's `host` and
/// WebFetch's URL host. `None` for any other tool.
#[must_use]
pub fn network_host(tool: &str, input: &Value) -> Option<String> {
    let target = match tool {
        "SandboxNetworkAccess" => input["host"].as_str()?,
        "WebFetch" => input["url"].as_str()?,
        _ => return None,
    };
    super::computer::page_host(target)
}

/// Whether a URL's host is this computer by name or literal address
/// ([`is_local_host`]). A name that resolves here takes a lookup the
/// service makes before it asks.
#[must_use]
pub fn is_local_url(url: &str) -> bool {
    super::computer::page_host(url).is_some_and(|host| is_local_host(&host))
}

/// Whether an address is this computer: loopback, unspecified, `0.0.0.0/8`,
/// or the IPv4-mapped IPv6 form of one of those.
#[must_use]
pub fn is_local_ip(ip: std::net::IpAddr) -> bool {
    let local_v4 = |ip: std::net::Ipv4Addr| ip.is_loopback() || ip.octets()[0] == 0;
    match ip {
        std::net::IpAddr::V4(ip) => local_v4(ip),
        std::net::IpAddr::V6(ip) => {
            ip.is_loopback() || ip.is_unspecified() || ip.to_ipv4_mapped().is_some_and(local_v4)
        }
    }
}

/// An IPv4 address in the forms `inet_aton` accepts: one to four parts,
/// each decimal, octal (leading 0) or hex (0x), the last filling the rest.
fn loose_ipv4(host: &str) -> Option<std::net::Ipv4Addr> {
    let number = |part: &str| -> Option<u64> {
        if let Some(hex) = part.strip_prefix("0x") {
            u64::from_str_radix(hex, 16).ok()
        } else if part.len() > 1 && part.starts_with('0') {
            u64::from_str_radix(&part[1..], 8).ok()
        } else if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let parts = host.split('.').map(number).collect::<Option<Vec<u64>>>()?;
    let (last, head) = parts.split_last()?;
    if head.len() > 3 || head.iter().any(|part| *part > 255) {
        return None;
    }
    if *last >= 1u64 << (8 * (4 - head.len())) {
        return None;
    }
    let value = head.iter().enumerate().fold(*last, |value, (index, part)| {
        value | part << (24 - 8 * index)
    });
    u32::try_from(value).ok().map(std::net::Ipv4Addr::from)
}

fn is_env_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The git subcommand after global options such as `-C dir` or `-c k=v`.
fn git_subcommand<'a>(words: &[&'a str]) -> Option<&'a str> {
    let mut words = words.iter();
    while let Some(&word) = words.next() {
        match word {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                words.next();
            }
            _ if word.starts_with('-') => {}
            _ => return Some(word),
        }
    }
    None
}

/// Claude deny rules for a run: delivery stays off unless the gate allows
/// it, and the vault rules ([`VAULT_DENY_RULES`]) always hold.
#[must_use]
pub fn claude_deny_rules(access: &BruhAccess, level: AccessLevel) -> Vec<&'static str> {
    let delivery = if gate(access, level, GatedAction::Delivery) == Gate::Allow {
        &[][..]
    } else {
        &DELIVERY_DENY_RULES[..]
    };
    [delivery, &VAULT_DENY_RULES[..]].concat()
}

/// Absolute path components with `.` and empty parts dropped; `None` for a
/// relative path or one with `..`.
fn absolute_components(path: &str) -> Option<Vec<&str>> {
    if !path.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    (!parts.contains(&"..")).then_some(parts)
}

/// Whether `path` is `folder` or inside it, compared lexically. Relative
/// paths, `..` components and an empty or root folder never count.
///
/// This crate does no I/O, so it can't see symlinks: callers must pass
/// canonical paths (symlinks resolved; for a file that doesn't exist yet,
/// its nearest existing ancestor resolved) or a symlink inside `folder` can
/// lead outside it.
#[must_use]
pub fn inside_folder(path: &str, folder: &str) -> bool {
    match (absolute_components(path), absolute_components(folder)) {
        (Some(path), Some(folder)) => !folder.is_empty() && path.starts_with(&folder),
        _ => false,
    }
}

/// A one line summary of a tool call for an approval card. A command shows
/// its first non-blank line and says how many lines follow; the card's
/// [`approval_detail`] holds the rest.
#[must_use]
pub fn approval_summary(tool: &str, input: &Value) -> String {
    let path = || {
        input["file_path"]
            .as_str()
            .or_else(|| input["notebook_path"].as_str())
    };
    let (summary, more) = match tool {
        "Edit" | "MultiEdit" | "NotebookEdit" => (path().map(|p| format!("Edit {p}")), 0),
        "Write" => (path().map(|p| format!("Write {p}")), 0),
        "Bash" => match input["command"].as_str() {
            Some(command) => {
                let mut lines = command.lines().map(str::trim).filter(|l| !l.is_empty());
                let first = lines.next().unwrap_or_default();
                (Some(format!("Run {first}")), lines.count())
            }
            None => (None, 0),
        },
        "SandboxNetworkAccess" => (
            input["host"].as_str().map(|host| format!("Reach {host}")),
            0,
        ),
        "WebFetch" => (input["url"].as_str().map(|url| format!("Fetch {url}")), 0),
        BRUH_BROWSER_TOOL => (browser_summary(input), 0),
        BRUH_SECRET_TOOL => match (input["name"].as_str(), secret_argv(input)) {
            (Some(name), Some(argv)) => {
                let joined = argv.join(" ");
                let mut lines = joined.lines();
                let first = review_text(lines.next().unwrap_or_default());
                (
                    Some(format!("Use {}: {first}", review_text(name))),
                    lines.count(),
                )
            }
            _ => (None, 0),
        },
        _ => (None, 0),
    };
    let summary = summary.unwrap_or_else(|| tool.to_string());
    let mut shown = if summary.chars().count() <= SUMMARY_MAX_CHARS {
        summary
    } else {
        let mut cut: String = summary.chars().take(SUMMARY_MAX_CHARS - 1).collect();
        cut.push('…');
        cut
    };
    match more {
        0 => {}
        1 => shown.push_str(" (+1 more line)"),
        more => shown.push_str(&format!(" (+{more} more lines)")),
    }
    shown
}

/// The most of a tool call an approval card shows, in bytes.
pub const DETAIL_MAX_BYTES: usize = 4096;
/// Ends a detail cut at [`DETAIL_MAX_BYTES`].
pub const DETAIL_CUT_NOTE: &str = "Cut at 4 KB. The full call is longer.";

/// What the call would do in full, for the card to show under its summary:
/// the whole command, the text a write or edit puts in, or the input of any
/// other tool. `None` when the summary already says it all. Cut at
/// [`DETAIL_MAX_BYTES`], ending with [`DETAIL_CUT_NOTE`] when cut.
#[must_use]
pub fn approval_detail(tool: &str, input: &Value) -> Option<String> {
    let text = match tool {
        "Bash" => {
            let command = input["command"].as_str()?;
            let summary = approval_summary(tool, input);
            if summary.strip_prefix("Run ") == Some(command.trim()) {
                return None;
            }
            command.to_string()
        }
        "SandboxNetworkAccess" => return None,
        BRUH_BROWSER_TOOL => browser_detail(input, SUMMARY_MAX_CHARS)?,
        BRUH_SECRET_TOOL => secret_argv(input)?
            .into_iter()
            .map(review_text)
            .collect::<Vec<_>>()
            .join("\n"),
        "Write" => input["content"].as_str()?.to_string(),
        "Edit" => edit_text(input),
        "MultiEdit" => input["edits"]
            .as_array()?
            .iter()
            .map(edit_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        "NotebookEdit" => input["new_source"].as_str()?.to_string(),
        _ => match input {
            Value::Null => return None,
            Value::Object(fields) if fields.is_empty() => return None,
            input => serde_json::to_string_pretty(input).ok()?,
        },
    };
    Some(bounded(text))
}

/// A vault key request's command, when every argument is text.
fn secret_argv(input: &Value) -> Option<Vec<&str>> {
    input["command"]
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect()
}

fn edit_text(edit: &Value) -> String {
    let old = edit["old_string"].as_str().unwrap_or_default();
    let new = edit["new_string"].as_str().unwrap_or_default();
    format!("Replace:\n{old}\nWith:\n{new}")
}

fn bounded(text: String) -> String {
    if text.len() <= DETAIL_MAX_BYTES {
        return text;
    }
    let mut end = DETAIL_MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n{DETAIL_CUT_NOTE}", &text[..end])
}

/// Identity of a tool call: lowercase hex SHA-256 of the tool, a newline
/// and the compact JSON input.
#[must_use]
pub fn approval_fingerprint(tool: &str, input: &Value) -> String {
    format!("{:x}", Sha256::digest(format!("{tool}\n{input}")))
}

/// The opening lines of every bruh run: its name, what it may do and, when
/// its Computer is on, its browser and, with a usable login
/// (`vault_logins`), how it fills vault logins. `vault_keys` names the keys
/// it may use through [`BRUH_SECRET_TOOL`], empty when the tool isn't
/// offered.
#[must_use]
pub fn run_preamble(
    name: &str,
    access: &BruhAccess,
    level: AccessLevel,
    computer: bool,
    vault_logins: bool,
    vault_keys: &[String],
) -> String {
    let access_line = match level {
        AccessLevel::ReadOnly => "Access: read only. You can read files in your working folder but change nothing. Produce a reviewable answer or draft.",
        AccessLevel::Ask => "Access: ask first. You may edit files and run commands in your working folder. Each change waits for the owner's approval; if one is denied, continue without it and say so.",
        AccessLevel::Full => "Access: full. You may edit files and run commands in your working folder without asking.",
    };
    let mut lines = vec![format!("Your name is {name}."), access_line.to_string()];
    if computer {
        lines.push("You have your own browser, the local_browser tool. It keeps its logins between runs. If the owner takes control, your next step waits until they hand it back.".to_string());
        if vault_logins {
            lines.push(VAULT_PREAMBLE.to_string());
        }
    }
    if !vault_keys.is_empty() {
        lines.push(format!(
            "Vault keys you may use: {}. {SECRET_PREAMBLE}",
            vault_keys.join(", ")
        ));
    }
    if gate(access, level, GatedAction::Delivery) != Gate::Allow {
        lines.push("Never push, merge or open pull requests.".to_string());
    }
    lines.push("Never send messages in the owner's name.".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full(can_deliver: bool) -> BruhAccess {
        BruhAccess {
            level: AccessLevel::Full,
            can_deliver,
            ..Default::default()
        }
    }

    #[test]
    fn the_gate_table_follows_the_spec() {
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        assert_eq!(
            gate(&ask, AccessLevel::Ask, GatedAction::Write),
            Gate::Approve
        );
        assert_eq!(
            gate(&full(false), AccessLevel::Full, GatedAction::Write),
            Gate::Allow
        );
        assert_eq!(
            gate(
                &BruhAccess::default(),
                AccessLevel::ReadOnly,
                GatedAction::Write
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            gate(&full(false), AccessLevel::Full, GatedAction::Delivery),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            gate(&full(true), AccessLevel::Full, GatedAction::Delivery),
            Gate::Allow
        );
        let ask_deliver = BruhAccess {
            can_deliver: true,
            ..ask
        };
        assert_eq!(
            gate(&ask_deliver, AccessLevel::Ask, GatedAction::Delivery),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            gate(&full(false), AccessLevel::Full, GatedAction::SendAsOwner),
            Gate::Approve
        );
        let send = BruhAccess {
            send_as_me: true,
            ..full(false)
        };
        assert_eq!(
            gate(&send, AccessLevel::Full, GatedAction::SendAsOwner),
            Gate::Allow
        );
        assert_eq!(
            gate(
                &BruhAccess {
                    send_as_me: true,
                    ..ask
                },
                AccessLevel::Ask,
                GatedAction::SendAsOwner
            ),
            Gate::Approve
        );
        assert_eq!(
            gate(
                &BruhAccess::default(),
                AccessLevel::ReadOnly,
                GatedAction::SendAsOwner
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            gate(
                &BruhAccess::default(),
                AccessLevel::ReadOnly,
                GatedAction::Read
            ),
            Gate::Allow
        );
        // The effective level, not the stored one, decides.
        assert_eq!(
            gate(&full(true), AccessLevel::ReadOnly, GatedAction::Write),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            gate(&full(true), AccessLevel::ReadOnly, GatedAction::Delivery),
            Gate::Deny(DELIVERY_DENIAL)
        );
    }

    #[test]
    fn codex_runs_an_ask_bruh_read_only() {
        assert_eq!(
            effective_level(AccessLevel::Ask, AgentProvider::Codex),
            AccessLevel::ReadOnly
        );
        assert_eq!(
            effective_level(AccessLevel::Full, AgentProvider::Codex),
            AccessLevel::Full
        );
        assert_eq!(
            effective_level(AccessLevel::Full, AgentProvider::Grok),
            AccessLevel::ReadOnly
        );
        assert_eq!(
            effective_level(AccessLevel::Ask, AgentProvider::Claude),
            AccessLevel::Ask
        );
        assert!(level_unavailable(AccessLevel::Ask, AgentProvider::Codex).is_some());
        assert_eq!(
            level_unavailable(AccessLevel::Ask, AgentProvider::Claude),
            None
        );
        assert_eq!(
            level_unavailable(AccessLevel::Full, AgentProvider::Codex),
            None
        );
        assert_eq!(
            level_unavailable(AccessLevel::Full, AgentProvider::OpenCode),
            Some("Only Claude and Codex bruhs can do more than read.")
        );
        assert_eq!(
            level_unavailable(AccessLevel::ReadOnly, AgentProvider::Cursor),
            None
        );
        let input = serde_json::json!({"file_path": "/w/x"});
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        assert_eq!(
            permission_gate(&ask, AgentProvider::Codex, "Edit", &input, "/w", true),
            Gate::Deny(READ_ONLY_DENIAL)
        );
    }

    #[test]
    fn writes_outside_the_working_folder_always_ask() {
        let input = serde_json::json!({"file_path": "/work/bruh/notes.md"});
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "Edit",
                &input,
                "/work/bruh",
                false
            ),
            Gate::Allow
        );
        for path in [
            "/etc/hosts",
            "/work/bruh/../other/x",
            "/work/bruh-evil/x",
            "notes.md",
        ] {
            let input = serde_json::json!({"file_path": path});
            assert_eq!(
                permission_gate(
                    &full(false),
                    AgentProvider::Claude,
                    "Write",
                    &input,
                    "/work/bruh",
                    true
                ),
                Gate::Approve,
                "{path}"
            );
        }
        let notebook = serde_json::json!({"notebook_path": "/work/bruh/./a.ipynb"});
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "NotebookEdit",
                &notebook,
                "/work/bruh/",
                false
            ),
            Gate::Allow
        );
        assert_eq!(
            permission_gate(
                &full(true),
                AgentProvider::Claude,
                "mcp__x__y",
                &serde_json::json!({}),
                "/work/bruh",
                true
            ),
            Gate::Deny(APP_TOOL_DENIAL)
        );
        assert_eq!(
            permission_gate(
                &BruhAccess::default(),
                AgentProvider::Claude,
                "mcp__x__y",
                &serde_json::json!({}),
                "/work/bruh",
                true
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            permission_gate(
                &BruhAccess::default(),
                AgentProvider::Claude,
                "Edit",
                &input,
                "/work/bruh",
                true
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            permission_gate(
                &full(true),
                AgentProvider::Claude,
                "WebFetch",
                &serde_json::json!({}),
                "/work/bruh",
                true
            ),
            Gate::Approve
        );
    }

    #[test]
    fn inside_folder_is_lexical_and_strict() {
        assert!(inside_folder("/a/b/c", "/a/b"));
        assert!(inside_folder("/a//b/./c", "/a/b/"));
        assert!(inside_folder("/a/b", "/a/b"));
        assert!(!inside_folder("/a/bc", "/a/b"));
        assert!(!inside_folder("/a/b/../c", "/a/b"));
        assert!(!inside_folder("/a/b/c", "/a/../a/b"));
        assert!(!inside_folder("b/c", "/a"));
        assert!(!inside_folder("/a/b", "a"));
        assert!(!inside_folder("/a/b", ""));
        assert!(!inside_folder("/a/b", "/"));
    }

    #[test]
    fn reads_are_allowed_at_every_level() {
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        let input = serde_json::json!({"file_path": "/etc/hosts", "pattern": "x"});
        for access in [BruhAccess::default(), ask, full(false)] {
            for tool in ["Read", "Grep", "Glob", "LS"] {
                for provider in [
                    AgentProvider::Claude,
                    AgentProvider::Codex,
                    AgentProvider::Grok,
                ] {
                    assert_eq!(
                        permission_gate(&access, provider, tool, &input, "/w", false),
                        Gate::Allow,
                        "{tool} {:?} {provider:?}",
                        access.level
                    );
                }
            }
        }
    }

    #[test]
    fn full_bash_asks_unless_the_shell_is_confined() {
        let input = serde_json::json!({"command": "cargo test"});
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "Bash",
                &input,
                "/w",
                true
            ),
            Gate::Allow
        );
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "Bash",
                &input,
                "/w",
                false
            ),
            Gate::Approve
        );
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        assert_eq!(
            permission_gate(&ask, AgentProvider::Claude, "Bash", &input, "/w", true),
            Gate::Approve
        );
        assert_eq!(
            permission_gate(
                &BruhAccess::default(),
                AgentProvider::Claude,
                "Bash",
                &input,
                "/w",
                true
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
    }

    #[test]
    fn forge_commands_ask_even_at_full_with_can_deliver() {
        let gate = |access: &BruhAccess, command: &str| {
            permission_gate(
                access,
                AgentProvider::Claude,
                "Bash",
                &serde_json::json!({"command": command}),
                "/w",
                true,
            )
        };
        for command in [
            "gh issue comment 3 --body hi",
            "gh api -X POST repos/a/b/issues",
            "cargo test && gh pr comment 4 -b ok",
            "sudo /usr/bin/gh release create v1",
            "sh -c 'gh issue create'",
            "glab issue note 2 -m hi",
            "curl -X POST https://api.github.com/repos/a/b/issues",
            "wget --post-data=x https://GitLab.com/api/v4/projects",
            "git -c alias.x='!gh issue comment 3 -b hi' x",
            "git -c alias.y='!curl -X POST https://api.github.com/x' y",
            "git -c 'alias.z=!sh post.sh' z",
            "git config alias.p '!gh api -X POST x' && git p",
        ] {
            assert!(is_forge_command(command), "{command}");
            assert_eq!(gate(&full(true), command), Gate::Approve, "{command}");
            assert_eq!(gate(&full(false), command), Gate::Approve, "{command}");
        }
        let send_as_me = BruhAccess {
            send_as_me: true,
            ..full(true)
        };
        assert_eq!(gate(&send_as_me, "gh issue comment 3 -b hi"), Gate::Allow);
        // Delivery itself follows Can deliver; git over a forge URL is not a forge command.
        for command in [
            "git push",
            "gh pr create --fill",
            "git clone https://github.com/a/b",
            "git -C repo status",
            "cargo test",
            "echo github",
        ] {
            assert!(!is_forge_command(command), "{command}");
            assert_eq!(gate(&full(true), command), Gate::Allow, "{command}");
        }
        assert_eq!(
            gate(&BruhAccess::default(), "gh issue list"),
            Gate::Deny(READ_ONLY_DENIAL)
        );
    }

    #[test]
    fn the_send_as_me_hint_names_the_forges_without_dashes() {
        assert!(SEND_AS_ME_HINT.contains("GitHub") && SEND_AS_ME_HINT.contains("Full access"));
        assert!(
            !SEND_AS_ME_HINT.contains(['–', '—', '•', '·']) && !SEND_AS_ME_HINT.contains(" - ")
        );
    }

    /// WebFetch of this computer is denied at every level with the same
    /// reason, like the browser; another site still asks first.
    #[test]
    fn web_fetch_never_reaches_this_computer() {
        let levels = [
            BruhAccess::default(),
            BruhAccess {
                level: AccessLevel::Ask,
                ..BruhAccess::default()
            },
            full(true),
            full(false),
        ];
        let fetch = |access: &BruhAccess, url: &str| {
            permission_gate(
                access,
                AgentProvider::Claude,
                "WebFetch",
                &serde_json::json!({"url": url, "prompt": "Read it"}),
                "/w",
                true,
            )
        };
        for url in [
            "http://127.0.0.1:3001/api/reactive/agents/a/computer",
            "https://bruhs.localhost/api/reactive/agents/a/computer",
            "http://[::1]/",
            "http://0x7f000001/",
            "http://localhost:8080",
            "https://user@127.0.0.1/x",
        ] {
            assert!(is_local_url(url), "{url}");
            for access in &levels {
                assert_eq!(
                    fetch(access, url),
                    Gate::Deny(LOCAL_HOST_DENIAL),
                    "{url} {:?}",
                    access.level
                );
            }
        }
        assert!(!is_local_url("https://example.com/localhost"));
        let host = |tool: &str, input: serde_json::Value| network_host(tool, &input);
        assert_eq!(
            host(
                "WebFetch",
                serde_json::json!({"url": "https://Api.Example.com:8443/x"})
            )
            .as_deref(),
            Some("api.example.com")
        );
        assert_eq!(
            host(
                "SandboxNetworkAccess",
                serde_json::json!({"host": "[::1]:3001"})
            )
            .as_deref(),
            Some("::1")
        );
        assert_eq!(
            host(
                "SandboxNetworkAccess",
                serde_json::json!({"host": "cdn.example"})
            )
            .as_deref(),
            Some("cdn.example")
        );
        assert_eq!(host("Bash", serde_json::json!({"command": "curl x"})), None);
        assert_eq!(fetch(&full(true), "https://example.com/"), Gate::Approve);
    }

    #[test]
    fn this_computer_is_never_reachable() {
        for host in [
            "localhost",
            "LOCALHOST.",
            "bruhs.localhost",
            "localhost:3001",
            "127.0.0.1",
            "127.8.9.10",
            "127.1",
            "2130706433",
            "0x7f000001",
            "0177.0.0.1",
            "0.0.0.0",
            "0",
            "::1",
            "[::1]",
            "[::1]:3001",
            "::",
            "::ffff:127.0.0.1",
            "127.0.0.1:3001",
        ] {
            assert!(is_local_host(host), "{host}");
            for access in [full(true), full(false)] {
                assert_eq!(
                    permission_gate(
                        &access,
                        AgentProvider::Claude,
                        "SandboxNetworkAccess",
                        &serde_json::json!({"host": host}),
                        "/w",
                        true,
                    ),
                    Gate::Deny(LOCAL_HOST_DENIAL),
                    "{host}"
                );
            }
        }
        for host in [
            "example.com",
            "localhost.example.com",
            "128.0.0.1",
            "10.0.0.1",
            "1.2.3.4",
            "::2",
            "github.com",
            "0x7f.example",
        ] {
            assert!(!is_local_host(host), "{host}");
        }
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "SandboxNetworkAccess",
                &serde_json::json!({"host": "example.com"}),
                "/w",
                true,
            ),
            Gate::Approve
        );
    }

    #[test]
    fn chained_delivery_commands_are_still_delivery() {
        for command in [
            "git push origin main",
            "cd repo && git push",
            "FOO=1 gh pr merge 3",
            "make; glab mr create",
            "cargo test\ngit push",
            "make & git push",
            "echo $(git push)",
            "git -C repo push",
            "  gh pr create --fill",
        ] {
            assert!(is_delivery_command(command), "{command}");
            let input = serde_json::json!({"command": command});
            for confined in [true, false] {
                assert_eq!(
                    permission_gate(
                        &full(false),
                        AgentProvider::Claude,
                        "Bash",
                        &input,
                        "/w",
                        confined
                    ),
                    Gate::Deny(DELIVERY_DENIAL)
                );
            }
        }
        for command in [
            "git status && cargo test",
            "echo git push",
            "gh pr view 3",
            "git pushd",
            "",
        ] {
            assert!(!is_delivery_command(command), "{command}");
        }
        let push = serde_json::json!({"command": "git push"});
        assert_eq!(
            permission_gate(
                &full(true),
                AgentProvider::Claude,
                "Bash",
                &push,
                "/w",
                true
            ),
            Gate::Allow
        );
        // Delivery allowed still needs a confined shell for the rest of the line.
        for command in ["git push", "rm -rf ~/x && git push"] {
            let input = serde_json::json!({"command": command});
            assert_eq!(
                permission_gate(
                    &full(true),
                    AgentProvider::Claude,
                    "Bash",
                    &input,
                    "/w",
                    false
                ),
                Gate::Approve,
                "{command}"
            );
        }
        let ask_deliver = BruhAccess {
            level: AccessLevel::Ask,
            can_deliver: true,
            ..Default::default()
        };
        assert_eq!(
            permission_gate(
                &ask_deliver,
                AgentProvider::Claude,
                "Bash",
                &push,
                "/w",
                true
            ),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            claude_deny_rules(&full(true), AccessLevel::Full),
            VAULT_DENY_RULES.to_vec()
        );
        assert_eq!(
            claude_deny_rules(&full(false), AccessLevel::Full),
            [DELIVERY_DENY_RULES.as_slice(), VAULT_DENY_RULES.as_slice()].concat()
        );
    }

    #[test]
    fn a_command_summary_never_hides_lines_silently() {
        let summary =
            |command: &str| approval_summary("Bash", &serde_json::json!({"command": command}));
        assert_eq!(summary("cargo test"), "Run cargo test");
        assert_eq!(summary("\n  \ncargo test\n"), "Run cargo test");
        assert_eq!(summary("cd x\n\nrm -rf y"), "Run cd x (+1 more line)");
        assert_eq!(summary("a\nb\nc\n"), "Run a (+2 more lines)");
        let long = summary(&format!("{}\nrm -rf /", "x".repeat(400)));
        assert!(long.ends_with("… (+1 more line)"), "{long}");
    }

    #[test]
    fn approval_details_hold_the_whole_call_up_to_4_kb() {
        let detail = |tool: &str, input: serde_json::Value| approval_detail(tool, &input);
        assert_eq!(
            detail("Bash", serde_json::json!({"command": "cargo test"})),
            None
        );
        assert_eq!(
            detail("Bash", serde_json::json!({"command": "cd x\nrm -rf y"})).as_deref(),
            Some("cd x\nrm -rf y")
        );
        assert_eq!(
            detail("SandboxNetworkAccess", serde_json::json!({"host": "a.com"})),
            None
        );
        assert_eq!(
            detail(
                "Write",
                serde_json::json!({"file_path": "/w/a", "content": "hi"})
            )
            .as_deref(),
            Some("hi")
        );
        assert_eq!(
            detail(
                "Edit",
                serde_json::json!({"file_path": "/w/a", "old_string": "a", "new_string": "b"})
            )
            .as_deref(),
            Some("Replace:\na\nWith:\nb")
        );
        assert_eq!(detail("WebFetch", serde_json::json!({})), None);
        assert_eq!(
            detail("WebFetch", serde_json::json!({"url": "https://a.com"})).as_deref(),
            Some("{\n  \"url\": \"https://a.com\"\n}")
        );
        let huge = format!("é{}", "x".repeat(10_000));
        let cut = detail("Write", serde_json::json!({"content": huge})).unwrap();
        assert!(cut.ends_with(DETAIL_CUT_NOTE));
        assert!(cut.len() <= DETAIL_MAX_BYTES + DETAIL_CUT_NOTE.len() + 2);
        assert!(cut.starts_with('é'));
    }

    #[test]
    fn approvals_have_short_summaries_and_stable_fingerprints() {
        let input = serde_json::json!({"command": "cargo test\nsecond line"});
        assert_eq!(
            approval_summary("Bash", &input),
            "Run cargo test (+1 more line)"
        );
        assert_eq!(
            approval_summary("Edit", &serde_json::json!({"file_path": "/w/a.rs"})),
            "Edit /w/a.rs"
        );
        assert_eq!(
            approval_summary("Write", &serde_json::json!({"file_path": "/w/a.rs"})),
            "Write /w/a.rs"
        );
        assert_eq!(
            approval_summary("WebFetch", &serde_json::json!({})),
            "WebFetch"
        );
        assert_eq!(
            approval_summary(
                "WebFetch",
                &serde_json::json!({"url": "https://example.com/a", "prompt": "Summarize"})
            ),
            "Fetch https://example.com/a"
        );
        assert_eq!(
            approval_summary(
                "SandboxNetworkAccess",
                &serde_json::json!({"host": "example.com"})
            ),
            "Reach example.com"
        );
        assert_eq!(
            approval_fingerprint("Bash", &input),
            approval_fingerprint("Bash", &input.clone())
        );
        assert_ne!(
            approval_fingerprint("Bash", &input),
            approval_fingerprint("Bash", &serde_json::json!({"command": "ls"}))
        );
        assert_ne!(
            approval_fingerprint("Bash", &input),
            approval_fingerprint("Write", &input)
        );
        let fingerprint = approval_fingerprint("Bash", &input);
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let long = approval_summary("Edit", &serde_json::json!({"file_path": "x".repeat(400)}));
        assert_eq!(long.chars().count(), 200);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn the_preamble_names_the_bruh_and_its_access() {
        let text = run_preamble("Penny", &full(false), AccessLevel::Full, false, false, &[]);
        assert!(text.starts_with("Your name is Penny."));
        assert!(
            text.contains("without asking")
                && text.contains("Never push, merge or open pull requests.")
        );
        assert!(
            !run_preamble("Penny", &full(true), AccessLevel::Full, false, false, &[])
                .contains("Never push")
        );
        assert_eq!(
            run_preamble("Penny", &BruhAccess::default(), AccessLevel::ReadOnly, false, false, &[]),
            "Your name is Penny.\n\
             Access: read only. You can read files in your working folder but change nothing. Produce a reviewable answer or draft.\n\
             Never push, merge or open pull requests.\n\
             Never send messages in the owner's name."
        );
        let ask = run_preamble(
            "Penny",
            &BruhAccess {
                level: AccessLevel::Ask,
                can_deliver: true,
                ..Default::default()
            },
            AccessLevel::Ask,
            false,
            false,
            &[],
        );
        assert!(ask.contains("Access: ask first.") && ask.contains("Never push"));
        assert!(
            run_preamble("Penny", &full(true), AccessLevel::Full, false, false, &[])
                .ends_with("Never send messages in the owner's name.")
        );
    }

    #[test]
    fn local_addresses_include_mapped_and_unspecified_forms() {
        for ip in [
            "127.0.0.1",
            "::1",
            "0.0.0.0",
            "::",
            "::ffff:127.0.0.1",
            "0.1.2.3",
        ] {
            assert!(is_local_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["10.0.0.1", "192.168.1.2", "8.8.8.8", "2001:db8::1"] {
            assert!(!is_local_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn the_run_preamble_mentions_the_browser_only_when_on() {
        let full = BruhAccess {
            level: AccessLevel::Full,
            ..Default::default()
        };
        let on = run_preamble("Ada", &full, AccessLevel::Full, true, false, &[]);
        assert!(on.contains("the local_browser tool"));
        let lines: Vec<&str> = on.lines().collect();
        assert!(lines[1].starts_with("Access: full."), "{on}");
        assert_eq!(
            lines[2],
            "You have your own browser, the local_browser tool. It keeps its logins between runs. If the owner takes control, your next step waits until they hand it back."
        );
        assert!(
            !run_preamble("Ada", &full, AccessLevel::Full, false, false, &[])
                .contains("local_browser")
        );
    }

    #[test]
    fn the_preamble_mentions_vault_logins_only_when_usable() {
        let full = BruhAccess {
            level: AccessLevel::Full,
            ..Default::default()
        };
        let on = run_preamble("Ada", &full, AccessLevel::Full, true, true, &[]);
        assert!(on.contains("You never see the values."), "{on}");
        assert!(on.contains(VAULT_PREAMBLE), "{on}");
        assert!(!run_preamble("Ada", &full, AccessLevel::Full, true, false, &[]).contains("vault"));
        assert!(!run_preamble("Ada", &full, AccessLevel::Full, false, true, &[]).contains("vault"));
    }

    #[test]
    fn the_vault_key_tool_passes_the_gate_and_its_card_shows_the_command() {
        let none = serde_json::json!({});
        for (access, level) in [
            (full(false), AccessLevel::Full),
            (full(false), AccessLevel::Ask),
        ] {
            let access = BruhAccess { level, ..access };
            assert_eq!(
                permission_gate(
                    &access,
                    AgentProvider::Claude,
                    BRUH_SECRET_TOOL,
                    &none,
                    "/w",
                    false
                ),
                Gate::Allow,
                "{level:?}"
            );
            assert_eq!(
                permission_gate(
                    &access,
                    AgentProvider::Claude,
                    "mcp__bruhs_secrets__request_connection",
                    &none,
                    "/w",
                    false
                ),
                Gate::Deny(APP_TOOL_DENIAL),
                "{level:?}"
            );
        }
        // Read only, and every runtime but Claude, gets no vault key.
        assert_eq!(
            permission_gate(
                &BruhAccess::default(),
                AgentProvider::Claude,
                BRUH_SECRET_TOOL,
                &none,
                "/w",
                false
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Codex,
                BRUH_SECRET_TOOL,
                &none,
                "/w",
                false
            ),
            Gate::Deny(APP_TOOL_DENIAL)
        );

        let call = serde_json::json!({
            "name": "K", "reason": "Ship it.", "env_var": "K", "command": ["deploy", "--prod"],
        });
        assert_eq!(
            approval_summary(BRUH_SECRET_TOOL, &call),
            "Use K: deploy --prod"
        );
        assert_eq!(
            approval_detail(BRUH_SECRET_TOOL, &call).as_deref(),
            Some("deploy\n--prod")
        );
        // A line break or control character in an argument can't hide the
        // rest of the command on the card.
        let tricky = serde_json::json!({
            "name": "K\u{1b}", "command": ["sh", "-c", "true\nrm -rf /"],
        });
        assert_eq!(
            approval_summary(BRUH_SECRET_TOOL, &tricky),
            "Use K\\u{1b}: sh -c true (+1 more line)"
        );
        assert_eq!(
            approval_detail(BRUH_SECRET_TOOL, &tricky).as_deref(),
            Some("sh\n-c\ntrue\\nrm -rf /")
        );
    }

    #[test]
    fn the_preamble_names_vault_keys_only_when_offered() {
        let full = BruhAccess {
            level: AccessLevel::Full,
            ..Default::default()
        };
        let keys = ["API_KEY".to_owned(), "DEPLOY_KEY".to_owned()];
        let on = run_preamble("Ada", &full, AccessLevel::Full, false, false, &keys);
        assert!(
            on.contains(&format!(
                "Vault keys you may use: API_KEY, DEPLOY_KEY. {SECRET_PREAMBLE}"
            )),
            "{on}"
        );
        assert_eq!(
            SECRET_PREAMBLE,
            "For a command that needs one, use request_secret with the key's name; the owner approves each use."
        );
        assert!(
            !run_preamble("Ada", &full, AccessLevel::Full, false, false, &[])
                .contains("request_secret")
        );
    }

    /// A program word of many stars can't stall the gate: the glob match
    /// is iterative, not a search over every split.
    #[test]
    fn a_word_of_many_stars_is_checked_quickly() {
        let started = std::time::Instant::now();
        for stars in [40, 200, 5000] {
            let word = format!("{}x", "*".repeat(stars));
            assert!(!is_vault_command(&word), "{stars}");
            assert!(is_vault_command(
                &format!("{}y", "*".repeat(stars)).replace('y', "op")
            ));
        }
        assert!(glob_matches(b"s*c?r[iI]t*", b"security"));
        assert!(!glob_matches(b"s*c?r[iI]t*x", b"security"));
        assert!(glob_matches(b"*", b"op") && !glob_matches(b"o", b"op"));
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "{elapsed:?}"
        );
    }

    #[test]
    fn vault_commands_and_folders_are_refused_at_every_level() {
        let deep = format!("{}ls", "$(".repeat(40));
        let refused = [
            "security find-generic-password -s bruhs-ai-secrets -w",
            "/usr/bin/security dump-keychain",
            "sh -c \"op read op://P/I/password\"",
            "FOO=1 env op item get x",
            "ls && sudo security export",
            // Wrappers, quoting, substitution and pipelines.
            "env -i PATH=/usr/bin security list-keychains",
            "sudo -u root -E /usr/bin/security find-internet-password",
            "command security dump-keychain",
            "exec op read op://P/I/password",
            "nohup op signin",
            "time nice -n 5 security find-generic-password",
            "echo x | xargs -n 1 op read",
            "printf x | xargs op read",
            "bash -c 'cd /tmp && security dump-keychain'",
            "zsh -lc \"op whoami\"",
            "'security' dump-keychain",
            "\"/usr/bin/security\" dump-keychain",
            "s\"ecur\"ity dump-keychain",
            "\\op read x",
            "SECURITY dump-keychain",
            "echo $(security find-generic-password -w)",
            "x=`op read op://P/I/password`",
            "cat <(op read op://P/I/password)",
            "op read x | pbcopy",
            "cat file | /opt/homebrew/bin/op inject",
            "timeout 5 security dump-keychain",
            "if security dump-keychain; then echo y; fi",
            "! op whoami",
            "eval \"op read x\"",
            "/usr/bin/secur* dump-keychain",
            ">out security dump-keychain",
            // Quote-aware: these still run the program.
            "bash -c 'x && security find-generic-password'",
            "sh -c \"op read op://a/b/c\"",
            "echo \"$(security find-generic-password -w)\"",
            "echo \"`op read x`\"",
            "(cd /tmp; security dump-keychain)",
            "env -S \"op read x\"",
            "sudo sh -c \"eval 'security dump-keychain'\"",
            "cat <<EOF\n$(security dump-keychain)\nEOF",
            "cat <<-EOF\n\t`op read x`\n\tEOF\nls",
            "cat <<'EOF'\nliteral\nEOF\nsecurity dump-keychain",
            "ls 2>&1 | op read x",
            "f() { security dump-keychain; }",
            "x=1; op read x",
            // Nested deeper than anyone writes by hand.
            &deep,
        ];
        for line in refused {
            assert!(is_vault_command(line), "{line}");
            for level in [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full] {
                for can_deliver in [false, true] {
                    let access = BruhAccess {
                        level,
                        can_deliver,
                        ..Default::default()
                    };
                    assert_eq!(
                        permission_gate(
                            &access,
                            AgentProvider::Claude,
                            "Bash",
                            &serde_json::json!({"command": line}),
                            "/w",
                            true
                        ),
                        Gate::Deny(VAULT_COMMAND_DENIAL),
                        "{line} {level:?}"
                    );
                }
            }
        }
        for line in [
            "echo security",
            "git log --oneline",
            "cat opinions.txt",
            "./op-tool",
            "grep op src/main.rs",
            "open https://example.com",
            "openssl rand -hex 16",
            "opam install dune",
            "cargo test security",
            "command -v security",
            "sh scripts/op.sh",
            // Quoted text is data, never a command.
            "git commit -m \"fix(security): patch xss\"",
            "git commit -m \"feat(op): read references\"",
            "git commit -m \"$(cat <<'EOF'\nfeat(vault): keep bruhs out\n\nSecurity: op read is refused; security too\nEOF\n)\"",
            "git commit -F - <<EOF\nSecurity fixes | op\nEOF",
            "gh pr create --title \"Vault\" --body \"Keeps bruhs out; op is refused | security too\"",
            "git log --grep='(security)'",
            "grep 'a|op' notes.txt",
            "echo done # security dump-keychain",
            "cat <<< \"security dump-keychain\"",
            "bash -c 'echo security'",
        ] {
            assert!(!is_vault_command(line), "{line}");
        }
        for access in [full(true), full(false), BruhAccess::default()] {
            let rules = claude_deny_rules(&access, access.level);
            assert!(
                VAULT_DENY_RULES.iter().all(|rule| rules.contains(rule)),
                "{access:?}"
            );
        }
    }
}
