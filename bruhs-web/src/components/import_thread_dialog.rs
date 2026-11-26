//! Import thread: pick a thread from Claude Code, Codex or Cursor
//! and continue it here. A Claude Code or Codex thread resumes its own
//! session (and stays usable in the tool it came from); a Cursor chat comes
//! in as its transcript. See `bruhs_models::logic::portable`.
//!
//! The Export file tab brings in chats from a ChatGPT, Claude.ai or Grok
//! data export, as transcripts (`export_tab`).
//!
//! Also the thread's way back out: [`portable_messages`] feeds the
//! palette's "Copy thread as Markdown".

use std::collections::HashSet;
use std::time::Duration;

use bruhs_models::agent::Role;
use bruhs_models::imports::{import_key, EXPORT_TAB};
use bruhs_models::logic::portable::{
    import_model, ImportCandidate, ImportSource, ImportedThread, PortableMessage, PortablePart,
    PortableToolCall,
};
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{import_sessions, import_thread, list_dir};
use crate::state::{
    now_epoch_ms, ActiveView, AgentMessage, AppView, ImportThreadOpen, MessagePart, MessageStatus,
    SetActiveView, Thread, ThreadNotice, ToolCallInfo,
};
use crate::ui::Dialog;

mod export_tab;
use export_tab::{scroll_into_view_later, ExportState, ExportTab};

/// The dialog's tab: a local tool, or the Export file tab (F24). Only a
/// local tab searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportTab {
    Local(ImportSource),
    Export,
}

/// An imported message as a bruhs message.
pub fn agent_message(message: &PortableMessage) -> AgentMessage {
    let parts = message
        .parts
        .iter()
        .map(|part| match part {
            PortablePart::Text(text) => MessagePart::Text(text.clone()),
            PortablePart::Thinking(text) => MessagePart::Thinking(text.clone()),
            PortablePart::ToolCall(call) => MessagePart::ToolCall(ToolCallInfo {
                id: call.id.clone(),
                tool: call.tool.clone(),
                input: call.input.clone(),
                output: call.output.clone(),
                is_error: call.is_error,
                started_at_ms: None,
                ended_at_ms: None,
            }),
        })
        .collect();
    let at_ms = message.started_at_ms.and_then(|ms| {
        bruhs_models::EpochMs::new(ms)
            .ok()
            .map(bruhs_models::EpochMs::as_f64)
    });
    AgentMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: if message.role == Role::User {
            Role::User
        } else {
            Role::Assistant
        },
        status: MessageStatus::Finished { at_ms },
        usage: None,
        started_at_ms: at_ms,
        model: message.model.clone(),
        parts,
        error: None,
        files: Vec::new(),
        checkpoint: None,
    }
}

/// A thread's messages in the shape every exporter reads. Tool messages
/// and empty ones are left out.
pub fn portable_messages(messages: &[AgentMessage]) -> Vec<PortableMessage> {
    messages
        .iter()
        .filter(|message| matches!(message.role, Role::User | Role::Assistant))
        .map(|message| PortableMessage {
            role: message.role,
            parts: message
                .parts
                .iter()
                .map(|part| match part {
                    MessagePart::Text(text) => PortablePart::Text(text.clone()),
                    MessagePart::Thinking(text) => PortablePart::Thinking(text.clone()),
                    MessagePart::ToolCall(call) => PortablePart::ToolCall(PortableToolCall {
                        id: call.id.clone(),
                        tool: call.tool.clone(),
                        input: call.input.clone(),
                        output: call.output.clone(),
                        is_error: call.is_error,
                    }),
                })
                .collect(),
            started_at_ms: message
                .started_at_ms
                .map(bruhs_models::EpochMs::saturating_from_f64)
                .map(bruhs_models::EpochMs::as_millis),
            model: message.model.clone(),
        })
        .filter(|message| !message.parts.is_empty())
        .collect()
}

/// The bruhs thread an import becomes. A thread that ran in the home
/// directory (or nowhere known) is a chat without a project, as bruhs's own
/// home-directory chats are.
pub fn imported_thread(
    imported: &ImportedThread,
    home: Option<&str>,
    now: bruhs_models::EpochMs,
) -> Thread {
    let candidate = &imported.candidate;
    let project_path = candidate
        .cwd
        .clone()
        .filter(|cwd| !cwd.is_empty() && Some(cwd.trim_end_matches('/')) != home);
    let project_name = project_path
        .as_deref()
        .and_then(|path| path.trim_end_matches('/').rsplit('/').next())
        .unwrap_or_default()
        .to_string();
    let mut thread = Thread::new(candidate.title.clone(), project_name, project_path, now);
    thread.session_id = candidate.session_id.clone();
    thread.model = Some(import_model(candidate));
    thread.import_context = imported.context.clone();
    // A resumed session lives in its folder: a new worktree would hide it
    // from the CLI.
    thread.run_locked = candidate.session_id.is_some();
    // A chat from an export keeps its key, so it isn't offered again, and
    // sorts by when it happened (D8).
    if candidate.source.is_export() {
        thread.import_key = Some(import_key(candidate.source, &candidate.id));
        thread.updated_at = candidate.updated_at;
    }
    thread
}

/// The second line of a row: where the thread ran, how long ago, and
/// whether it resumes or comes in as a transcript.
fn row_detail(candidate: &ImportCandidate, home: Option<&str>) -> String {
    let place = match (candidate.cwd.as_deref(), home) {
        (Some(cwd), Some(home)) if cwd.starts_with(home) => format!("~{}", &cwd[home.len()..]),
        (Some(cwd), _) => cwd.to_string(),
        (None, _) => "no folder".to_string(),
    };
    let when = format_relative_ms(
        candidate.updated_at,
        now_epoch_ms(),
        RelativeTimeStyle::Compact,
    );
    let how = if candidate.session_id.is_some() {
        "resumes"
    } else {
        "transcript"
    };
    format!("{place}, {when}, {how}")
}

#[component]
pub fn ImportThreadDialog() -> impl IntoView {
    let ImportThreadOpen(open) = expect_context::<ImportThreadOpen>();
    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let ActiveView(_active_view) = expect_context::<ActiveView>();
    let thread_notice = expect_context::<ThreadNotice>();

    let tab = RwSignal::new(ImportTab::Local(ImportSource::ClaudeCode));
    let query = RwSignal::new(String::new());
    let found = RwSignal::new(Vec::<ImportCandidate>::new());
    let loading = RwSignal::new(false);
    let importing = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let picked = RwSignal::new(0usize);
    let home = RwSignal::new(None::<String>);
    // Only the newest search may fill the list.
    let generation = StoredValue::new(0u64);
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Threads already here (by session, or by import key) are not offered
    // again.
    let known_keys = Memo::new(move |_| {
        threads.with(|ts| {
            ts.iter()
                .filter_map(|thread| thread.import_key.clone())
                .collect::<HashSet<String>>()
        })
    });
    let export = ExportState::new(known_keys.into());
    provide_context(export);
    let visible = Memo::new(move |_| {
        let known: HashSet<String> = threads.with(|ts| {
            ts.iter()
                .filter_map(|thread| thread.session_id.clone())
                .collect()
        });
        known_keys.with(|keys| {
            found.with(|rows| {
                rows.iter()
                    .filter(|row| {
                        row.session_id
                            .as_ref()
                            .is_none_or(|session| !known.contains(session))
                            && !keys.contains(&import_key(row.source, &row.id))
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
        })
    });

    // The chosen tab stays in view in the scrolling strip (M5).
    Effect::new(move |_| {
        if open.get() {
            tab.track();
            scroll_into_view_later(
                "[role=tablist][aria-label=\"Import from\"] [role=tab][aria-selected=true]",
            );
        }
    });

    // Closing the dialog, by any path, discards the export's batch (F23).
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        if was_open == Some(true) && !is_open {
            export.discard();
        }
        is_open
    });

    Effect::new(move |_| {
        if open.get() {
            query.set(String::new());
            picked.set(0);
            error.set(None);
            if home.get_untracked().is_none() {
                spawn_local(async move {
                    if let Ok(listing) = list_dir(None).await {
                        home.set(Some(listing.current.trim_end_matches('/').to_string()));
                    }
                });
            }
            if let Some(input) = input_ref.get() {
                let _ = input.focus();
            }
        }
    });

    // Search on open, on a source change, and 200ms after typing stops.
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let ImportTab::Local(source) = tab.get() else {
            return;
        };
        let text = query.get();
        // Rows of another source must not stay pickable while this loads.
        if found.with_untracked(|rows| rows.first().is_some_and(|row| row.source != source)) {
            found.set(Vec::new());
        }
        let this = generation.get_value() + 1;
        generation.set_value(this);
        loading.set(true);
        let delay = if text.is_empty() { 0 } else { 200 };
        set_timeout(
            move || {
                if generation.get_value() != this {
                    return;
                }
                spawn_local(async move {
                    let result = import_sessions(source, &text).await;
                    if generation.get_value() != this {
                        return;
                    }
                    match result {
                        Ok(rows) => {
                            found.set(rows);
                            error.set(None);
                        }
                        Err(err) => {
                            found.set(Vec::new());
                            error.set(Some(err.to_string()));
                        }
                    }
                    picked.set(0);
                    loading.set(false);
                });
            },
            Duration::from_millis(delay),
        );
    });

    let close = move || open.set(false);
    let on_close = Callback::new(move |_: ()| close());

    // Every import becomes a thread the same way: persisted, then made the
    // active thread.
    let import_one = Callback::new(move |imported: ImportedThread| {
        let thread = imported_thread(&imported, home.get_untracked().as_deref(), now_epoch_ms());
        let messages: Vec<AgentMessage> = imported.messages.iter().map(agent_message).collect();
        let id = thread.id.clone();
        set_threads.update(|ts| ts.push(thread.clone()));
        catalog.persist(None, thread);
        saves.edit_conversation(id.clone(), messages.clone());
        set_active_view.set(AppView::Chat);
        set_active_thread_id.set(Some(id));
        set_messages.set(messages);
    });

    let import = move |candidate: ImportCandidate| {
        if importing.get_untracked() {
            return;
        }
        importing.set(true);
        error.set(None);
        spawn_local(async move {
            match import_thread(candidate.source, &candidate.id).await {
                Ok(imported) => {
                    import_one.run(imported);
                    thread_notice.flash(format!("Imported from {}", candidate.source.label()));
                    importing.set(false);
                    close();
                }
                Err(err) => {
                    error.set(Some(format!("Could not import: {err}")));
                    importing.set(false);
                }
            }
        });
    };

    // An export row comes in behind the dialog, which stays open on the
    // batch (F23); the tab's status line says what came in. Import all
    // queues every row it takes.
    let import_export = Callback::new(move |row: ImportCandidate| {
        if export.enqueue(row) {
            spawn_local(export.drain(import_one));
        }
    });
    let busy = Signal::derive(move || importing.get() || export.importing.get());

    let on_keydown = move |event: web_sys::KeyboardEvent| {
        let count = visible.with_untracked(Vec::len);
        match event.key().as_str() {
            "ArrowDown" | "ArrowUp" => {
                event.prevent_default();
                if let Some(next) = crate::components::command_bar::move_selected_index(
                    picked.get_untracked(),
                    count,
                    &event.key(),
                ) {
                    picked.set(next);
                }
            }
            "Enter" => {
                event.prevent_default();
                let row = visible.with_untracked(|rows| {
                    rows.get(picked.get_untracked().min(count.saturating_sub(1)))
                        .cloned()
                });
                if let Some(row) = row {
                    import(row);
                }
            }
            _ => {}
        }
    };

    // The arrow hint shows only with a row to choose (M6).
    let choosable = move || match tab.get() {
        ImportTab::Local(_) => error.with(Option::is_none) && visible.with(|rows| !rows.is_empty()),
        ImportTab::Export => export.has_rows(),
    };
    let local_source = move || match tab.get() {
        ImportTab::Local(source) => source,
        ImportTab::Export => ImportSource::ClaudeCode,
    };
    let local_rows = move || {
        view! {
            <div class="max-h-80 overflow-y-auto py-1" role="listbox" aria-label="Threads">
                {move || {
                    if let Some(message) = error.get() {
                        return view! {
                            <div class="px-4 py-6 text-center text-label-sm text-lychee-400">{message}</div>
                        }
                        .into_any();
                    }
                    let rows = visible.get();
                    if rows.is_empty() {
                        let text = if loading.get() {
                            "Loading…".to_string()
                        } else {
                            format!("No {} threads found", local_source().label())
                        };
                        return view! {
                            <div class="px-4 py-6 text-center text-label-sm text-rambutan-300">{text}</div>
                        }
                        .into_any();
                    }
                    let home = home.get();
                    let selected = picked.get().min(rows.len() - 1);
                    rows.into_iter()
                        .enumerate()
                        .map(|(index, row)| {
                            let detail = row_detail(&row, home.as_deref());
                            let title = bruhs_models::logic::text::plain_title(&row.title).into_owned();
                            let chosen = index == selected;
                            view! {
                                <button
                                    type="button"
                                    role="option"
                                    aria-selected=chosen.to_string()
                                    on:mouseenter=move |_| picked.set(index)
                                    on:click=move |_| import(row.clone())
                                    class=if chosen {
                                        "flex w-full flex-col items-start gap-0.5 bg-longan-700/70 px-4 py-2 text-left cursor-pointer"
                                    } else {
                                        "flex w-full flex-col items-start gap-0.5 px-4 py-2 text-left hover:bg-longan-700/40 cursor-pointer"
                                    }
                                >
                                    <span class="w-full truncate text-label-sm text-rambutan-200">{title}</span>
                                    <span class="w-full truncate text-label-sm text-rambutan-400">{detail}</span>
                                </button>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        }
    };

    view! {
        <Dialog open=open on_close=on_close max_width="max-w-lg".to_string() label="Import thread" anchor_top=true>
            <div class="border-b border-longan-800 px-4 py-3">
                <h2 class="text-label-sm text-rambutan-200">"Import thread"</h2>
                <div class="mt-2 flex gap-1 overflow-x-auto" role="tablist" aria-label="Import from">
                    {ImportSource::LOCAL
                        .into_iter()
                        .map(ImportTab::Local)
                        .chain([ImportTab::Export])
                        .map(|each| {
                            let label = match each {
                                ImportTab::Local(source) => source.label(),
                                ImportTab::Export => EXPORT_TAB,
                            };
                            view! {
                                <button
                                    type="button"
                                    role="tab"
                                    aria-selected=move || (tab.get() == each).to_string()
                                    on:click=move |_| tab.set(each)
                                    class=move || {
                                        if tab.get() == each {
                                            "bruhs-control shrink-0 whitespace-nowrap rounded-grape bg-blueberry-600/20 px-2.5 py-1 text-label-sm text-blueberry-200 inset-ring inset-ring-blueberry-500/30 cursor-pointer"
                                        } else {
                                            "bruhs-control shrink-0 whitespace-nowrap rounded-grape px-2.5 py-1 text-label-sm text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-200 transition-colors cursor-pointer"
                                        }
                                    }
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
                <Show when=move || tab.get() != ImportTab::Export>
                    <input
                        node_ref=input_ref
                        // The Dialog focuses this, not the first source tab, so
                        // typing searches at once.
                        data-autofocus=""
                        type="text"
                        placeholder="Search titles and folders"
                        aria-label="Search threads"
                        prop:value=move || query.get()
                        on:input=move |event| query.set(event_target_value(&event))
                        on:keydown=on_keydown
                        class="mt-2 w-full rounded-grape bg-longan-900 px-2.5 py-1.5 text-label-sm text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-longan-700 focus:outline-none focus:inset-ring-persimmon-500/60"
                    />
                </Show>
            </div>

            <Show
                when=move || tab.get() == ImportTab::Export
                fallback=local_rows
            >
                <ExportTab on_import=import_export busy=busy />
            </Show>

            <div class="flex items-center justify-between border-t border-longan-800/50 px-4 py-2.5 text-label-sm text-rambutan-300">
                <span>
                    {move || if busy.get() {
                        "Importing…"
                    } else if choosable() {
                        "↑↓ to choose, enter to import"
                    } else {
                        ""
                    }}
                </span>
                {cancel_button(on_close)}
            </div>
        </Dialog>
    }
}

/// The footer's Cancel: a 44 px target on touch, like the dialog's other
/// controls.
fn cancel_button(close: Callback<()>) -> impl IntoView {
    view! {
        <button
            type="button"
            on:click=move |_| close.run(())
            class="bruhs-control text-rambutan-500 hover:text-rambutan-300 transition-colors cursor-pointer"
        >
            "Cancel"
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::EpochMs;

    #[test]
    fn cancel_is_a_touch_sized_target() {
        // `bruhs-control` is 44 px tall and wide on a coarse pointer, as
        // every other control in the dialog is (ui standards).
        let html = Owner::new().with(|| {
            leptos::tachys::view::RenderHtml::to_html(cancel_button(Callback::new(|()| {})))
        });
        assert!(html.contains(">Cancel<"), "{html}");
        assert!(html.contains("bruhs-control"), "{html}");
    }

    fn imported(cwd: Option<&str>, session: Option<&str>) -> ImportedThread {
        ImportedThread::new(
            ImportCandidate {
                source: ImportSource::ClaudeCode,
                id: "id".into(),
                title: "Paging fix".into(),
                cwd: cwd.map(str::to_string),
                updated_at: EpochMs::UNIX_EPOCH,
                provider: session.map(|_| AgentProvider::Claude),
                session_id: session.map(str::to_string),
                model: Some("claude-opus-5-5".into()),
            },
            vec![PortableMessage {
                role: Role::User,
                parts: vec![PortablePart::Text("hi".into())],
                started_at_ms: Some(1_000),
                model: None,
            }],
        )
    }

    #[test]
    fn imported_threads_carry_their_key_and_time() {
        let at = EpochMs::saturating_from_i64(1_600_000_000_000);
        let export = ImportedThread::new(
            ImportCandidate {
                source: ImportSource::ChatGpt,
                id: "abc".into(),
                title: "Trip plan".into(),
                cwd: None,
                updated_at: at,
                provider: None,
                session_id: None,
                model: None,
            },
            vec![PortableMessage {
                role: Role::User,
                parts: vec![PortablePart::Text("Where to?".into())],
                started_at_ms: Some(1_600_000_000_000),
                model: None,
            }],
        );
        let now = EpochMs::saturating_from_i64(1_700_000_000_000);
        let thread = imported_thread(&export, Some("/Users/me"), now);
        assert_eq!(thread.import_key.as_deref(), Some("chatgpt:abc"));
        assert_eq!(
            thread.updated_at, at,
            "an old chat sorts by when it happened"
        );
        assert_eq!(thread.project_path, None);
        assert_eq!(thread.session_id, None);
        assert!(!thread.run_locked);
        assert!(export.context.is_some());
        assert_eq!(thread.import_context, export.context);

        // A local tool's thread keeps today's fields.
        let local = imported_thread(
            &imported(Some("/Users/me/shop"), Some("s-1")),
            Some("/Users/me"),
            now,
        );
        assert_eq!(local.import_key, None);
        assert_eq!(local.updated_at, now);
        assert_eq!(local.session_id.as_deref(), Some("s-1"));
        assert!(local.run_locked);
    }

    #[test]
    fn an_import_keeps_its_session_project_and_model() {
        let thread = imported_thread(
            &imported(Some("/Users/me/shop"), Some("s-1")),
            Some("/Users/me"),
            EpochMs::UNIX_EPOCH,
        );
        assert_eq!(thread.title, "Paging fix");
        assert_eq!(thread.project_path.as_deref(), Some("/Users/me/shop"));
        assert_eq!(thread.project_name, "shop");
        assert_eq!(thread.session_id.as_deref(), Some("s-1"));
        assert_eq!(thread.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(thread.import_context, None);
        assert!(thread.run_locked, "a resumed session stays in its folder");
    }

    #[test]
    fn a_home_directory_thread_becomes_a_chat_without_a_project() {
        let thread = imported_thread(
            &imported(Some("/Users/me/"), Some("s-1")),
            Some("/Users/me"),
            EpochMs::UNIX_EPOCH,
        );
        assert_eq!(thread.project_path, None);
        assert_eq!(thread.project_name, "");
    }

    #[test]
    fn a_transcript_import_carries_its_context() {
        let thread = imported_thread(&imported(None, None), None, EpochMs::UNIX_EPOCH);
        assert!(thread.session_id.is_none());
        assert!(thread
            .import_context
            .as_deref()
            .is_some_and(|c| c.contains("hi")));
    }

    #[test]
    fn messages_round_trip_through_the_portable_shape() {
        let portable = PortableMessage {
            role: Role::Assistant,
            parts: vec![
                PortablePart::Thinking("plan".into()),
                PortablePart::ToolCall(PortableToolCall {
                    id: "t".into(),
                    tool: "Bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                    output: Some("a.txt".into()),
                    is_error: false,
                }),
                PortablePart::Text("done".into()),
            ],
            started_at_ms: Some(1_700_000_000_000),
            model: Some("claude-haiku-5-5".into()),
        };
        let message = agent_message(&portable);
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.started_at_ms, Some(1_700_000_000_000.0));
        assert_eq!(portable_messages(&[message]), vec![portable]);
    }
}
