//! Bounded requests travel through the existing turn broker; only owner UIs apply them.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardRequest {
    List,
    Read {
        board_id: String,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        limit: Option<u32>,
    },
    Propose {
        board_id: String,
        base_revision: u64,
        description: String,
        edits_json: String,
    },
    Status {
        proposal_id: String,
    },
}
impl BoardRequest {
    pub const fn writes(&self) -> bool {
        matches!(self, Self::Propose { .. })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let id = match self {
            Self::List => return Ok(()),
            Self::Read { board_id, .. } | Self::Propose { board_id, .. } => board_id,
            Self::Status { proposal_id } => proposal_id,
        };
        if uuid::Uuid::parse_str(id).is_err() {
            return Err("Use the complete board or proposal UUID returned by the app.");
        }
        if let Self::Propose {
            description,
            edits_json,
            ..
        } = self
        {
            if description.trim().is_empty()
                || description.len() > 4096
                || edits_json.len() > 128 * 1024
            {
                return Err(
                    "Describe the proposal within 4 KiB and keep edits_json within the broker's 128 KiB edit budget.",
                );
            }
        }
        if let Self::Read {
            cursor: Some(cursor),
            ..
        } = self
        {
            validate_id(cursor).map_err(|_| "The board cursor is invalid.")?;
        }
        Ok(())
    }
}
pub fn board_tool_reply(value: Value) -> crate::thread_link::ThreadReply {
    let reply = crate::thread_link::ThreadReply::ok(value.to_string());
    if serde_json::to_vec(&reply).map_or(true, |bytes| {
        bytes.len() > crate::thread_link::BROKER_FRAME_BYTES
    }) {
        crate::thread_link::ThreadReply::refused(
            "This board reply exceeds the turn broker's frame budget. Reduce the page limit or shorten the board metadata in the owner editor.",
        )
    } else {
        reply
    }
}
pub fn shared_board<'a>(board: &'a BoardDocument, project: &str) -> BoardResult<&'a BoardDocument> {
    if board.project_path != project {
        return Err(BoardError::Invalid(
            "This board is outside the calling thread's working folder.".into(),
        ));
    }
    if !board.agent_read {
        return Err(BoardError::Invalid(
            "This board is private. Its owner must enable Share with agents in the board editor."
                .into(),
        ));
    }
    Ok(board)
}
pub fn board_page(
    board: &BoardDocument,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> BoardResult<Value> {
    let max = usize::try_from(limit.unwrap_or(25).clamp(1, 100)).unwrap_or(25);
    let mut shapes = Vec::new();
    let mut bytes = 0;
    let mut next_cursor = None;
    let mut last = None;
    for shape in board
        .shapes
        .values()
        .filter(|s| cursor.is_none_or(|c| s.id.as_str() > c))
    {
        let value = serde_json::to_value(shape).map_err(|e| BoardError::Invalid(e.to_string()))?;
        let len = serde_json::to_vec(&value)
            .map_err(|e| BoardError::Invalid(e.to_string()))?
            .len();
        if len > 128 * 1024 && shapes.is_empty() {
            return Err(BoardError::Invalid(
                "This shape exceeds the broker page budget. Reduce its text in the editor.".into(),
            ));
        }
        if shapes.len() == max || bytes + len > 128 * 1024 {
            next_cursor = last;
            break;
        }
        bytes += len;
        last = Some(shape.id.clone());
        shapes.push(value);
    }
    if shapes.is_empty() && next_cursor.is_some() {
        return Err(BoardError::Invalid(
            "This shape exceeds the broker page budget. Reduce its text in the editor.".into(),
        ));
    }
    let value = json!({"id":board.id,"project_path":board.project_path,"title":board.title,"revision":board.revision,"background":board.background,"sources":board.sources,"assets":board.assets,"shapes":shapes,"next_cursor":next_cursor,"shape_count":board.shapes.len(),"authority":"The UI owns this local board. Edits require an immutable proposal and owner acceptance."});
    if serde_json::to_vec(&value)
        .map_err(|e| BoardError::Invalid(e.to_string()))?
        .len()
        > 192 * 1024
    {
        return Err(BoardError::Invalid("Board metadata exceeds the broker reply budget. Use the owner editor to reduce source or asset metadata.".into()));
    }
    Ok(value)
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Edits {
    Operations {
        operations: Vec<BoardOperation>,
    },
    Mermaid {
        mermaid: String,
        #[serde(default)]
        source_path: Option<String>,
    },
}
pub fn proposal_operations(
    board: &BoardDocument,
    edits_json: &str,
    now: i64,
) -> BoardResult<Vec<BoardOperation>> {
    let edits: Edits = serde_json::from_str(edits_json).map_err(|e| {
        BoardError::Invalid(format!(
            "Use edits_json with operations:[...] or mermaid plus optional source_path: {e}"
        ))
    })?;
    match edits {
        Edits::Operations { operations } => Ok(operations),
        Edits::Mermaid {
            mermaid,
            source_path,
        } => {
            let mut imported = import_mermaid(
                &mermaid,
                board.project_path.clone(),
                "Agent diagram".into(),
                now,
            )?;
            if let Some(path) = source_path {
                validate_relative_path(&path)?;
                let id = uuid::Uuid::new_v4().to_string();
                imported.sources.insert(
                    id.clone(),
                    Source {
                        id: id.clone(),
                        path,
                        sha256: "0".repeat(64),
                        mermaid: Some(mermaid),
                        stale: false,
                    },
                );
                for shape in imported.shapes.values_mut() {
                    shape.source_id = Some(id.clone());
                }
            }
            let mut editor = BoardEditor::new(board.clone())?;
            editor.paste(&imported, Point::new(24.0, 24.0), now)?;
            board_diff(board, editor.document())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_cursor_is_bounded_and_private_boards_refuse_reads() {
        let mut board = benchmark_scene(1000).unwrap();
        let project = board.project_path.clone();
        assert!(shared_board(&board, &project).is_err());
        board.agent_read = true;
        assert!(shared_board(&board, "another").is_err());
        assert!(shared_board(&board, &project).is_ok());
        let first = board_page(&board, None, Some(3)).unwrap();
        assert_eq!(first["shapes"].as_array().unwrap().len(), 3);
        let second = board_page(&board, first["next_cursor"].as_str(), Some(3)).unwrap();
        assert_ne!(first["shapes"][0]["id"], second["shapes"][0]["id"]);
        assert!(proposal_operations(
            &board,
            r#"{"mermaid":"flowchart LR\n a-->b","source_path":"../secret"}"#,
            0
        )
        .is_err());
    }
}
