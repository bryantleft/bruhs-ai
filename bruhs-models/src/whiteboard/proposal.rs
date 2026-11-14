//! Agent edits are immutable previews, tied to the board revision and caller.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardRequester {
    pub thread_id: String,
    pub turn_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardProposal {
    pub id: String,
    pub board_id: String,
    pub project_path: String,
    pub requester: BoardRequester,
    pub base_revision: u64,
    pub base_digest: String,
    pub description: String,
    pub operations: Vec<BoardOperation>,
    pub digest: String,
}
fn document_digest(board: &BoardDocument) -> BoardResult<String> {
    serde_json::to_vec(board)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|e| BoardError::Invalid(e.to_string()))
}
impl BoardProposal {
    pub fn new(
        board: &BoardDocument,
        requester: BoardRequester,
        description: String,
        operations: Vec<BoardOperation>,
    ) -> BoardResult<Self> {
        if !board.agent_read {
            return Err(BoardError::Invalid("This board is private. Its owner must enable Share with agents before an agent can read or propose changes.".into()));
        }
        validate_id(&requester.thread_id)?;
        validate_id(&requester.turn_id)?;
        if description.trim().is_empty() || operations.is_empty() {
            return Err(BoardError::Invalid(
                "Describe the proposed edit and include at least one operation.".into(),
            ));
        }
        if operations
            .iter()
            .any(|op| matches!(op, BoardOperation::AgentRead { .. }))
        {
            return Err(BoardError::Invalid(
                "Agents cannot change their own board sharing permission.".into(),
            ));
        }
        let mut proposal = Self {
            id: uuid::Uuid::new_v4().to_string(),
            board_id: board.id.clone(),
            project_path: board.project_path.clone(),
            requester,
            base_revision: board.revision,
            base_digest: document_digest(board)?,
            description,
            operations,
            digest: String::new(),
        };
        proposal.digest = proposal.hash()?;
        proposal.preview(board, board.updated_at_ms)?;
        Ok(proposal)
    }
    fn hash(&self) -> BoardResult<String> {
        let bytes = serde_json::to_vec(&(
            &self.id,
            &self.board_id,
            &self.project_path,
            &self.requester,
            self.base_revision,
            &self.base_digest,
            &self.description,
            &self.operations,
        ))
        .map_err(|e| BoardError::Invalid(e.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn validate_for(&self, board: &BoardDocument) -> BoardResult<()> {
        if self.hash()? != self.digest {
            return Err(BoardError::Invalid(
                "The proposed edit changed after review. Request a new preview.".into(),
            ));
        }
        if self.board_id != board.id
            || self.project_path != board.project_path
            || self.base_revision != board.revision
            || self.base_digest != document_digest(board)?
        {
            return Err(BoardError::Conflict);
        }
        if !board.agent_read {
            return Err(BoardError::Invalid(
                "Agent sharing was disabled. This proposal can no longer be accepted.".into(),
            ));
        }
        if self
            .operations
            .iter()
            .any(|op| matches!(op, BoardOperation::AgentRead { .. }))
        {
            return Err(BoardError::Invalid(
                "A proposal cannot grant board access.".into(),
            ));
        }
        Ok(())
    }
    pub fn preview(&self, board: &BoardDocument, now_ms: i64) -> BoardResult<BoardDocument> {
        self.validate_for(board)?;
        let mut preview = BoardEditor::new(board.clone())?;
        preview.edit(self.operations.clone(), now_ms)?;
        Ok(preview.document().clone())
    }
    /// Called only by a visible owner's Accept control, never an agent endpoint.
    pub fn accept(&self, editor: &mut BoardEditor, now_ms: i64) -> BoardResult<()> {
        self.validate_for(editor.document())?;
        editor.edit(self.operations.clone(), now_ms)
    }
    pub fn changes(&self, board: &BoardDocument) -> BoardResult<Vec<ProposalChange>> {
        let after = self.preview(board, board.updated_at_ms)?;
        let ids = board
            .shapes
            .keys()
            .chain(after.shapes.keys())
            .collect::<std::collections::BTreeSet<_>>();
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                let before = board.shapes.get(id);
                let next = after.shapes.get(id);
                if before == next {
                    None
                } else {
                    Some(ProposalChange {
                        id: id.clone(),
                        before: before.cloned(),
                        after: next.cloned(),
                    })
                }
            })
            .collect())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalChange {
    pub id: String,
    pub before: Option<Shape>,
    pub after: Option<Shape>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Accepted { revision: u64 },
    Rejected,
    Stale,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_require_grant_and_tampered_stale_or_replayed_edits_cannot_apply() {
        let mut board = benchmark_scene(1000).unwrap();
        let requester = BoardRequester {
            thread_id: "thread-a".into(),
            turn_id: "turn-a".into(),
        };
        let ops = vec![BoardOperation::Title {
            title: "Proposed".into(),
        }];
        assert!(
            BoardProposal::new(&board, requester.clone(), "Rename".into(), ops.clone()).is_err()
        );
        board.agent_read = true;
        let proposal = BoardProposal::new(&board, requester, "Rename".into(), ops).unwrap();
        let mut editor = BoardEditor::new(board).unwrap();
        assert_ne!(editor.document().title, "Proposed");
        assert_eq!(
            proposal.preview(editor.document(), 1).unwrap().title,
            "Proposed"
        );
        let mut altered = proposal.clone();
        altered.description = "Changed".into();
        assert!(altered.accept(&mut editor, 1).is_err());
        proposal.accept(&mut editor, 1).unwrap();
        assert!(proposal.accept(&mut editor, 2).is_err());
        assert_eq!(editor.document().title, "Proposed");
    }
}
