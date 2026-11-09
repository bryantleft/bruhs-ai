//! The "Open in editor" table and the argument shapes each launch style
//! expects. Moved from
//! bruhs-core `editor.rs`, which keeps the PATH lookup and the spawn; the web
//! picker's `EDITOR_CHOICES` duplicated the ids, labels and families.

/// Section an editor is listed under in pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorFamily {
    VsCode,
    JetBrains,
    Other,
}

impl EditorFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code family",
            Self::JetBrains => "JetBrains",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStyle {
    /// `cmd path[:line]` (Zed).
    DirectPath,
    /// `cmd -g path[:line]` (VS Code and forks).
    Goto,
    /// `cmd --line N path` (JetBrains).
    LineColumn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorDefinition {
    pub id: &'static str,
    pub label: &'static str,
    /// CLI names to look for on `PATH`, in order of preference.
    pub commands: &'static [&'static str],
    /// Arguments that always precede the path (Kiro's `ide`).
    pub base_args: &'static [&'static str],
    pub launch_style: LaunchStyle,
    /// macOS application name for `open -a` when no CLI is installed.
    pub mac_app: Option<&'static str>,
    /// Picker section.
    pub family: EditorFamily,
}

pub const EDITORS: &[EditorDefinition] = &[
    EditorDefinition {
        id: "cursor",
        label: "Cursor",
        commands: &["cursor"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Cursor"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "vscode",
        label: "VS Code",
        commands: &["code"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Visual Studio Code"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "vscode-insiders",
        label: "VS Code Insiders",
        commands: &["code-insiders"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Visual Studio Code - Insiders"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "vscodium",
        label: "VSCodium",
        commands: &["codium"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("VSCodium"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "windsurf",
        label: "Windsurf",
        commands: &["windsurf"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Windsurf"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "trae",
        label: "Trae",
        commands: &["trae"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Trae"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "kiro",
        label: "Kiro",
        commands: &["kiro"],
        base_args: &["ide"],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Kiro"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "antigravity",
        label: "Antigravity",
        commands: &["agy"],
        base_args: &[],
        launch_style: LaunchStyle::Goto,
        mac_app: Some("Antigravity"),
        family: EditorFamily::VsCode,
    },
    EditorDefinition {
        id: "zed",
        label: "Zed",
        commands: &["zed", "zeditor"],
        base_args: &[],
        launch_style: LaunchStyle::DirectPath,
        mac_app: Some("Zed"),
        family: EditorFamily::Other,
    },
    EditorDefinition {
        id: "idea",
        label: "IntelliJ IDEA",
        commands: &["idea"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("IntelliJ IDEA"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "pycharm",
        label: "PyCharm",
        commands: &["pycharm"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("PyCharm"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "webstorm",
        label: "WebStorm",
        commands: &["webstorm"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("WebStorm"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "goland",
        label: "GoLand",
        commands: &["goland"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("GoLand"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "rider",
        label: "Rider",
        commands: &["rider"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("Rider"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "clion",
        label: "CLion",
        commands: &["clion"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("CLion"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "phpstorm",
        label: "PhpStorm",
        commands: &["phpstorm"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("PhpStorm"),
        family: EditorFamily::JetBrains,
    },
    EditorDefinition {
        id: "rubymine",
        label: "RubyMine",
        commands: &["rubymine"],
        base_args: &[],
        launch_style: LaunchStyle::LineColumn,
        mac_app: Some("RubyMine"),
        family: EditorFamily::JetBrains,
    },
];

pub const DEFAULT_EDITOR_ID: &str = "vscode";

pub fn editor_by_id(id: &str) -> Option<&'static EditorDefinition> {
    EDITORS.iter().find(|e| e.id == id)
}

/// Command line for `command` (one of `editor.commands`) opening `path`,
/// optionally at `line`.
pub fn editor_args(
    editor: &EditorDefinition,
    command: &str,
    path: &str,
    line: Option<u32>,
) -> (String, Vec<String>) {
    let mut args: Vec<String> = editor.base_args.iter().map(|a| a.to_string()).collect();
    match editor.launch_style {
        LaunchStyle::Goto => {
            match line {
                Some(n) => {
                    args.push("-g".to_string());
                    args.push(format!("{path}:{n}"));
                }
                None => args.push(path.to_string()),
            };
        }
        LaunchStyle::DirectPath => match line {
            Some(n) => args.push(format!("{path}:{n}")),
            None => args.push(path.to_string()),
        },
        LaunchStyle::LineColumn => {
            if let Some(n) = line {
                args.push("--line".to_string());
                args.push(n.to_string());
            }
            args.push(path.to_string());
        }
    }
    (command.to_string(), args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(id: &str) -> &'static EditorDefinition {
        editor_by_id(id).expect("editor exists")
    }

    #[test]
    fn goto_style_appends_path_line() {
        assert_eq!(
            editor_args(ed("vscode"), "code", "/p/a.rs", Some(12)),
            (
                "code".to_string(),
                vec!["-g".to_string(), "/p/a.rs:12".to_string()]
            )
        );
        assert_eq!(
            editor_args(ed("cursor"), "cursor", "/p", None),
            ("cursor".to_string(), vec!["/p".to_string()])
        );
    }

    #[test]
    fn line_column_style_uses_line_flag() {
        assert_eq!(
            editor_args(ed("idea"), "idea", "/p/a.rs", Some(12)),
            (
                "idea".to_string(),
                vec![
                    "--line".to_string(),
                    "12".to_string(),
                    "/p/a.rs".to_string()
                ]
            )
        );
        assert_eq!(
            editor_args(ed("idea"), "idea", "/p", None),
            ("idea".to_string(), vec!["/p".to_string()])
        );
    }

    #[test]
    fn direct_path_without_line_is_bare() {
        assert_eq!(
            editor_args(ed("zed"), "zed", "/p", None),
            ("zed".to_string(), vec!["/p".to_string()])
        );
        assert_eq!(
            editor_args(ed("zed"), "zeditor", "/p/a.rs", Some(3)),
            ("zeditor".to_string(), vec!["/p/a.rs:3".to_string()])
        );
    }

    #[test]
    fn kiro_keeps_base_args() {
        assert_eq!(
            editor_args(ed("kiro"), "kiro", "/p/a.rs", Some(1)),
            (
                "kiro".to_string(),
                vec!["ide".to_string(), "-g".to_string(), "/p/a.rs:1".to_string()]
            )
        );
    }

    #[test]
    fn table_is_unique_and_matches_the_web_picker() {
        assert!(editor_by_id("emacs-of-doom").is_none());
        assert!(editor_by_id(DEFAULT_EDITOR_ID).is_some());
        // Web `state::EDITOR_CHOICES`, in order.
        let choices: Vec<(&str, &str, &str)> = EDITORS
            .iter()
            .map(|e| (e.id, e.label, e.family.label()))
            .collect();
        assert_eq!(
            choices,
            vec![
                ("cursor", "Cursor", "VS Code family"),
                ("vscode", "VS Code", "VS Code family"),
                ("vscode-insiders", "VS Code Insiders", "VS Code family"),
                ("vscodium", "VSCodium", "VS Code family"),
                ("windsurf", "Windsurf", "VS Code family"),
                ("trae", "Trae", "VS Code family"),
                ("kiro", "Kiro", "VS Code family"),
                ("antigravity", "Antigravity", "VS Code family"),
                ("zed", "Zed", "Other"),
                ("idea", "IntelliJ IDEA", "JetBrains"),
                ("pycharm", "PyCharm", "JetBrains"),
                ("webstorm", "WebStorm", "JetBrains"),
                ("goland", "GoLand", "JetBrains"),
                ("rider", "Rider", "JetBrains"),
                ("clion", "CLion", "JetBrains"),
                ("phpstorm", "PhpStorm", "JetBrains"),
                ("rubymine", "RubyMine", "JetBrains"),
            ]
        );
        let mut ids: Vec<&str> = EDITORS.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), EDITORS.len(), "editor ids are unique");
    }
}
