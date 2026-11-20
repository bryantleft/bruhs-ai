//! Frozen pre-ownership restorer for controlled comparisons; do not update to match production.
use super::super::super::*;

impl RestoredRecords {
    /// The saved records, or empty ones; seeds the focus-block suggestion.
    pub(super) fn original_from_persisted(persisted: Option<&PersistedWorkspaceState>) -> Self {
        let projects = persisted
            .map(|state| {
                state
                    .projects
                    .iter()
                    .map(|project| DesktopProject {
                        id: project.id.clone(),
                        name: project.name.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let work_items = persisted
            .map(|state| {
                state
                    .work_items
                    .iter()
                    .map(|item| DesktopWorkItem {
                        source: None,
                        id: item.id.clone(),
                        project_id: item.project_id.clone(),
                        title: item.title.clone(),
                        description: item.description.clone(),
                        status: item.status,
                        priority: item.priority,
                        due_date: item.due_date.clone(),
                        planning: item.planning.clone(),
                        updated_at: item.updated_at,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let calendar_events = persisted
            .map(|state| {
                state
                    .calendar_events
                    .iter()
                    .filter_map(|event| {
                        Some(DesktopCalendarEvent {
                            id: event.id.clone(),
                            title: event.title.clone(),
                            start_at: parse_persisted_datetime(&event.start_at)?,
                            end_at: parse_persisted_datetime(&event.end_at)?,
                            all_day: event.all_day,
                            color: event.color.clone(),
                            notes: event.notes.clone(),
                            ..Default::default()
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let captures = persisted
            .map(|state| state.captures.clone())
            .unwrap_or_default();
        let knowledge_documents = persisted
            .map(|state| state.knowledge_documents.clone())
            .unwrap_or_default();
        let source_clips = persisted
            .map(|state| state.source_clips.clone())
            .unwrap_or_default();
        let inbox_conversations = persisted
            .map(|state| state.inbox_conversations.clone())
            .unwrap_or_default();
        let meeting_notes = persisted
            .map(|state| state.meeting_notes.clone())
            .unwrap_or_default();
        let relationships = persisted
            .map(|state| state.relationships.clone())
            .unwrap_or_default();
        let scheduling_preferences = persisted
            .map(|state| state.scheduling_preferences.clone())
            .unwrap_or_default();
        let life_context = persisted
            .map(|state| state.life_context.clone())
            .unwrap_or_default();
        let mut automation_state = persisted
            .map(|state| state.automation_state.clone())
            .unwrap_or_default();
        if automation_state
            .suggestions
            .iter()
            .all(|suggestion| suggestion.id != "protect-focus")
        {
            automation_state
                .suggestions
                .push(bruhs_models::ProactiveSuggestion {
                id: "protect-focus".into(),
                title: "Protect a focus block".into(),
                explanation:
                    "Create a task, calendar block, and local note as one reversible transaction."
                        .into(),
                evidence: format!(
                    "{} tasks and {} calendar entries are currently stored locally.",
                    work_items.len(),
                    calendar_events.len()
                ),
                domain: bruhs_models::AgentDomain::Tasks,
                operations: vec![
                    bruhs_models::WorkflowOperation::CreateTask {
                        title: "Finish the highest-impact task".into(),
                    },
                    bruhs_models::WorkflowOperation::AddCapture {
                        content: "Focus plan".into(),
                    },
                ],
                dismissed: false,
                shown_count: 0,
                last_shown_at_ms: None,
            });
        }
        let active_project = persisted
            .and_then(|state| state.active_project.as_ref())
            .map(|project| DesktopProjectLocation {
                path: project.path.clone(),
                name: project.name.clone(),
            });
        let recent_projects = persisted
            .map(|state| {
                state
                    .recent_projects
                    .iter()
                    .map(|project| DesktopProjectLocation {
                        path: project.path.clone(),
                        name: project.name.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            projects,
            work_items,
            calendar_events,
            captures,
            knowledge_documents,
            source_clips,
            inbox_conversations,
            meeting_notes,
            relationships,
            scheduling_preferences,
            life_context,
            automation_state,
            active_project,
            recent_projects,
        }
    }
}
