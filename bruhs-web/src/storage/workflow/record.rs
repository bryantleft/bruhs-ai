use super::*;

pub enum CreatedRecord {
    Thread(Box<Thread>),
    Task(Box<WorkItem>),
    Event(Box<CalendarEvent>),
    Capture(Box<CaptureRecord>),
    Knowledge(Box<KnowledgeDocument>),
    Inbox(Box<InboxConversation>),
    MeetingNote(Box<MeetingNote>),
    Finance(Box<FinanceTransaction>),
    Workout(Box<WorkoutLog>),
    Relationship(Box<bruhs_models::RelationshipProfile>),
}

impl CreatedRecord {
    pub(super) fn prepare(operation: &WorkflowOperation, now: i64) -> Result<Self, SaveError> {
        match operation {
            WorkflowOperation::CreateChatThread { title } => {
                Ok(Self::Thread(Box::new(Thread::new(
                    title.clone(),
                    String::new(),
                    None,
                    bruhs_models::EpochMs::new(now).map_err(|_| SaveError::Corrupt)?,
                ))))
            }
            WorkflowOperation::CreateTask { title } => Ok(Self::Task(Box::new(WorkItem {
                id: uuid::Uuid::new_v4().to_string(),
                project_id: None,
                title: title.clone(),
                description: Some("Created by an approved bruhs workflow.".into()),
                status: WorkItemStatus::Todo,
                priority: Priority::Medium,
                due_date: None,
                planning: WorkItemPlanning::default(),
                created_at: now,
                updated_at: now,
            }))),
            WorkflowOperation::CreateCalendarEvent {
                title,
                start_at_ms,
                end_at_ms,
            } => Ok(Self::Event(Box::new(CalendarEvent {
                id: uuid::Uuid::new_v4().to_string(),
                title: title.clone(),
                start_at_ms: *start_at_ms,
                end_at_ms: *end_at_ms,
                all_day: false,
                color: Some("persimmon".into()),
                notes: Some("Created by an approved bruhs workflow.".into()),
                created_at: now,
                updated_at: now,
                source: EventSource::Local,
                google_event_id: None,
                calendar_id: None,
                attendees: Vec::new(),
                organizer: None,
                location: None,
                conferencing: None,
                recurring_event_id: None,
                html_link: None,
            }))),
            WorkflowOperation::CreateKnowledgeNote { title, body } => Ok(Self::Knowledge(
                Box::new(KnowledgeDocument::new(title, body, now)),
            )),
            WorkflowOperation::CreateMeetingNote { title, body } => {
                MeetingNote::new(title.clone(), body.clone(), now)
                    .map(|note| Self::MeetingNote(Box::new(note)))
                    .map_err(|error| SaveError::Rejected(error.message()))
            }
            WorkflowOperation::DraftInboxMessage { recipient, body } => Ok(Self::Inbox(Box::new(
                InboxConversation::local_draft(recipient, body, now),
            ))),
            WorkflowOperation::AddCapture { content } => {
                CaptureRecord::new(CaptureKind::Note, content, now)
                    .map(|record| Self::Capture(Box::new(record)))
                    .map_err(|_| {
                        SaveError::Rejected("Capture text is required. No records were changed.")
                    })
            }
            WorkflowOperation::AddFinanceTransaction {
                merchant,
                category,
                amount_minor,
            } => Ok(Self::Finance(Box::new(FinanceTransaction::manual(
                merchant,
                category,
                *amount_minor,
                now,
            )))),
            WorkflowOperation::LogWorkout { activity, minutes } => Ok(Self::Workout(Box::new(
                WorkoutLog::manual(activity, *minutes, now),
            ))),
            _ => Err(SaveError::Rejected(
                "This operation does not have a workspace adapter yet. No records were changed.",
            )),
        }
    }

    pub(super) fn parts(&self) -> Result<(&str, &'static str, JsValue, String), SaveError> {
        fn encode<T: serde::Serialize>(record: &T) -> Result<(JsValue, String), SaveError> {
            Ok((
                to_js(record)?,
                serde_json::to_string(record).map_err(|_| SaveError::Corrupt)?,
            ))
        }
        let (id, store, (value, snapshot)) = match self {
            Self::Thread(record) => (&record.id, STORE_THREADS, encode(record)?),
            Self::Task(record) => (&record.id, STORE_WORK_ITEMS, encode(record)?),
            Self::Event(record) => (&record.id, STORE_EVENTS, encode(record)?),
            Self::Capture(record) => (&record.id, STORE_CAPTURES, encode(record)?),
            Self::Knowledge(record) => (&record.id, STORE_KNOWLEDGE_DOCUMENTS, encode(record)?),
            Self::Inbox(record) => (&record.id, STORE_INBOX_CONVERSATIONS, encode(record)?),
            Self::MeetingNote(record) => (&record.id, STORE_MEETING_NOTES, encode(record)?),
            Self::Finance(record) => (&record.id, STORE_METADATA, encode(record)?),
            Self::Workout(record) => (&record.id, STORE_METADATA, encode(record)?),
            Self::Relationship(_) => {
                return Err(SaveError::Rejected(
                    "People reminders require the identity-checked relationship adapter.",
                ))
            }
        };
        Ok((id, store, value, snapshot))
    }
}
