//! A text field's editing actions and their key bindings (in the "Input"
//! key context, which every field sets).

use gpui::{actions, App, KeyBinding};
use serde::Deserialize;

/// The key context every text field sets.
pub const CONTEXT: &str = "Input";

actions!(
    text_field,
    [
        ToggleBold,
        ToggleItalic,
        ToggleStrike,
        ToggleCode,
        Backspace,
        Delete,
        DeleteToBeginningOfLine,
        DeleteToEndOfLine,
        DeleteToPreviousWordStart,
        DeleteToNextWordEnd,
        Escape,
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        MoveHome,
        MoveEnd,
        MoveToStart,
        MoveToEnd,
        MoveToPreviousWord,
        MoveToNextWord,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectToStartOfLine,
        SelectToEndOfLine,
        SelectToStart,
        SelectToEnd,
        SelectToPreviousWordStart,
        SelectToNextWordEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        /// The system emoji and symbols picker.
        ShowCharacterPalette,
        MovePageUp,
        MovePageDown,
        /// Tab in a field. A field never traps it: unhandled, it moves focus.
        IndentInline,
        /// Shift-Tab in a field.
        OutdentInline,
    ]
);

/// Enter in a field. A multi-line field inserts a newline for a plain or
/// Shift Enter (unless it submits on Enter); every Enter reports
/// [`super::InputEvent::PressEnter`].
#[derive(gpui::Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = text_field, no_json)]
pub struct Enter {
    /// Cmd-Enter (Ctrl-Enter off macOS): always a submit.
    pub secondary: bool,
    /// Shift was held.
    pub shift: bool,
}

impl Enter {
    /// Whether `action` is a plain or Shift Enter (not the secondary one).
    pub fn is_primary(action: &dyn gpui::Action) -> bool {
        action.partial_eq(&Enter {
            secondary: false,
            shift: false,
        }) || action.partial_eq(&Enter {
            secondary: false,
            shift: true,
        })
    }
}

pub(super) fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    let mac = cfg!(target_os = "macos");
    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("shift-delete", Delete, ctx),
        KeyBinding::new("escape", Escape, ctx),
        KeyBinding::new("up", MoveUp, ctx),
        KeyBinding::new("down", MoveDown, ctx),
        KeyBinding::new("left", MoveLeft, ctx),
        KeyBinding::new("right", MoveRight, ctx),
        KeyBinding::new("home", MoveHome, ctx),
        KeyBinding::new("end", MoveEnd, ctx),
        KeyBinding::new("ctrl-home", MoveToStart, ctx),
        KeyBinding::new("ctrl-end", MoveToEnd, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("shift-home", SelectToStartOfLine, ctx),
        KeyBinding::new("shift-end", SelectToEndOfLine, ctx),
        KeyBinding::new("ctrl-shift-home", SelectToStart, ctx),
        KeyBinding::new("ctrl-shift-end", SelectToEnd, ctx),
        KeyBinding::new("tab", IndentInline, ctx),
        KeyBinding::new("shift-tab", OutdentInline, ctx),
        KeyBinding::new("pageup", MovePageUp, ctx),
        KeyBinding::new("pagedown", MovePageDown, ctx),
        KeyBinding::new(
            "enter",
            Enter {
                secondary: false,
                shift: false,
            },
            ctx,
        ),
        KeyBinding::new(
            "shift-enter",
            Enter {
                secondary: false,
                shift: true,
            },
            ctx,
        ),
        KeyBinding::new(
            if mac { "cmd-enter" } else { "ctrl-enter" },
            Enter {
                secondary: true,
                shift: false,
            },
            ctx,
        ),
    ];
    if mac {
        bindings.extend([
            KeyBinding::new("cmd-backspace", DeleteToBeginningOfLine, ctx),
            KeyBinding::new("cmd-delete", DeleteToEndOfLine, ctx),
            KeyBinding::new("alt-backspace", DeleteToPreviousWordStart, ctx),
            KeyBinding::new("alt-delete", DeleteToNextWordEnd, ctx),
            KeyBinding::new("cmd-left", MoveHome, ctx),
            KeyBinding::new("cmd-right", MoveEnd, ctx),
            KeyBinding::new("ctrl-a", MoveHome, ctx),
            KeyBinding::new("ctrl-e", MoveEnd, ctx),
            KeyBinding::new("cmd-up", MoveToStart, ctx),
            KeyBinding::new("cmd-down", MoveToEnd, ctx),
            KeyBinding::new("alt-left", MoveToPreviousWord, ctx),
            KeyBinding::new("alt-right", MoveToNextWord, ctx),
            KeyBinding::new("shift-cmd-left", SelectToStartOfLine, ctx),
            KeyBinding::new("shift-cmd-right", SelectToEndOfLine, ctx),
            KeyBinding::new("ctrl-shift-a", SelectToStartOfLine, ctx),
            KeyBinding::new("ctrl-shift-e", SelectToEndOfLine, ctx),
            KeyBinding::new("cmd-shift-up", SelectToStart, ctx),
            KeyBinding::new("cmd-shift-down", SelectToEnd, ctx),
            KeyBinding::new("alt-shift-left", SelectToPreviousWordStart, ctx),
            KeyBinding::new("alt-shift-right", SelectToNextWordEnd, ctx),
            KeyBinding::new("cmd-a", SelectAll, ctx),
            KeyBinding::new("cmd-c", Copy, ctx),
            KeyBinding::new("cmd-x", Cut, ctx),
            KeyBinding::new("cmd-v", Paste, ctx),
            KeyBinding::new("cmd-z", Undo, ctx),
            KeyBinding::new("cmd-shift-z", Redo, ctx),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-backspace", DeleteToPreviousWordStart, ctx),
            KeyBinding::new("ctrl-delete", DeleteToNextWordEnd, ctx),
            KeyBinding::new("ctrl-left", MoveToPreviousWord, ctx),
            KeyBinding::new("ctrl-right", MoveToNextWord, ctx),
            KeyBinding::new("ctrl-shift-left", SelectToPreviousWordStart, ctx),
            KeyBinding::new("ctrl-shift-right", SelectToNextWordEnd, ctx),
            KeyBinding::new("ctrl-a", SelectAll, ctx),
            KeyBinding::new("ctrl-c", Copy, ctx),
            KeyBinding::new("ctrl-x", Cut, ctx),
            KeyBinding::new("ctrl-v", Paste, ctx),
            KeyBinding::new("ctrl-z", Undo, ctx),
            KeyBinding::new("ctrl-y", Redo, ctx),
            KeyBinding::new("ctrl-shift-z", Redo, ctx),
        ]);
    }
    let rich = Some("RichComposer");
    bindings.extend([
        KeyBinding::new(if mac { "cmd-b" } else { "ctrl-b" }, ToggleBold, rich),
        KeyBinding::new(if mac { "cmd-i" } else { "ctrl-i" }, ToggleItalic, rich),
        KeyBinding::new(
            if mac { "cmd-shift-s" } else { "ctrl-shift-s" },
            ToggleStrike,
            rich,
        ),
        KeyBinding::new(if mac { "cmd-e" } else { "ctrl-e" }, ToggleCode, rich),
    ]);
    cx.bind_keys(bindings);
}

/// The formatting command owned by a focused rich composer.
pub fn composer_format_action(stroke: &gpui::Keystroke) -> Option<Box<dyn gpui::Action>> {
    let m = stroke.modifiers;
    let command = if cfg!(target_os = "macos") {
        m.platform && !m.control
    } else {
        m.control && !m.platform
    };
    if !command || m.alt || m.function {
        return None;
    }
    match (stroke.key.as_str(), m.shift) {
        ("b", false) => Some(Box::new(ToggleBold)),
        ("i", false) => Some(Box::new(ToggleItalic)),
        ("s", true) => Some(Box::new(ToggleStrike)),
        ("e", false) => Some(Box::new(ToggleCode)),
        _ => None,
    }
}
