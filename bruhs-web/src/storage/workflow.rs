//! Local workflow execution: domain records and checkpoints share one commit.

use super::metadata::{
    encode, finish, next_revision, read_in, LocalTransaction, Metadata, SaveError,
};
use super::{
    from_js, open_db, to_js, STORE_CAPTURES, STORE_EVENTS, STORE_INBOX_CONVERSATIONS,
    STORE_KNOWLEDGE_DOCUMENTS, STORE_MEETING_NOTES, STORE_MESSAGES, STORE_METADATA, STORE_THREADS,
    STORE_WORK_ITEMS,
};
use crate::state::Thread;
use bruhs_models::{
    AgentDomain, AutomationState, CalendarEvent, CaptureKind, CaptureRecord, EventSource,
    FinanceTransaction, InboxConversation, KnowledgeDocument, LifeContext, MeetingNote, Priority,
    WorkItem, WorkItemPlanning, WorkItemStatus, WorkflowOperation, WorkoutLog,
};
use futures_util::future::join_all;
use idb::TransactionMode;
use std::collections::HashMap;
use wasm_bindgen::JsValue;
mod life;
mod people;
pub use life::{is_life_domain, rollback_needs_life};
pub use people::rollback_people_ids;

pub struct WorkflowCommit<T> {
    pub automation: Metadata<AutomationState>,
    pub life: Option<Metadata<LifeContext>>,
    pub result: T,
}

mod record;
pub use record::CreatedRecord;

fn domain_store(domain: AgentDomain) -> Result<&'static str, SaveError> {
    match domain {
        AgentDomain::Chat => Ok(STORE_THREADS),
        AgentDomain::Tasks => Ok(STORE_WORK_ITEMS),
        AgentDomain::Calendar => Ok(STORE_EVENTS),
        AgentDomain::Capture => Ok(STORE_CAPTURES),
        AgentDomain::Knowledge => Ok(STORE_KNOWLEDGE_DOCUMENTS),
        AgentDomain::Inbox => Ok(STORE_INBOX_CONVERSATIONS),
        AgentDomain::Meetings => Ok(STORE_MEETING_NOTES),
        _ => Err(SaveError::Rejected(
            "This domain has no rollback adapter. No records were changed.",
        )),
    }
}

fn snapshot(domain: AgentDomain, raw: JsValue) -> Result<String, SaveError> {
    let result = match domain {
        AgentDomain::Chat => serde_json::to_string(&from_js::<Thread>(raw)?),
        AgentDomain::Tasks => serde_json::to_string(&from_js::<WorkItem>(raw)?),
        AgentDomain::Calendar => serde_json::to_string(&from_js::<CalendarEvent>(raw)?),
        AgentDomain::Capture => serde_json::to_string(&from_js::<CaptureRecord>(raw)?),
        AgentDomain::Knowledge => serde_json::to_string(&from_js::<KnowledgeDocument>(raw)?),
        AgentDomain::Inbox => serde_json::to_string(&from_js::<InboxConversation>(raw)?),
        AgentDomain::Meetings => serde_json::to_string(&from_js::<MeetingNote>(raw)?),
        _ => return Err(SaveError::Rejected("Unsupported rollback domain.")),
    };
    result.map_err(|_| SaveError::Corrupt)
}

async fn checked_state<T: serde::de::DeserializeOwned + PartialEq>(
    tx: &idb::Transaction,
    expected: &Metadata<T>,
) -> Result<Metadata<T>, SaveError> {
    let mut current: Metadata<T> = read_in(tx, &expected.key).await?;
    let next = next_revision(current.revision, expected.revision)?;
    if current.value != expected.value {
        return Err(SaveError::Conflict);
    }
    current.revision = next;
    Ok(current)
}

pub async fn execute_step(
    expected: &Metadata<AutomationState>,
    expected_life: Option<&Metadata<LifeContext>>,
    expected_relationships: Option<&crate::relationship_store::ExpectedRelationships>,
    job_id: &str,
    now: i64,
) -> Result<WorkflowCommit<CreatedRecord>, SaveError> {
    let operation = expected.value.pending_operation(job_id).ok_or(SaveError::Rejected(
        "This job is paused, finished, changed since preview, or a required Write grant was revoked. No records were changed."))?;
    if operation.domain() == AgentDomain::People {
        return people::execute(expected, expected_relationships, operation, job_id, now).await;
    }
    let record = CreatedRecord::prepare(operation, now)?;
    let (id, store_name, value, receipt) = record.parts()?;
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_METADATA, store_name], TransactionMode::ReadWrite)?,
    );
    let result = async {
        let mut current = checked_state(&tx, expected).await?;
        let mut life =
            life::checked(&tx, expected_life, is_life_domain(operation.domain())).await?;
        if let Some(life) = &mut life {
            match &record {
                CreatedRecord::Finance(record) => life
                    .value
                    .finance
                    .transactions
                    .insert(0, record.as_ref().clone()),
                CreatedRecord::Workout(record) => life
                    .value
                    .health
                    .workouts
                    .insert(0, record.as_ref().clone()),
                _ => return Err(SaveError::Rejected("Unexpected Life operation.")),
            }
        }
        if !current
            .value
            .complete_step_with_snapshot(job_id, id, receipt, now)
        {
            return Err(SaveError::Rejected(
                "The job no longer permits this step. No records were changed.",
            ));
        }
        // Drain both requests even on failure: cancelling one future leaves
        // its JS request holding an invalidated Rust event callback.
        let (created, saved) = futures_util::join!(
            async {
                if life.is_none() {
                    tx.object_store(store_name)?.add(&value, None)?.await?;
                }
                Ok::<_, SaveError>(())
            },
            save_checkpoints(&tx, &current, life.as_ref())
        );
        created?;
        saved?;
        Ok((current, life))
    }
    .await;
    let (automation, life) = finish(tx, result).await?;
    Ok(WorkflowCommit {
        automation,
        life,
        result: record,
    })
}

async fn save_checkpoints(
    tx: &idb::Transaction,
    automation: &Metadata<AutomationState>,
    life: Option<&Metadata<LifeContext>>,
) -> Result<(), SaveError> {
    let (automation_saved, life_saved) = futures_util::join!(
        async {
            tx.object_store(STORE_METADATA)?
                .put(&encode(automation)?, None)?
                .await?;
            Ok::<_, SaveError>(())
        },
        async {
            if let Some(life) = life {
                tx.object_store(STORE_METADATA)?
                    .put(&encode(life)?, None)?
                    .await?;
            }
            Ok::<_, SaveError>(())
        }
    );
    automation_saved?;
    life_saved
}

mod rollback;
pub use rollback::rollback;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_preparation_is_local_empty_and_preserves_reviewed_title() {
        let CreatedRecord::Thread(thread) = CreatedRecord::prepare(
            &WorkflowOperation::CreateChatThread {
                title: "Reviewed chat 🗒".into(),
            },
            42,
        )
        .unwrap() else {
            panic!("chat record")
        };
        assert_eq!(thread.title, "Reviewed chat 🗒");
        assert_eq!(thread.updated_at.as_millis(), 42);
        assert!(thread.project_name.is_empty());
        assert!(thread.project_path.is_none());
        assert!(thread.session_id.is_none());
        assert!(thread.model.is_none());
        assert!(!thread.pinned && !thread.settled);
        assert_eq!(domain_store(AgentDomain::Chat).unwrap(), STORE_THREADS);
    }
    #[test]
    fn life_preparation_preserves_payload_and_manual_defaults() {
        let CreatedRecord::Finance(record) = CreatedRecord::prepare(
            &WorkflowOperation::AddFinanceTransaction {
                merchant: "Receipt".into(),
                category: "Manual".into(),
                amount_minor: -9_007_199_254_740_993,
            },
            42,
        )
        .unwrap() else {
            panic!("finance record")
        };
        assert_eq!(record.amount_minor, -9_007_199_254_740_993);
        assert_eq!(record.currency, "USD");
        assert_eq!(record.occurred_at_ms, 42);
        assert!(record.account_id.is_none());
        let CreatedRecord::Workout(record) = CreatedRecord::prepare(
            &WorkflowOperation::LogWorkout {
                activity: "Walk".into(),
                minutes: 37,
            },
            43,
        )
        .unwrap() else {
            panic!("workout record")
        };
        assert_eq!(record.duration_minutes, 37);
        assert_eq!(record.perceived_exertion, 5);
        assert_eq!(record.occurred_at_ms, 43);
    }
    #[test]
    fn meeting_note_preparation_uses_local_text_store_and_exact_payload() {
        let CreatedRecord::MeetingNote(note) = CreatedRecord::prepare(
            &WorkflowOperation::CreateMeetingNote {
                title: "Reviewed note".into(),
                body: "Keep exact\n\n  text".into(),
            },
            42,
        )
        .unwrap() else {
            panic!("meeting note record")
        };
        assert_eq!(note.title, "Reviewed note");
        assert_eq!(note.body, "Keep exact\n\n  text");
        assert_eq!(note.created_at, 42);
        assert_eq!(
            domain_store(AgentDomain::Meetings).unwrap(),
            STORE_MEETING_NOTES
        );
    }
}
