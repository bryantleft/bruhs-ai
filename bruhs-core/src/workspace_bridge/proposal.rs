use bruhs_models::WorkflowOperation;
use serde::{Deserialize, Serialize};

/// One ordered review unit, not authorization to execute any of its steps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowProposal {
    pub title: String,
    pub operations: Vec<WorkflowOperation>,
}

impl WorkflowProposal {
    pub fn single(operation: WorkflowOperation) -> Self {
        Self {
            title: format!("MCP proposal — {}", operation.domain().label()),
            operations: vec![operation],
        }
    }

    /// Adapter support and current grants must also be checked by the app.
    pub fn validate(&self) -> Result<(), ProposalError> {
        if self.title.trim().is_empty() || self.title.len() > 4_096 {
            return Err(ProposalError::Title);
        }
        if self.operations.is_empty() || self.operations.len() > 128 {
            return Err(ProposalError::OperationCount);
        }
        if self
            .operations
            .iter()
            .any(|operation| !operation.is_valid())
        {
            return Err(ProposalError::InvalidOperation);
        }
        Ok(())
    }
}

/// Why a [`WorkflowProposal`] is not reviewable. The messages are shown to
/// the user as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProposalError {
    #[error("A workflow title of 1–4096 bytes is required.")]
    Title,
    #[error("A workflow must contain 1–128 ordered operations.")]
    OperationCount,
    #[error("Invalid operation payload. Nothing was changed.")]
    InvalidOperation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_rejects_empty_oversized_and_invalid_late_operations() {
        let mut proposal = WorkflowProposal::single(WorkflowOperation::CreateTask {
            title: "Valid".into(),
        });
        assert!(proposal.validate().is_ok());
        proposal
            .operations
            .push(WorkflowOperation::AddCapture { content: "".into() });
        assert_eq!(proposal.validate(), Err(ProposalError::InvalidOperation));
        proposal.operations.clear();
        assert_eq!(proposal.validate(), Err(ProposalError::OperationCount));
        proposal.operations = vec![
            WorkflowOperation::CreateTask {
                title: "Valid".into()
            };
            129
        ];
        assert!(proposal.validate().is_err());
        proposal.operations.truncate(128);
        assert!(proposal.validate().is_ok());
        proposal.title = " ".into();
        assert_eq!(proposal.validate(), Err(ProposalError::Title));
        assert_eq!(
            ProposalError::Title.to_string(),
            "A workflow title of 1–4096 bytes is required."
        );
    }

    #[test]
    fn batch_payload_has_no_approval_or_execution_field() {
        for field in ["approved", "execute", "auto_run"] {
            let mut value = serde_json::json!({"title":"Review", "operations":[{"type":"create_task","title":"Task"}]});
            value[field] = true.into();
            assert!(serde_json::from_value::<WorkflowProposal>(value).is_err());
        }
    }
}
