//! Saving and restoring the workspace: the debounced snapshot writer, atomic writes with one backup, and what a restart restores.

use super::*;
pub(super) const DESKTOP_PERSIST_DEBOUNCE_MS: u64 = 150;

#[cfg(not(test))]
pub(super) fn desktop_state_path() -> Option<PathBuf> {
    crate::paths::desktop_state_file()
}

pub(super) fn load_desktop_state_from_path(
    path: &Path,
) -> Result<Option<PersistedWorkspaceState>, String> {
    read_desktop_state_snapshot(path).map(|(state, _)| state)
}

pub(super) fn read_desktop_state_snapshot(
    path: &Path,
) -> Result<(Option<PersistedWorkspaceState>, DiskVersion), String> {
    state_file::reject_non_file(path).map_err(|error| error.to_string())?;
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((None, DiskVersion::Missing))
        }
        Err(err) => {
            return Err(format!(
                "Could not read saved desktop state at {}: {err}",
                path.display()
            ));
        }
    };

    serde_json::from_slice(&contents)
        .map(|state| (Some(state), DiskVersion::of_bytes(&contents)))
        .map_err(|err| {
            format!(
                "Saved desktop state at {} is invalid: {err}",
                path.display()
            )
        })
}

#[cfg(not(test))]
pub(super) fn load_desktop_state() -> (Option<PersistedWorkspaceState>, Option<String>, DiskVersion)
{
    let Some(path) = desktop_state_path() else {
        return (None, None, DiskVersion::Untrusted);
    };
    load_desktop_state_with_backup(&path)
}

pub(super) fn load_desktop_state_with_backup(
    path: &Path,
) -> (Option<PersistedWorkspaceState>, Option<String>, DiskVersion) {
    match read_desktop_state_snapshot(path) {
        Ok((state, version)) => (state, None, version),
        Err(err) => {
            tracing::error!("{err}");
            let backup_path = desktop_state_backup_path(path);
            match load_desktop_state_from_path(&backup_path) {
                Ok(Some(state)) => {
                    let recovery = format!(
                        "{err} Recovered the previous state from {}.",
                        backup_path.display()
                    );
                    tracing::warn!("{recovery}");
                    (Some(state), Some(recovery), DiskVersion::Untrusted)
                }
                Ok(None) => (None, Some(err), DiskVersion::Untrusted),
                Err(backup_err) => {
                    tracing::error!("{backup_err}");
                    (
                        None,
                        Some(format!("{err} {backup_err}")),
                        DiskVersion::Untrusted,
                    )
                }
            }
        }
    }
}

#[cfg(test)]
pub(super) fn load_desktop_state() -> (Option<PersistedWorkspaceState>, Option<String>, DiskVersion)
{
    (None, None, DiskVersion::Missing)
}

pub(super) fn atomic_state_temp_path(path: &Path, generation: u64) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("desktop-state.json");
    // A crash or another process may leave the same generation behind.
    // Never adopt, truncate, or remove a temp file we did not create.
    path.with_file_name(format!(
        ".{file_name}.{generation}.{}.tmp",
        uuid::Uuid::new_v4()
    ))
}

pub(super) fn desktop_state_backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("desktop-state.json");
    path.with_file_name(format!("{file_name}.bak"))
}

pub(super) fn create_private_state_file(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
pub(super) fn write_desktop_state_atomically(
    path: &Path,
    state: &PersistedWorkspaceState,
    generation: u64,
) -> io::Result<()> {
    let contents = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    write_desktop_state_bytes(path, &contents, generation)
}

pub(super) fn write_desktop_state_bytes(
    path: &Path,
    contents: &[u8],
    generation: u64,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;

    let temp_path = atomic_state_temp_path(path, generation);
    let backup_path = desktop_state_backup_path(path);
    let backup_temp_path = atomic_state_temp_path(&backup_path, generation);
    let mut owns_temp = false;
    let mut owns_backup_temp = false;
    let write_result = (|| {
        if path.exists() {
            let mut backup = create_private_state_file(&backup_temp_path)?;
            owns_backup_temp = true;
            io::copy(&mut fs::File::open(path)?, &mut backup)?;
            backup.sync_all()?;
            fs::rename(&backup_temp_path, &backup_path)?;
            owns_backup_temp = false;
        }
        let mut file = create_private_state_file(&temp_path)?;
        owns_temp = true;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temp_path, path)?;
        owns_temp = false;
        #[cfg(unix)]
        {
            // The file contents and the directory entry have distinct durability.
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    })();

    if write_result.is_err() {
        if owns_temp {
            let _ = fs::remove_file(&temp_path);
        }
        if owns_backup_temp {
            let _ = fs::remove_file(&backup_temp_path);
        }
    }
    write_result
}

pub(super) fn save_desktop_state(
    path: Option<PathBuf>,
    state: PersistedWorkspaceState,
    coordinator: Arc<PersistenceCoordinator>,
    generation: u64,
) -> Result<(), SaveError> {
    let contents = serde_json::to_vec_pretty(&state).map_err(io::Error::other)?;
    let mut expected = coordinator
        .write_lock
        .lock()
        .map_err(|_| SaveError::from("Desktop state writer lock was poisoned."))?;
    if coordinator.latest_generation.load(AtomicOrdering::Acquire) != generation {
        return Ok(());
    }
    let path = path.ok_or_else(|| SaveError::from("Desktop state path is unavailable."))?;
    state_file::guarded_write(&path, &mut expected, &contents, |bytes| {
        write_desktop_state_bytes(&path, bytes, generation)
    })
}

pub(super) fn parse_persisted_datetime(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, DESKTOP_DATETIME_FORMAT).ok()
}

pub(super) fn initial_permission_mode_from_persisted(
    persisted: Option<&PersistedWorkspaceState>,
) -> PermissionMode {
    persisted
        .and_then(|state| state.permission_mode)
        .or_else(|| {
            let state = persisted?;
            state
                .active_thread_id
                .as_deref()
                .and_then(|active_id| {
                    state
                        .threads
                        .iter()
                        .find(|thread| thread.id == active_id)
                        .map(|thread| thread.permission_mode)
                })
                .or_else(|| state.threads.first().map(|thread| thread.permission_mode))
        })
        .unwrap_or(PermissionMode::Default)
}

pub(super) fn restored_thread_model_id(current_model: &str, _persisted_model_id: &str) -> String {
    current_model.to_string()
}

/// The saved provider session and its owner. A session whose provider this
/// build cannot read is dropped: resuming it with the wrong CLI would fail.
pub(super) fn restored_session(
    persisted: &PersistedThread,
) -> (Option<String>, Option<bruhs_models::agent::AgentProvider>) {
    let provider = persisted
        .session_provider
        .as_deref()
        .and_then(|provider| provider.parse().ok());
    match (persisted.session_id.clone(), provider) {
        (Some(id), Some(provider)) if !id.is_empty() => (Some(id), Some(provider)),
        _ => (None, None),
    }
}

pub(super) fn restored_thread_permission_mode(
    current_permission_mode: PermissionMode,
    _persisted_permission_mode: PermissionMode,
) -> PermissionMode {
    current_permission_mode
}

pub(super) fn initial_active_view_after_start(_persisted_active_view: Option<AppView>) -> AppView {
    AppView::Chat
}

pub(super) fn initial_sidebar_visible_after_start(
    _persisted_sidebar_visible: Option<bool>,
) -> bool {
    true
}

pub(super) fn initial_active_thread_id_after_start(
    _persisted_active_thread_id: Option<String>,
    thread_ids: &[String],
) -> Option<String> {
    thread_ids.first().cloned()
}

pub(super) fn initial_selected_project_after_start(
    _persisted_project_id: Option<String>,
) -> Option<String> {
    None
}

pub(super) fn initial_selected_work_item_after_start(
    _persisted_work_item_id: Option<String>,
) -> Option<String> {
    None
}

pub(super) fn initial_calendar_mode_after_start(
    _persisted_calendar_mode: Option<DesktopCalendarMode>,
) -> DesktopCalendarMode {
    DesktopCalendarMode::Month
}

pub(super) fn initial_calendar_anchor_after_start(
    _persisted_calendar_anchor: Option<&str>,
    today: NaiveDate,
) -> NaiveDate {
    today
}

pub(super) fn initial_selected_calendar_event_after_start(
    _persisted_event_id: Option<String>,
) -> Option<String> {
    None
}

impl Workspace {
    pub(super) fn snapshot(&self, cx: &App) -> PersistedWorkspaceState {
        PersistedWorkspaceState {
            current_model: Some(self.current_model.clone()),
            permission_mode: Some(self.permission_mode),
            sidebar_visible: Some(self.sidebar_visible),
            sidebar_tab: Some(self.sidebar_tab),
            active_view: Some(self.active_view.clone()),
            // A smoke fixture thread is never written to disk.
            active_thread_id: if self.active_thread_id.is_some()
                && self.active_thread_id == self.smoke_thread_id
            {
                self.smoke_previous_active_id.clone()
            } else {
                self.active_thread_id.clone()
            },
            task_view_mode: Some(self.task_view_mode),
            selected_project_id: self.selected_project_id.clone(),
            selected_work_item_id: self.selected_work_item_id.clone(),
            calendar_mode: Some(self.calendar_mode),
            calendar_anchor: Some(self.calendar_anchor.format(DESKTOP_DATE_FORMAT).to_string()),
            selected_calendar_event_id: self.selected_calendar_event_id.clone(),
            threads: self
                .threads
                .iter()
                .filter(|thread| Some(&thread.id) != self.smoke_thread_id.as_ref())
                .map(|thread| thread.snapshot(cx))
                .collect(),
            projects: self
                .projects
                .iter()
                .map(|project| PersistedProject {
                    id: project.id.clone(),
                    name: project.name.clone(),
                })
                .collect(),
            work_items: self
                .work_items
                .iter()
                .map(|item| PersistedWorkItem {
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
                .collect(),
            calendar_events: self
                .calendar_events
                .iter()
                .map(|event| PersistedCalendarEvent {
                    id: event.id.clone(),
                    title: event.title.clone(),
                    start_at: event.start_at.format(DESKTOP_DATETIME_FORMAT).to_string(),
                    end_at: event.end_at.format(DESKTOP_DATETIME_FORMAT).to_string(),
                    all_day: event.all_day,
                    color: event.color.clone(),
                    notes: event.notes.clone(),
                })
                .collect(),
            captures: self.captures.clone(),
            knowledge_documents: self.knowledge_documents.clone(),
            source_clips: self.source_clips.clone(),
            inbox_conversations: self.inbox_conversations.clone(),
            meeting_notes: self.meeting_notes.clone(),
            relationships: self.relationships.clone(),
            people_cache: self.meetings_hub.read(cx).people_snapshot(),
            meeting_cache: self.meetings_hub.read(cx).meeting_cache(),
            meeting_actions: self.meetings_hub.read(cx).action_checkpoints(),
            scheduling_preferences: self.scheduling_preferences.clone(),
            life_context: self.life_context.clone(),
            automation_state: self.automation_state.clone(),
            active_project: self
                .active_project
                .as_ref()
                .map(|project| PersistedProjectLocation {
                    path: project.path.clone(),
                    name: project.name.clone(),
                }),
            recent_projects: self
                .recent_projects
                .iter()
                .map(|project| PersistedProjectLocation {
                    path: project.path.clone(),
                    name: project.name.clone(),
                })
                .collect(),
            sidebar_settled_open: Some(self.sidebar_settled_open),
            sidebar_working_open: Some(self.sidebar_working_open),
            sidebar_snoozed_open: Some(self.sidebar_snoozed_open),
            sidebar_working_section: Some(self.sidebar_working_section),
            update_checks: Some(self.updates.read(cx).enabled),
            title_instructions: Some(self.title_instructions.clone()),
            title_instructions_by_project: self.title_instructions_by_project.clone(),
            editor_id: Some(self.editor_id.clone()),
            terminal_drawer_height: Some(self.terminal_drawer_height),
            right_panel_width: Some(self.right_panel_width),
            meeting_prefs: Some(self.meetings_hub.read(cx).prefs.clone()),
            run_choice: self.run_choice.as_str().to_string(),
            claude_effort: self.claude_effort.clone(),
            follow_up: self.follow_up_steer.then(|| "steer".to_string()),
            claude_autocompact: self.claude_autocompact.clone(),
            compact_idle: (self.compact_idle != IdleCompact::Return)
                .then(|| self.compact_idle.as_str().to_string()),
            previous_worktree: self.previous_worktree.clone(),
            keybindings: self.keybindings.clone(),
        }
    }

    pub(super) fn persist(&mut self, cx: &mut Context<'_, Self>) {
        #[cfg(test)]
        {
            self.persist_requests += 1;
        }
        // Optimistic edits must be searchable even when their save fails or
        // changes neither the collection length nor a coarse timestamp.
        self.universal_search_index.replace(None);
        #[cfg(not(test))]
        let path = desktop_state_path();
        #[cfg(test)]
        let path = match self.persistence_test_path.clone() {
            Some(path) => Some(path),
            // Most rendering tests do not need disk; durability tests opt in
            // per workspace, never through process-wide environment variables.
            None => return,
        };
        {
            if !self.save_status.begin() {
                return;
            }
            if !self.save_queue.request() {
                return;
            }
            cx.spawn(async move |workspace, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(
                        DESKTOP_PERSIST_DEBOUNCE_MS,
                    ))
                    .await;
                // Snapshot once per batch, not once per keystroke or fragment.
                let Ok((state, meeting_action_receipts, coordinator, generation)) = workspace
                    .update(cx, |this, cx| {
                        this.save_queue.begin_write();
                        let generation = this
                            .persistence
                            .latest_generation
                            .fetch_add(1, AtomicOrdering::AcqRel)
                            + 1;
                        (
                            this.snapshot(cx),
                            this.meetings_hub.read(cx).action_save_receipts(),
                            this.persistence.clone(),
                            generation,
                        )
                    })
                else {
                    return;
                };
                let result = cx
                    .background_spawn(async move {
                        save_desktop_state(path, state, coordinator, generation)
                    })
                    .await;
                let _ = workspace.update(cx, |this, cx| {
                    let dirty = this.save_queue.finish();
                    this.save_status.finish(result);
                    let saved = this.save_status.is_saved();
                    this.meetings_hub.update(cx, |hub, cx| {
                        if saved {
                            hub.meeting_actions_saved(meeting_action_receipts, cx);
                        } else {
                            hub.meeting_actions_save_failed(meeting_action_receipts, cx);
                        }
                    });
                    if saved && dirty {
                        this.persist(cx);
                    }
                    this.drive_background_automations(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn persist_and_notify(&mut self, cx: &mut Context<'_, Self>) {
        self.persist(cx);
        cx.notify();
    }
}

/// The records a restart restores from the state file.
pub(super) struct RestoredRecords {
    pub(super) projects: Vec<DesktopProject>,
    pub(super) work_items: Vec<DesktopWorkItem>,
    pub(super) calendar_events: Vec<DesktopCalendarEvent>,
    pub(super) captures: Vec<CaptureRecord>,
    pub(super) knowledge_documents: Vec<KnowledgeDocument>,
    pub(super) source_clips: Vec<SourceClip>,
    pub(super) inbox_conversations: Vec<InboxConversation>,
    pub(super) meeting_notes: Vec<bruhs_models::MeetingNote>,
    pub(super) relationships: Vec<RelationshipProfile>,
    pub(super) scheduling_preferences: SchedulingPreferences,
    pub(super) life_context: LifeContext,
    pub(super) automation_state: AutomationState,
    pub(super) active_project: Option<DesktopProjectLocation>,
    pub(super) recent_projects: Vec<DesktopProjectLocation>,
}

impl RestoredRecords {
    /// The saved records, or empty ones; seeds the focus-block suggestion.
    pub(super) fn from_persisted(persisted: Option<&PersistedWorkspaceState>) -> Self {
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

impl Workspace {
    /// Contact captures become people in the hub.
    pub(super) fn restore_contact_people(
        captures: &[CaptureRecord],
        meetings_hub: &Entity<MeetingsHub>,
        cx: &mut Context<'_, Self>,
    ) {
        for record in captures
            .iter()
            .filter(|record| record.kind == CaptureKind::Contact)
        {
            let mut person = PersonProfile::new(&record.title);
            if let Some(id) = &record.target_id {
                person.id = id.clone();
            }
            person.created_at = record.created_at_ms;
            person.updated_at = record.updated_at_ms;
            meetings_hub.update(cx, |hub, cx| hub.capture_person(person, cx));
        }
    }
}
