//! Shortcut rules. A rule is a key, a command id, and an optional `when`
//! clause. The last rule whose key and condition both match wins.
//!
//! `mod` is Command on macOS and Control on other systems. A new default is
//! appended only when its command id is not already in the saved list, so a
//! customized command keeps the keys the user stored.

use serde::{Deserialize, Serialize};

/// One shortcut. `when` is absent when the rule is always eligible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybinding {
    pub key: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

/// A key with its modifiers already resolved for this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: String,
    pub command: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Context keys a `when` clause can read. An unknown name is false.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyContext {
    pub terminal_focus: bool,
    pub terminal_open: bool,
    pub preview_focus: bool,
    pub preview_open: bool,
    pub model_picker_open: bool,
    pub usage_page_open: bool,
    pub composer_focus: bool,
    pub composer_draft: bool,
    pub turn_running: bool,
    pub editable_focus: bool,
    pub is_web: bool,
    pub is_desktop: bool,
}

impl KeyContext {
    fn flag(self, name: &str) -> bool {
        match name {
            "terminalFocus" => self.terminal_focus,
            "terminalOpen" => self.terminal_open,
            "previewFocus" => self.preview_focus,
            "previewOpen" => self.preview_open,
            "modelPickerOpen" => self.model_picker_open,
            "usagePageOpen" => self.usage_page_open,
            "composerFocus" => self.composer_focus,
            "composerDraft" => self.composer_draft,
            "turnRunning" => self.turn_running,
            "editableFocus" => self.editable_focus,
            "isWeb" => self.is_web,
            "isDesktop" => self.is_desktop,
            _ => false,
        }
    }
}

/// Bindings the app ships. Several rows can share a command id.
pub fn default_keybindings() -> Vec<Keybinding> {
    const ROWS: &[(&str, &str, Option<&str>)] = &[
        ("mod+k", "command.palette", None),
        ("mod+shift+space", "capture.open", None),
        ("mod+shift+y", "view.today", None),
        ("mod+shift+k", "view.knowledge", None),
        ("mod+shift+i", "view.inbox", None),
        ("mod+n", "chat.new", None),
        ("mod+shift+n", "chat.new", None),
        ("mod+shift+o", "chat.new", None),
        ("mod+b", "sidebar.toggle", None),
        ("mod+alt+b", "rightPanel.toggle", None),
        ("mod+w", "rightPanel.close", None),
        ("alt+w", "rightPanel.close", None),
        ("mod+d", "diff.toggle", None),
        ("mod+shift+t", "tasks.toggle", None),
        ("mod+shift+m", "model.picker", None),
        ("mod+shift+x", "workspace.menu", None),
        ("mod+shift+g", "branch.menu", None),
        ("mod+shift+e", "effort.menu", None),
        ("mod+shift+l", "worktree.previous", None),
        ("mod+p", "mention.file", None),
        ("mod+shift+[", "thread.previous", None),
        ("mod+alt+up", "thread.previous", None),
        ("mod+shift+]", "thread.next", None),
        ("mod+alt+down", "thread.next", None),
        ("mod+1", "thread.jump.1", Some("!modelPickerOpen")),
        ("mod+2", "thread.jump.2", Some("!modelPickerOpen")),
        ("mod+3", "thread.jump.3", Some("!modelPickerOpen")),
        ("mod+4", "thread.jump.4", Some("!modelPickerOpen")),
        ("mod+5", "thread.jump.5", Some("!modelPickerOpen")),
        ("mod+6", "thread.jump.6", Some("!modelPickerOpen")),
        ("mod+7", "thread.jump.7", Some("!modelPickerOpen")),
        ("mod+8", "thread.jump.8", Some("!modelPickerOpen")),
        ("mod+9", "thread.jump.9", Some("!modelPickerOpen")),
        ("mod+shift+p", "thread.pin", None),
        ("mod+shift+s", "thread.settle", None),
        ("mod+shift+c", "thread.copyLink", None),
        ("mod+/", "settings.open", None),
        ("mod+j", "terminal.toggle", None),
        ("mod+shift+j", "preview.toggle", None),
        ("mod+s", "draft.stash", None),
        ("mod+shift+f", "search.open", None),
        (
            "mod+f",
            "chat.find",
            Some("!terminalFocus && !previewFocus"),
        ),
        ("mod+o", "project.openInEditor", None),
        ("mod+alt+o", "project.openInEditor", None),
        ("mod+z", "thread.undo", None),
        ("escape", "dialog.dismiss", None),
        ("mod+[", "navigation.back", Some("!terminalFocus")),
        ("mod+]", "navigation.forward", Some("!terminalFocus")),
        ("/", "composer.focus", Some("!editableFocus")),
        ("c", "task.new", Some("!editableFocus")),
        // Unbound until the user picks a key. Cmd+Shift+C and Cmd+Shift+K
        // stay the thread link and Knowledge.
        ("", "pull.copyLink", None),
        ("", "pull.copyNumber", None),
    ];
    ROWS.iter()
        .map(|(key, command, when)| Keybinding {
            key: (*key).to_string(),
            command: (*command).to_string(),
            when: when.map(str::to_string),
        })
        .collect()
}

/// Parse `mod+shift+g`. `mod_is_command` expands `mod` to Command, otherwise
/// to Control. A chord with no key is `None`.
pub fn parse_chord(raw: &str, mod_is_command: bool) -> Option<Chord> {
    let parts: Vec<&str> = raw
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut command = false;
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None;
    for (index, part) in parts.iter().enumerate() {
        let token = part.to_ascii_lowercase();
        let last = index + 1 == parts.len();
        let modifier = match token.as_str() {
            "mod" => {
                if mod_is_command {
                    command = true;
                } else {
                    ctrl = true;
                }
                true
            }
            "cmd" | "meta" | "command" => {
                command = true;
                true
            }
            "ctrl" | "control" => {
                ctrl = true;
                true
            }
            "alt" | "option" => {
                alt = true;
                true
            }
            "shift" => {
                shift = true;
                true
            }
            _ => false,
        };
        if modifier {
            if last {
                return None;
            }
            continue;
        }
        if !last || key.is_some() {
            return None;
        }
        key = Some(canonical_key(&token));
    }
    key.map(|key| Chord {
        key,
        command,
        ctrl,
        alt,
        shift,
    })
}

/// Key name for a DOM or GPUI event. Letter codes win over the character,
/// so Option on macOS does not rename the binding.
pub fn event_key(code: &str, key: &str) -> String {
    if let Some(letter) = code.strip_prefix("Key") {
        if letter.len() == 1 && letter.chars().all(|c| c.is_ascii_alphabetic()) {
            return letter.to_ascii_lowercase();
        }
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        if digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) {
            return digit.to_string();
        }
    }
    match code {
        "Space" => return "space".to_string(),
        "Slash" => return "/".to_string(),
        "BracketLeft" => return "[".to_string(),
        "BracketRight" => return "]".to_string(),
        "Escape" => return "escape".to_string(),
        "Enter" => return "enter".to_string(),
        "ArrowUp" => return "up".to_string(),
        "ArrowDown" => return "down".to_string(),
        "ArrowLeft" => return "left".to_string(),
        "ArrowRight" => return "right".to_string(),
        "Backquote" => return "`".to_string(),
        "Minus" => return "-".to_string(),
        "Equal" => return "=".to_string(),
        "Backslash" => return "\\".to_string(),
        "Semicolon" => return ";".to_string(),
        "Quote" => return "'".to_string(),
        "Comma" => return ",".to_string(),
        "Period" => return ".".to_string(),
        _ => {}
    }
    canonical_key(key)
}

pub fn chord_from_event(
    code: &str,
    key: &str,
    command: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> Chord {
    Chord {
        key: event_key(code, key),
        command,
        ctrl,
        alt,
        shift,
    }
}

/// True when `when` is empty or a well-formed expression.
pub fn when_syntax_ok(when: &str) -> bool {
    let when = when.trim();
    if when.is_empty() {
        return true;
    }
    parse_when(when).is_some()
}

/// Rules to run. Missing storage gets the defaults. Saved rules stay, and
/// any default command they do not mention is appended.
pub fn stored_keybindings(saved: Option<Vec<Keybinding>>) -> Vec<Keybinding> {
    match saved {
        Some(saved) => fill_new_defaults(&saved, &default_keybindings()),
        None => default_keybindings(),
    }
}

/// GPUI spelling of a rule key. `mod` becomes `cmd`.
pub fn gpui_shortcut(rule_key: &str) -> Option<String> {
    let parts: Vec<&str> = rule_key
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let mut cmd = false;
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None;
    for (index, part) in parts.iter().enumerate() {
        let token = part.to_ascii_lowercase();
        let last = index + 1 == parts.len();
        let modifier = match token.as_str() {
            "mod" | "cmd" | "meta" | "command" => {
                cmd = true;
                true
            }
            "ctrl" | "control" => {
                ctrl = true;
                true
            }
            "alt" | "option" => {
                alt = true;
                true
            }
            "shift" => {
                shift = true;
                true
            }
            _ => false,
        };
        if modifier {
            if last {
                return None;
            }
            continue;
        }
        if !last {
            return None;
        }
        key = Some(canonical_key(&token));
    }
    let key = key?;
    let mut out = String::new();
    for (enabled, name) in [(cmd, "cmd"), (ctrl, "ctrl"), (alt, "alt"), (shift, "shift")] {
        if enabled {
            if !out.is_empty() {
                out.push('-');
            }
            out.push_str(name);
        }
    }
    if !out.is_empty() {
        out.push('-');
    }
    out.push_str(&key);
    Some(out)
}

/// Keep `saved` in order. Append each default whose command id is absent.
pub fn fill_new_defaults(saved: &[Keybinding], defaults: &[Keybinding]) -> Vec<Keybinding> {
    let mut out = saved.to_vec();
    for rule in defaults {
        let present = out.iter().any(|saved| saved.command == rule.command);
        if !present {
            out.push(rule.clone());
        }
    }
    out
}

/// True when some rule's key is this chord, even if its `when` fails.
/// An empty or unparsable key does not claim the chord it used to use.
pub fn chord_claimed(rules: &[Keybinding], pressed: &Chord, mod_is_command: bool) -> bool {
    rules
        .iter()
        .any(|rule| parse_chord(&rule.key, mod_is_command).is_some_and(|chord| chord == *pressed))
}

/// Command id of the last matching rule. An unparsable key or `when` is skipped.
pub fn resolve<'a>(
    rules: &'a [Keybinding],
    pressed: &Chord,
    ctx: &KeyContext,
    mod_is_command: bool,
) -> Option<&'a str> {
    let mut found = None;
    for rule in rules {
        if rule.command.is_empty() {
            continue;
        }
        let Some(chord) = parse_chord(&rule.key, mod_is_command) else {
            continue;
        };
        if &chord != pressed {
            continue;
        }
        if !when_holds(rule.when.as_deref(), ctx) {
            continue;
        }
        found = Some(rule.command.as_str());
    }
    found
}

fn when_holds(when: Option<&str>, ctx: &KeyContext) -> bool {
    let Some(when) = when.map(str::trim).filter(|text| !text.is_empty()) else {
        return true;
    };
    parse_when(when).is_some_and(|expr| expr.eval(*ctx))
}

fn canonical_key(token: &str) -> String {
    match token.to_ascii_lowercase().as_str() {
        "slash" => "/".to_string(),
        "bracketleft" | "bracket-left" => "[".to_string(),
        "bracketright" | "bracket-right" => "]".to_string(),
        "esc" | "escape" => "escape".to_string(),
        "return" | "enter" => "enter".to_string(),
        "space" | " " => "space".to_string(),
        "arrowup" | "up" => "up".to_string(),
        "arrowdown" | "down" => "down".to_string(),
        "arrowleft" | "left" => "left".to_string(),
        "arrowright" | "right" => "right".to_string(),
        other => other.to_string(),
    }
}

enum Expr {
    Flag(String),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

impl Expr {
    fn eval(&self, ctx: KeyContext) -> bool {
        match self {
            Self::Flag(name) => ctx.flag(name),
            Self::Not(inner) => !inner.eval(ctx),
            Self::And(left, right) => left.eval(ctx) && right.eval(ctx),
            Self::Or(left, right) => left.eval(ctx) || right.eval(ctx),
        }
    }
}

struct Parser<'a> {
    input: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn skip(&mut self) {
        let extra: usize = self
            .rest()
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(char::len_utf8)
            .sum();
        self.index += extra;
    }

    fn rest(&self) -> &'a str {
        &self.input[self.index..]
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip();
        if self.rest().starts_with(token) {
            self.index += token.len();
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> Option<Expr> {
        let mut left = self.parse_and()?;
        while self.eat("||") {
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_and(&mut self) -> Option<Expr> {
        let mut left = self.parse_unary()?;
        while self.eat("&&") {
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        if self.eat("!") {
            let inner = self.parse_unary()?;
            return Some(Expr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        if self.eat("(") {
            let inner = self.parse_or()?;
            if !self.eat(")") {
                return None;
            }
            return Some(inner);
        }
        self.skip();
        let rest = self.rest();
        let length = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .count();
        if length == 0 {
            return None;
        }
        let start = rest.as_bytes()[0];
        if !start.is_ascii_alphabetic() && start != b'_' {
            return None;
        }
        let name = rest[..length].to_string();
        self.index += length;
        Some(Expr::Flag(name))
    }
}

fn parse_when(input: &str) -> Option<Expr> {
    let mut parser = Parser { input, index: 0 };
    let expr = parser.parse_or()?;
    parser.skip();
    if parser.index == input.len() {
        Some(expr)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(raw: &str, mod_is_command: bool) -> Chord {
        parse_chord(raw, mod_is_command).expect("chord")
    }

    #[test]
    fn mod_is_command_on_macos_and_control_elsewhere() {
        let mac = parse_chord("mod+shift+g", true).unwrap();
        assert!(mac.command && mac.shift && !mac.ctrl && mac.key == "g");
        let other = parse_chord("mod+shift+g", false).unwrap();
        assert!(other.ctrl && other.shift && !other.command);
        let command = parse_chord("cmd+k", false).unwrap();
        assert!(command.command && !command.ctrl && command.key == "k");
        assert!(parse_chord("mod", true).is_none());
        assert!(parse_chord("mod++k", true).is_some());
    }

    #[test]
    fn event_codes_use_the_physical_key() {
        assert_eq!(event_key("KeyG", "©"), "g");
        assert_eq!(event_key("Digit1", "1"), "1");
        assert_eq!(event_key("Space", " "), "space");
        assert_eq!(event_key("Slash", "/"), "/");
        assert_eq!(event_key("BracketLeft", "["), "[");
        assert_eq!(event_key("Escape", "Escape"), "escape");
        assert_eq!(event_key("ArrowUp", "ArrowUp"), "up");
    }

    #[test]
    fn the_last_matching_rule_wins() {
        let rules = vec![
            Keybinding {
                key: "mod+k".into(),
                command: "first".into(),
                when: None,
            },
            Keybinding {
                key: "mod+k".into(),
                command: "second".into(),
                when: Some("terminalFocus".into()),
            },
            Keybinding {
                key: "mod+k".into(),
                command: "third".into(),
                when: Some("!terminalFocus".into()),
            },
        ];
        let pressed = press("mod+k", true);
        let idle = KeyContext::default();
        assert_eq!(resolve(&rules, &pressed, &idle, true), Some("third"));
        let typing = KeyContext {
            terminal_focus: true,
            ..KeyContext::default()
        };
        assert_eq!(resolve(&rules, &pressed, &typing, true), Some("second"));
    }

    #[test]
    fn a_broken_when_clause_is_skipped() {
        let rules = vec![
            Keybinding {
                key: "mod+f".into(),
                command: "broken".into(),
                when: Some("terminalFocus &&".into()),
            },
            Keybinding {
                key: "mod+f".into(),
                command: "chat.find".into(),
                when: Some("!terminalFocus && !previewFocus".into()),
            },
        ];
        let pressed = press("mod+f", true);
        assert_eq!(
            resolve(&rules, &pressed, &KeyContext::default(), true),
            Some("chat.find")
        );
        assert!(!when_syntax_ok("terminalFocus &&"));
        assert!(when_syntax_ok("a || b && c"));
        let ctx = KeyContext {
            terminal_focus: true,
            ..KeyContext::default()
        };
        assert!(!when_holds(Some("a || b && c"), &ctx));
        assert!(when_holds(Some("terminalFocus || madeUp"), &ctx));
        assert!(!when_holds(Some("madeUp"), &KeyContext::default()));
    }

    #[test]
    fn and_binds_tighter_than_or_and_parentheses_group() {
        let ctx = KeyContext {
            terminal_open: true,
            ..KeyContext::default()
        };
        // `&&` binds tighter, so the open terminal makes this true.
        // The other grouping would require preview focus and be false.
        assert!(when_holds(
            Some("terminalOpen || terminalFocus && previewFocus"),
            &ctx
        ));
        assert!(!when_holds(
            Some("(terminalOpen || terminalFocus) && previewFocus"),
            &ctx
        ));
        assert!(when_holds(Some("!!terminalOpen"), &ctx));
    }

    #[test]
    fn new_defaults_keep_a_customized_command_and_append_a_new_one() {
        let defaults = default_keybindings();
        assert!(defaults.iter().all(|rule| {
            rule.key.is_empty()
                && matches!(rule.command.as_str(), "pull.copyLink" | "pull.copyNumber")
                || parse_chord(&rule.key, true).is_some()
        }));
        assert_eq!(fill_new_defaults(&defaults, &defaults), defaults);

        let mut saved = defaults.clone();
        let palette = saved
            .iter_mut()
            .find(|rule| rule.command == "command.palette")
            .unwrap();
        palette.key = "mod+shift+k".into();
        let extra = Keybinding {
            key: "mod+shift+h".into(),
            command: "host.cycle".into(),
            when: None,
        };
        let mut with_new = defaults.clone();
        with_new.push(extra);
        let filled = fill_new_defaults(&saved, &with_new);
        assert_eq!(
            filled
                .iter()
                .find(|rule| rule.command == "command.palette")
                .map(|rule| rule.key.as_str()),
            Some("mod+shift+k")
        );
        assert_eq!(
            filled.last().map(|rule| rule.command.as_str()),
            Some("host.cycle")
        );
        assert_eq!(
            filled
                .iter()
                .filter(|rule| rule.command == "chat.new")
                .count(),
            3
        );
        assert_eq!(stored_keybindings(None).len(), defaults.len());
        assert_eq!(gpui_shortcut("mod+shift+g").as_deref(), Some("cmd-shift-g"));
        assert_eq!(gpui_shortcut("alt+w").as_deref(), Some("alt-w"));
        assert_eq!(gpui_shortcut("/").as_deref(), Some("/"));
        let palette = press("mod+k", true);
        assert!(chord_claimed(&defaults, &palette, true));
        let mut cleared = defaults;
        cleared
            .iter_mut()
            .find(|rule| rule.command == "command.palette")
            .expect("palette")
            .key
            .clear();
        assert!(!chord_claimed(&cleared, &palette, true));
    }

    #[test]
    fn model_picker_hides_the_thread_jumps() {
        let rules = default_keybindings();
        let pressed = chord_from_event("Digit1", "1", true, false, false, false);
        assert_eq!(
            resolve(&rules, &pressed, &KeyContext::default(), true),
            Some("thread.jump.1")
        );
        let picking = KeyContext {
            model_picker_open: true,
            ..KeyContext::default()
        };
        assert_eq!(resolve(&rules, &pressed, &picking, true), None);
    }
}
