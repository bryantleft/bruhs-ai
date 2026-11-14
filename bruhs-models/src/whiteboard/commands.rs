//! One command catalog for both command palettes.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardAction {
    Undo,
    Redo,
    SelectAll,
    Delete,
    Group,
    Ungroup,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignTop,
    AlignMiddle,
    AlignBottom,
    DistributeHorizontal,
    DistributeVertical,
    Copy,
    Paste,
    EditText,
    ConnectNext,
    Rename,
    Link,
    Import,
    ImportMermaid,
    ExportMermaid,
    ExportJson,
    ExportSvg,
    ExportPng,
    Fit,
    ResetZoom,
    ToggleSnap,
    Share,
    Save,
    SaveCopy,
    RefreshSources,
    AllBoards,
    NewBoard,
}
impl BoardAction {
    pub const ALL: [Self; 35] = [
        Self::Undo,
        Self::Redo,
        Self::SelectAll,
        Self::Delete,
        Self::Group,
        Self::Ungroup,
        Self::AlignLeft,
        Self::AlignCenter,
        Self::AlignRight,
        Self::AlignTop,
        Self::AlignMiddle,
        Self::AlignBottom,
        Self::DistributeHorizontal,
        Self::DistributeVertical,
        Self::Copy,
        Self::Paste,
        Self::EditText,
        Self::ConnectNext,
        Self::Rename,
        Self::Link,
        Self::Import,
        Self::ImportMermaid,
        Self::ExportMermaid,
        Self::ExportJson,
        Self::ExportSvg,
        Self::ExportPng,
        Self::Fit,
        Self::ResetZoom,
        Self::ToggleSnap,
        Self::Share,
        Self::Save,
        Self::SaveCopy,
        Self::RefreshSources,
        Self::AllBoards,
        Self::NewBoard,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::SelectAll => "Select all",
            Self::Delete => "Delete selection",
            Self::Group => "Group",
            Self::Ungroup => "Ungroup",
            Self::AlignLeft => "Align left",
            Self::AlignCenter => "Align center",
            Self::AlignRight => "Align right",
            Self::AlignTop => "Align top",
            Self::AlignMiddle => "Align middle",
            Self::AlignBottom => "Align bottom",
            Self::DistributeHorizontal => "Distribute horizontally",
            Self::DistributeVertical => "Distribute vertically",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::EditText => "Edit text",
            Self::ConnectNext => "Connect next node",
            Self::Rename => "Rename board",
            Self::Link => "Link selection",
            Self::Import => "Import board",
            Self::ImportMermaid => "Import Mermaid",
            Self::ExportMermaid => "Export Mermaid",
            Self::ExportJson => "Export portable JSON",
            Self::ExportSvg => "Export SVG",
            Self::ExportPng => "Export PNG",
            Self::Fit => "Fit board",
            Self::ResetZoom => "Reset zoom",
            Self::ToggleSnap => "Toggle snapping",
            Self::Share => "Toggle agent sharing",
            Self::Save => "Retry save",
            Self::SaveCopy => "Save a copy",
            Self::RefreshSources => "Check source drift",
            Self::AllBoards => "All project boards",
            Self::NewBoard => "New board",
        }
    }
    /// Returns false only for an action requiring platform UI, storage or clipboard.
    pub fn apply(self, input: &mut BoardInput, now: i64) -> BoardResult<bool> {
        let editor = &mut input.editor;
        match self {
            Self::Undo => {
                editor.undo(now)?;
            }
            Self::Redo => {
                editor.redo(now)?;
            }
            Self::SelectAll => editor.select_all(),
            Self::Delete => editor.delete_selection(now)?,
            Self::Group => editor.group(now)?,
            Self::Ungroup => editor.ungroup(now)?,
            Self::AlignLeft => editor.align(Alignment::Left, now)?,
            Self::AlignCenter => editor.align(Alignment::Center, now)?,
            Self::AlignRight => editor.align(Alignment::Right, now)?,
            Self::AlignTop => editor.align(Alignment::Top, now)?,
            Self::AlignMiddle => editor.align(Alignment::Middle, now)?,
            Self::AlignBottom => editor.align(Alignment::Bottom, now)?,
            Self::DistributeHorizontal => editor.align(Alignment::DistributeHorizontal, now)?,
            Self::DistributeVertical => editor.align(Alignment::DistributeVertical, now)?,
            Self::ResetZoom => editor.camera.zoom = 1.0,
            Self::ToggleSnap => input.snapping = !input.snapping,
            Self::Share => editor.edit(
                vec![BoardOperation::AgentRead {
                    allowed: !editor.document().agent_read,
                }],
                now,
            )?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardCommand {
    Tool(BoardTool),
    Action(BoardAction),
}
impl BoardCommand {
    pub fn all() -> impl Iterator<Item = Self> {
        BoardTool::ALL
            .into_iter()
            .map(Self::Tool)
            .chain(BoardAction::ALL.into_iter().map(Self::Action))
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool.label(),
            Self::Action(action) => action.label(),
        }
    }
}
