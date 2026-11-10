//! Universal local search across every hydrated bruhs domain.
//!
//! Source records are projected into `SearchIndex` only when data changes.
//! Keystrokes run a synchronous scan over normalized text; IndexedDB is read
//! once by the app-owned search context, never per query or route visit.

use std::collections::{BTreeMap, HashMap, HashSet};

use bruhs_models::{
    CaptureKind, CaptureRecord, InboxConversation, KnowledgeDocument, SearchDocument, SearchHit,
    SearchIndex, SearchKind, SourceClip,
};
use leptos::prelude::*;

use crate::state::{
    activate_thread, AppView, CalendarEventsSignal, CapturesSignal, InboxConversationsSignal,
    KnowledgeDocumentsSignal, LifeContextSignal, LifeSectionSignal, MeetingSegmentsSignal,
    MeetingsSignal, PeopleSignal, ProjectsSignal, SchedulingPreferencesSignal, SelectedLifeRecord,
    SelectedLifeRecordSignal, SelectedPersonSignal, SetActiveView, SetCaptureEditId,
    SetCaptureOpen, SetEventCreateOpen, SetSelectedEventId, SetSelectedProject,
    SetSelectedWorkItem, SourceClipsSignal, Thread, TranscriptSegment, Turns, UnreadThreads,
    WorkItemsSignal,
};
use crate::storage;
use crate::ui::{Icon, Input};

const SEARCH_LIMIT: usize = 50;
const RESULT_PREVIEW_CHARS: usize = 120;

pub(super) fn scroll_result_into_view(id: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    {
        let options = web_sys::ScrollIntoViewOptions::new();
        options.set_block(web_sys::ScrollLogicalPosition::Nearest);
        options.set_inline(web_sys::ScrollLogicalPosition::Nearest);
        element.scroll_into_view_with_scroll_into_view_options(&options);
    }
}

fn join_non_empty(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

struct SearchSources<'a> {
    threads: &'a [Thread],
    work_items: &'a [bruhs_models::WorkItem],
    projects: &'a [bruhs_models::Project],
    events: &'a [bruhs_models::CalendarEvent],
    scheduling: &'a bruhs_models::SchedulingPreferences,
    life: &'a bruhs_models::LifeContext,
    meetings: &'a [bruhs_models::Meeting],
    meeting_notes: &'a [bruhs_models::MeetingNote],
    people: &'a [bruhs_models::PersonProfile],
    captures: &'a [CaptureRecord],
    conversations: &'a [InboxConversation],
    documents: &'a [KnowledgeDocument],
    clips: &'a [SourceClip],
    stored_segments: &'a [TranscriptSegment],
    live_segments: &'a BTreeMap<String, Vec<TranscriptSegment>>,
}

fn build_documents(sources: SearchSources<'_>) -> Vec<SearchDocument> {
    let SearchSources {
        threads,
        work_items,
        projects,
        events,
        scheduling,
        life,
        meetings,
        meeting_notes,
        people,
        captures,
        conversations,
        documents: knowledge_documents,
        clips,
        stored_segments,
        live_segments,
    } = sources;
    let mut documents = Vec::with_capacity(
        threads.len()
            + work_items.len()
            + projects.len()
            + events.len()
            + scheduling.links.len()
            + life.finance.accounts.len()
            + life.finance.transactions.len()
            + life.finance.budgets.len()
            + life.finance.income_streams.len()
            + life.finance.holdings.len()
            + life.health.workouts.len()
            + life.health.sleep.len()
            + life.health.recovery.len()
            + life.health.habits.len()
            + meetings.len()
            + meeting_notes.len()
            + people.len()
            + captures.len()
            + conversations.len()
            + knowledge_documents.len()
            + clips.len()
            + stored_segments.len(),
    );
    let project_names = projects
        .iter()
        .map(|project| (project.id.as_str(), project.name.as_str()))
        .collect::<HashMap<_, _>>();
    let mut project_task_counts = HashMap::<&str, usize>::new();
    for item in work_items {
        if let Some(project_id) = item.project_id.as_deref() {
            *project_task_counts.entry(project_id).or_default() += 1;
        }
    }
    let meeting_metadata = meetings
        .iter()
        .map(|meeting| {
            (
                meeting.id.as_str(),
                (meeting.title.as_str(), meeting.updated_at),
            )
        })
        .collect::<HashMap<_, _>>();

    documents.extend(meeting_notes.iter().map(SearchDocument::from));
    documents.extend(threads.iter().map(|thread| {
        SearchDocument::new(
            &thread.id,
            SearchKind::Thread,
            &thread.title,
            &thread.project_name,
            join_non_empty([
                thread.branch.clone().unwrap_or_default(),
                thread.project_path.clone().unwrap_or_default(),
            ]),
            thread.updated_at.as_millis(),
        )
    }));
    documents.extend(work_items.iter().map(|item| {
        let project = item
            .project_id
            .as_deref()
            .and_then(|id| project_names.get(id).copied())
            .unwrap_or("No project");
        SearchDocument::new(
            &item.id,
            SearchKind::Task,
            &item.title,
            project,
            join_non_empty([
                item.description.clone().unwrap_or_default(),
                item.due_date.clone().unwrap_or_default(),
                item.status.label().to_string(),
                item.priority.label().to_string(),
                item.planning.cycle.clone().unwrap_or_default(),
                item.planning.waiting_on.clone().unwrap_or_default(),
                item.planning
                    .recurrence
                    .map_or_else(String::new, |rule| rule.label().to_string()),
            ]),
            item.updated_at,
        )
    }));
    documents.extend(projects.iter().map(|project| {
        let task_count = project_task_counts
            .get(project.id.as_str())
            .copied()
            .unwrap_or_default();
        SearchDocument::new(
            &project.id,
            SearchKind::Project,
            &project.name,
            format!(
                "{task_count} task{}",
                if task_count == 1 { "" } else { "s" }
            ),
            "",
            0,
        )
    }));
    documents.extend(events.iter().map(|event| {
        let attendees = event.attendees.iter().map(|attendee| {
            attendee
                .display_name
                .clone()
                .unwrap_or_else(|| attendee.email.clone())
        });
        SearchDocument::new(
            &event.id,
            SearchKind::Event,
            &event.title,
            if event.all_day {
                "All-day event"
            } else {
                "Calendar event"
            },
            join_non_empty(
                event
                    .notes
                    .clone()
                    .into_iter()
                    .chain(event.location.clone())
                    .chain(attendees),
            ),
            event.updated_at,
        )
    }));
    documents.extend(scheduling.links.iter().map(|link| {
        SearchDocument::new(
            &link.id,
            SearchKind::SchedulingLink,
            &link.title,
            format!("{} minutes · #/book/{}", link.duration_minutes, link.slug),
            format!(
                "{} hour notice · {} day window · {} minute buffers",
                link.minimum_notice_hours,
                link.booking_window_days,
                link.buffer_before_minutes + link.buffer_after_minutes
            ),
            scheduling.updated_at_ms,
        )
    }));
    documents.extend(life.finance.accounts.iter().map(|account| {
        SearchDocument::new(
            &account.id,
            SearchKind::Finance,
            &account.name,
            format!("{} account", account.kind.label()),
            format!(
                "{} {}",
                account.currency,
                account.institution.clone().unwrap_or_default()
            ),
            account.updated_at_ms,
        )
    }));
    documents.extend(life.finance.transactions.iter().map(|transaction| {
        SearchDocument::new(
            &transaction.id,
            SearchKind::Finance,
            &transaction.merchant,
            &transaction.category,
            transaction.note.clone().unwrap_or_default(),
            transaction.occurred_at_ms,
        )
    }));
    documents.extend(life.finance.budgets.iter().map(|budget| {
        SearchDocument::new(
            &budget.id,
            SearchKind::Finance,
            &budget.category,
            "Monthly budget",
            "",
            life.finance.updated_at_ms,
        )
    }));
    documents.extend(life.finance.income_streams.iter().map(|income| {
        SearchDocument::new(
            &income.id,
            SearchKind::Finance,
            &income.name,
            "Income stream",
            "",
            life.finance.updated_at_ms,
        )
    }));
    documents.extend(life.finance.holdings.iter().map(|holding| {
        SearchDocument::new(
            &holding.id,
            SearchKind::Finance,
            &holding.symbol,
            "Holding",
            &holding.name,
            holding.updated_at_ms,
        )
    }));
    documents.extend(life.health.workouts.iter().map(|workout| {
        SearchDocument::new(
            &workout.id,
            SearchKind::Health,
            &workout.activity,
            "Workout",
            format!(
                "{} minutes · effort {}",
                workout.duration_minutes, workout.perceived_exertion
            ),
            workout.occurred_at_ms,
        )
    }));
    documents.extend(life.health.sleep.iter().map(|sleep| {
        SearchDocument::new(
            &sleep.id,
            SearchKind::Health,
            "Sleep log",
            format!("{} minutes", sleep.duration_minutes),
            format!("{}% quality", sleep.quality_percent),
            sleep.ended_at_ms,
        )
    }));
    documents.extend(life.health.recovery.iter().map(|recovery| {
        SearchDocument::new(
            &recovery.id,
            SearchKind::Health,
            "Recovery log",
            format!("{}%", recovery.score_percent),
            recovery.note.clone().unwrap_or_default(),
            recovery.recorded_at_ms,
        )
    }));
    documents.extend(life.health.habits.iter().map(|habit| {
        SearchDocument::new(
            &habit.id,
            SearchKind::Health,
            &habit.title,
            "Habit",
            format!("{} days/week", habit.target_days_per_week),
            life.health.updated_at_ms,
        )
    }));
    documents.extend(meetings.iter().map(|meeting| {
        let body = meeting
            .summary
            .as_ref()
            .map_or_else(String::new, |summary| {
                join_non_empty(
                    std::iter::once(summary.tldr.clone())
                        .chain(summary.decisions.iter().cloned())
                        .chain(summary.action_items.iter().map(|item| item.text.clone()))
                        .chain(summary.topics.iter().flat_map(|topic| {
                            std::iter::once(topic.title.clone())
                                .chain(topic.bullets.iter().cloned())
                        })),
                )
            });
        SearchDocument::new(
            &meeting.id,
            SearchKind::Meeting,
            &meeting.title,
            match meeting.status {
                bruhs_models::MeetingStatus::Live => "Live meeting",
                bruhs_models::MeetingStatus::Ended => "Meeting",
                bruhs_models::MeetingStatus::Failed => "Interrupted meeting",
            },
            body,
            meeting.updated_at,
        )
    }));
    documents.extend(people.iter().map(|person| {
        SearchDocument::new(
            &person.id,
            SearchKind::Person,
            &person.display_name,
            format!(
                "{} observation{}",
                person.observations.len(),
                if person.observations.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            join_non_empty(
                person
                    .aliases
                    .iter()
                    .cloned()
                    .chain(person.observations.iter().map(|item| item.text.clone())),
            ),
            person.updated_at,
        )
    }));
    documents.extend(captures.iter().filter_map(|record| {
        if record.target_id.is_some() {
            return None;
        }
        let (kind, subtitle) = match record.kind {
            CaptureKind::Note => (SearchKind::Document, "Captured note"),
            CaptureKind::MessageDraft => (SearchKind::Conversation, "Message draft"),
            CaptureKind::Task | CaptureKind::Event | CaptureKind::Contact => return None,
        };
        Some(SearchDocument::new(
            &record.id,
            kind,
            &record.title,
            subtitle,
            &record.body,
            record.updated_at_ms,
        ))
    }));
    documents.extend(conversations.iter().map(|conversation| {
        SearchDocument::new(
            &conversation.id,
            SearchKind::Conversation,
            &conversation.subject,
            format!(
                "{} · {}",
                conversation.source.label(),
                conversation
                    .participants
                    .iter()
                    .map(|participant| participant.display_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            join_non_empty(
                conversation
                    .messages
                    .iter()
                    .map(|message| message.body.clone()),
            ),
            conversation.updated_at_ms,
        )
    }));
    documents.extend(knowledge_documents.iter().map(|document| {
        SearchDocument::new(
            &document.id,
            SearchKind::Document,
            &document.title,
            document
                .tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" "),
            join_non_empty(
                std::iter::once(document.body_markdown.clone())
                    .chain(document.links.iter().map(|link| link.label.clone())),
            ),
            document.updated_at_ms,
        )
    }));
    documents.extend(clips.iter().map(|clip| {
        SearchDocument::new(
            &clip.id,
            SearchKind::Clip,
            &clip.title,
            &clip.source_url,
            join_non_empty([
                clip.excerpt.clone(),
                clip.source_name.clone(),
                clip.author.clone(),
                clip.tags.join(" "),
            ]),
            clip.updated_at_ms,
        )
    }));

    let mut seen_segment_ids = HashSet::with_capacity(stored_segments.len());
    for segment in stored_segments
        .iter()
        .chain(live_segments.values().flatten())
        .filter(|segment| !segment.partial)
    {
        if !seen_segment_ids.insert(segment.id.as_str()) {
            continue;
        }
        let (title, updated_at) = meeting_metadata
            .get(segment.meeting_id.as_str())
            .copied()
            .unwrap_or(("Unknown meeting", 0));
        documents.push(SearchDocument::new(
            &segment.meeting_id,
            SearchKind::Transcript,
            title,
            "Meeting transcript",
            &segment.text,
            updated_at,
        ));
    }

    documents
}

pub(super) fn result_icon(kind: SearchKind) -> &'static str {
    match kind {
        SearchKind::Thread => "message-square",
        SearchKind::Task => "list-checks",
        SearchKind::Project => "folder",
        SearchKind::Event => "calendar",
        SearchKind::SchedulingLink => "calendar",
        SearchKind::Meeting | SearchKind::Transcript => "mic",
        SearchKind::Person => "user",
        SearchKind::Document | SearchKind::MeetingNote => "file-text",
        SearchKind::Clip => "bookmark",
        SearchKind::Conversation => "inbox",
        SearchKind::Finance => "wallet",
        SearchKind::Health => "activity",
    }
}

pub(super) fn preview(body: &str) -> String {
    let body = body.trim();
    if body.chars().count() <= RESULT_PREVIEW_CHARS {
        return body.to_string();
    }
    let mut value = body.chars().take(RESULT_PREVIEW_CHARS).collect::<String>();
    value.push('…');
    value
}

/// One lazy index and one record router shared by the page and command palette.
#[derive(Clone, Copy)]
pub(super) struct LocalSearch {
    pub index: Memo<SearchIndex>,
    pub transcripts_ready: RwSignal<bool>,
    pub load_history: Callback<()>,
    pub open: Callback<SearchHit>,
}

pub(super) fn provide_local_search() {
    let meeting_notes =
        expect_context::<crate::workflow_executor::WorkflowExecutor>().meeting_notes;
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let ProjectsSignal(projects) = expect_context::<ProjectsSignal>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SchedulingPreferencesSignal(scheduling) = expect_context::<SchedulingPreferencesSignal>();
    let LifeContextSignal(life) = expect_context::<LifeContextSignal>();
    let LifeSectionSignal(life_section) = expect_context::<LifeSectionSignal>();
    let SelectedLifeRecordSignal(selected_life) = expect_context::<SelectedLifeRecordSignal>();
    let SelectedPersonSignal(selected_person) = expect_context::<SelectedPersonSignal>();
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let CapturesSignal(captures) = expect_context::<CapturesSignal>();
    let KnowledgeDocumentsSignal(knowledge_documents) =
        expect_context::<KnowledgeDocumentsSignal>();
    let SourceClipsSignal(clips) = expect_context::<SourceClipsSignal>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let MeetingSegmentsSignal(live_segments) = expect_context::<MeetingSegmentsSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SetSelectedWorkItem(set_selected_work_item) = expect_context::<SetSelectedWorkItem>();
    let SetSelectedProject(set_selected_project) = expect_context::<SetSelectedProject>();
    let SetSelectedEventId(set_selected_event_id) = expect_context::<SetSelectedEventId>();
    let SetEventCreateOpen(set_event_create_open) = expect_context::<SetEventCreateOpen>();
    let SetCaptureOpen(set_capture_open) = expect_context::<SetCaptureOpen>();
    let SetCaptureEditId(set_capture_edit_id) = expect_context::<SetCaptureEditId>();
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let turns = expect_context::<Turns>();
    let UnreadThreads(unread_threads) = expect_context::<UnreadThreads>();

    let stored_segments = RwSignal::new(Vec::<TranscriptSegment>::new());
    let transcripts_ready = RwSignal::new(false);
    let history_started = StoredValue::new(false);

    let load_history = Callback::new(move |()| {
        if history_started.get_value() {
            return;
        }
        history_started.set_value(true);
        transcripts_ready.set(false);
        // History is shared across search surfaces, but unrelated routes should
        // not pay its hydration cost before the user opens either surface.
        leptos::task::spawn_local(async move {
            match storage::load_all_segments().await {
                Ok(segments) => stored_segments.set(segments),
                Err(error) => {
                    history_started.set_value(false);
                    log::warn!("universal search transcript hydrate failed: {error:?}");
                }
            }
            transcripts_ready.set(true);
        });
    });

    let index = Memo::new(move |_| {
        life.with(|life| {
            SearchIndex::new(build_documents(SearchSources {
                threads: &threads.read(),
                work_items: &work_items.read(),
                projects: &projects.read(),
                events: &events.read(),
                scheduling: &scheduling.read(),
                life,
                meetings: &meetings.read(),
                meeting_notes: &meeting_notes.read(),
                people: &people.read(),
                captures: &captures.read(),
                conversations: &conversations.read(),
                documents: &knowledge_documents.read(),
                clips: &clips.read(),
                stored_segments: &stored_segments.read(),
                live_segments: &live_segments.read(),
            }))
        })
    });

    let history = expect_context::<crate::thread_history::ThreadHistory>();
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let open_hit = Callback::new(move |hit: SearchHit| match hit.document.kind {
        SearchKind::Thread => {
            activate_thread(
                history,
                active_thread_id,
                hit.document.id,
                set_active_thread_id,
                set_messages,
                turns,
                unread_threads,
            );
            set_active_view.set(AppView::Chat);
        }
        SearchKind::Task => {
            set_selected_work_item.set(Some(hit.document.id));
            set_active_view.set(AppView::Tasks);
            super::record_focus::focus_record("task-detail-title".into());
        }
        SearchKind::Project => {
            set_selected_project.set(Some(hit.document.id));
            set_active_view.set(AppView::Tasks);
        }
        SearchKind::Event => {
            set_selected_event_id.set(Some(hit.document.id));
            set_active_view.set(AppView::Calendar);
            set_event_create_open.set(true);
        }
        SearchKind::SchedulingLink => {
            if let Some(link) = scheduling
                .get_untracked()
                .links
                .into_iter()
                .find(|link| link.id == hit.document.id)
            {
                set_active_view.set(AppView::Booking { slug: link.slug });
            }
        }
        SearchKind::Meeting | SearchKind::Transcript => {
            set_active_view.set(AppView::MeetingDetail {
                meeting_id: hit.document.id,
            });
        }
        SearchKind::MeetingNote => set_active_view.set(AppView::MeetingNoteDetail {
            note_id: hit.document.id,
        }),
        SearchKind::Person => {
            selected_person.set(Some(hit.document.id));
            set_active_view.set(AppView::People);
        }
        SearchKind::Document => {
            let is_legacy_capture = captures
                .with_untracked(|cs| cs.iter().any(|capture| capture.id == hit.document.id));
            if is_legacy_capture {
                set_capture_edit_id.set(Some(hit.document.id));
                set_capture_open.set(true);
            } else {
                set_active_view.set(AppView::DocumentDetail {
                    document_id: hit.document.id,
                });
            }
        }
        SearchKind::Conversation => {
            let is_conversation = conversations.with_untracked(|cs| {
                cs.iter()
                    .any(|conversation| conversation.id == hit.document.id)
            });
            if is_conversation {
                set_active_view.set(AppView::ConversationDetail {
                    conversation_id: hit.document.id,
                });
            } else {
                set_capture_edit_id.set(Some(hit.document.id));
                set_capture_open.set(true);
            }
        }
        SearchKind::Clip => set_active_view.set(AppView::ClipDetail {
            clip_id: hit.document.id,
        }),
        SearchKind::Finance => {
            selected_life.set(Some(SelectedLifeRecord {
                section: bruhs_models::LifeSection::Finance,
                id: hit.document.id,
            }));
            life_section.set(bruhs_models::LifeSection::Finance);
            set_active_view.set(AppView::Life);
        }
        SearchKind::Health => {
            selected_life.set(Some(SelectedLifeRecord {
                section: bruhs_models::LifeSection::Health,
                id: hit.document.id,
            }));
            life_section.set(bruhs_models::LifeSection::Health);
            set_active_view.set(AppView::Life);
        }
    });
    provide_context(LocalSearch {
        index,
        transcripts_ready,
        load_history,
        open: open_hit,
    });
}

#[component]
pub fn SearchPage() -> impl IntoView {
    let LocalSearch {
        index,
        transcripts_ready,
        load_history,
        open: open_hit,
    } = expect_context::<LocalSearch>();
    load_history.run(());
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let query = RwSignal::new(String::new());
    let selected = RwSignal::new(0usize);
    let hits = Memo::new(move |_| {
        query.with(|query| index.with(|index| index.search(query, SEARCH_LIMIT)))
    });
    Effect::new(move |_| {
        let last = hits.with(Vec::len).saturating_sub(1);
        if selected.get_untracked() > last {
            selected.set(last);
        }
    });
    let on_keydown = Callback::new(move |ev: web_sys::KeyboardEvent| {
        if ev.is_composing() {
            return;
        }
        let count = hits.with(Vec::len);
        match ev.key().as_str() {
            "ArrowDown" | "ArrowUp" => {
                ev.prevent_default();
                if let Some(next) = super::command_bar::move_selected_index(
                    selected.get_untracked(),
                    count,
                    &ev.key(),
                ) {
                    selected.set(next);
                    scroll_result_into_view(&format!("search-result-{next}"));
                }
            }
            "Enter" => {
                if let Some(hit) = hits.with(|hits| hits.get(selected.get_untracked()).cloned()) {
                    ev.prevent_default();
                    open_hit.run(hit);
                }
            }
            "Escape" => {
                ev.prevent_default();
                query.set(String::new());
                selected.set(0);
            }
            _ => {}
        }
    });

    view! {
        <div class="flex flex-col h-full bg-longan-950 text-rambutan-300 overflow-auto">
            <div class="max-w-3xl mx-auto w-full px-6 py-4 space-y-4">
                <header class="flex items-center justify-between gap-3">
                    <div class="flex items-center gap-3 min-w-0">
                        <button
                            type="button"
                            class="text-label text-rambutan-400 hover:text-rambutan-200 inset-ring inset-ring-longan-700 rounded-seed px-2 py-0.5 cursor-pointer"
                            on:click=move |_| set_active_view.set(AppView::Chat)
                        >
                            "← Chat"
                        </button>
                        <h1 class="text-body tracking-tight text-rambutan-100 truncate">
                            "Search everything"
                        </h1>
                    </div>
                    <span class="text-label-sm text-rambutan-500 tabular-nums">
                        {move || format!("{} indexed", index.with(|value| value.len()))}
                    </span>
                </header>

                <Input
                    input_type="search".to_string()
                    placeholder="Search tasks, notes, events, people, meetings, threads…".to_string()
                    value=Signal::derive(move || query.get())
                    aria_label="Search everything".to_string()
                    role="combobox".to_string()
                    aria_controls="search-results".to_string()
                    aria_expanded=Signal::derive(move || !hits.with(Vec::is_empty))
                    aria_activedescendant=Signal::derive(move || {
                        (!hits.with(Vec::is_empty)).then(|| format!("search-result-{}", selected.get()))
                    })
                    on_keydown=on_keydown
                    on_input=Callback::new(move |value| {
                        query.set(value);
                        selected.set(0);
                    })
                    autofocus=true
                />

                {move || {
                    let value = query.get();
                    if value.trim().is_empty() {
                        return view! {
                            <div class="rounded-grape inset-ring inset-ring-longan-800 bg-longan-900/30 px-4 py-3 text-body-sm text-rambutan-500">
                                "Find local records instantly. ↑↓ to choose · Enter to open · Esc to clear."
                            </div>
                        }.into_any();
                    }

                    let current_hits = hits.get();
                    if current_hits.is_empty() {
                        return view! {
                            <div class="text-body-sm text-rambutan-500">
                                {format!("No results for “{}”.", value.trim())}
                            </div>
                        }.into_any();
                    }

                    view! {
                        <div id="search-results" class="space-y-1" role="listbox" aria-label="Search results">
                            {current_hits.into_iter().enumerate().map(|(idx, hit)| {
                                let icon = result_icon(hit.document.kind);
                                let kind = hit.document.kind.label();
                                let title = hit.document.title.clone();
                                let subtitle = hit.document.subtitle.clone();
                                let body = preview(&hit.document.body);
                                let has_subtitle = !subtitle.is_empty();
                                let has_body = !body.is_empty();
                                let aria_label = if subtitle.is_empty() {
                                    format!("{kind}: {title}")
                                } else {
                                    format!("{kind}: {title}, {subtitle}")
                                };
                                view! {
                                    <button
                                        type="button"
                                        role="option"
                                        id=format!("search-result-{idx}")
                                        aria-selected=move || (selected.get() == idx).to_string()
                                        class:bg-longan-800=move || selected.get() == idx
                                        on:mousemove=move |event: web_sys::MouseEvent| {
                                            if event.movement_x() != 0 || event.movement_y() != 0 {
                                                selected.set(idx);
                                            }
                                        }
                                        aria-label=aria_label
                                        class="group w-full rounded-grape px-3 py-2.5 text-left inset-ring inset-ring-transparent hover:bg-longan-900/70 hover:inset-ring-longan-700 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400 transition-colors cursor-pointer"
                                        on:click=move |_| open_hit.run(hit.clone())
                                    >
                                        <span class="flex items-start gap-3 min-w-0">
                                            <span class="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-seed bg-longan-800/70 text-rambutan-400 inset-ring inset-ring-white/5 group-hover:text-rambutan-200">
                                                <Icon name=icon size=14 />
                                            </span>
                                            <span class="min-w-0 flex-1">
                                                <span class="flex items-baseline gap-2 min-w-0">
                                                    <span class="truncate text-label text-rambutan-100">{title}</span>
                                                    <span class="shrink-0 text-label-sm text-rambutan-500">{kind}</span>
                                                </span>
                                                <Show when=move || has_subtitle>
                                                    <span class="block truncate text-label-sm text-blueberry-300/80">{subtitle.clone()}</span>
                                                </Show>
                                                <Show when=move || has_body>
                                                    <span class="mt-0.5 block truncate text-body-sm text-rambutan-500">{body.clone()}</span>
                                                </Show>
                                            </span>
                                            <span class="mt-1 text-label-sm text-rambutan-500 opacity-0 group-hover:opacity-100 group-focus:opacity-100">"↵"</span>
                                        </span>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                        <Show when=move || !transcripts_ready.get()>
                            <div class="pt-2 text-label-sm text-rambutan-500" aria-live="polite">
                                "Adding transcript history…"
                            </div>
                        </Show>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_should_truncate_on_character_boundaries() {
        let source = "界".repeat(RESULT_PREVIEW_CHARS + 1);

        let value = preview(&source);

        assert_eq!(value.chars().count(), RESULT_PREVIEW_CHARS + 1);
        assert!(value.ends_with('…'));
    }

    #[test]
    fn search_icons_should_cover_every_domain() {
        for kind in [
            SearchKind::MeetingNote,
            SearchKind::Document,
            SearchKind::Clip,
            SearchKind::Conversation,
            SearchKind::SchedulingLink,
            SearchKind::Finance,
            SearchKind::Health,
        ] {
            assert!(!result_icon(kind).is_empty());
        }
    }

    #[test]
    fn capture_notes_and_drafts_should_join_search_without_domain_duplicates() {
        let captures = [
            CaptureRecord::new(CaptureKind::Note, "Launch note\nUse cohort B", 20)
                .expect("valid note"),
            CaptureRecord::new(CaptureKind::MessageDraft, "Follow up\nSend Friday", 10)
                .expect("valid draft"),
            CaptureRecord::new(CaptureKind::Task, "Already projected task", 5).expect("valid task"),
        ];
        let live_segments = BTreeMap::new();

        let documents = build_documents(SearchSources {
            threads: &[],
            work_items: &[],
            projects: &[],
            events: &[],
            scheduling: &bruhs_models::SchedulingPreferences::default(),
            life: &bruhs_models::LifeContext::default(),
            meetings: &[],
            meeting_notes: &[],
            people: &[],
            captures: &captures,
            conversations: &[],
            documents: &[],
            clips: &[],
            stored_segments: &[],
            live_segments: &live_segments,
        });

        assert_eq!(documents.len(), 2);
        assert!(documents
            .iter()
            .any(|document| document.kind == SearchKind::Document));
        assert!(documents
            .iter()
            .any(|document| document.kind == SearchKind::Conversation));
    }

    #[test]
    fn local_meeting_notes_join_web_index_with_exact_ids_and_body_text() {
        let notes = [bruhs_models::MeetingNote::new(
            "Local planning".into(),
            "Unique notequartz body".into(),
            1,
        )
        .unwrap()];
        let documents = build_documents(SearchSources {
            threads: &[],
            work_items: &[],
            projects: &[],
            events: &[],
            scheduling: &bruhs_models::SchedulingPreferences::default(),
            life: &bruhs_models::LifeContext::default(),
            meetings: &[],
            meeting_notes: &notes,
            people: &[],
            captures: &[],
            conversations: &[],
            documents: &[],
            clips: &[],
            stored_segments: &[],
            live_segments: &BTreeMap::new(),
        });
        let hits = SearchIndex::new(documents).search("notequartz", 50);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.kind, SearchKind::MeetingNote);
        assert_eq!(hits[0].document.id, notes[0].id);
        assert_eq!(hits[0].document.body, notes[0].body);
    }
}
