//! Thread sidebar on the panel tone: search and a
//! project filter up top, three-line thread cards (project, title, then
//! branch and provider), in-flight and recent threads first, everything
//! older and already seen folded under "Settled".

mod viewport;
mod window;

use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::provider::{
    provider_agent_label_for_model, provider_logo_src_for_model, PROVIDER_LOGO_CLASS,
};
use crate::format::relative_time;
use crate::state::{
    activate_thread, apply_drag, create_thread, effect_for, groups_for, instructions_text,
    new_thread_or_pick, set_settled, snooze_until, thread_facts, titled, wake, ActiveView, AppView,
    ComposerDraft, PendingPermissionsSignal, SetActiveView, SetProjectPickerOpen, SetSidebarOpen,
    ShelfClock, SidebarChrome, SidebarProjectFilter, SidebarUndo, StreamBufferSignal,
    StreamingThreadId, Thread, TitleInstructionsSignal, UndoEntry, UndoRestore, UnreadThreads,
    WorkspaceMode,
};
use crate::ui::{Icon, Menu, MenuAlign, MenuItem};

use bruhs_models::logic::sidebar::{
    snooze_after, snooze_presets, FoldOpen, SidebarSection, SnoozeUnit,
};

/// Idle this long (ms) with nothing unread, and a thread folds under
/// "Settled". T3's default is three days; bruhs keeps one day.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "sidebar unit tests read the one-day settle window"
    )
)]
pub const SETTLED_AFTER_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

fn clock_i64(ms: f64) -> i64 {
    bruhs_models::EpochMs::saturating_from_f64(ms).as_millis()
}

fn shelf_clock<'a>(query: &OrderQuery<'a>) -> ShelfClock<'a> {
    ShelfClock {
        now_ms: clock_i64(query.now_ms),
        active: query.active,
        working: query.working,
        needs_you: query.needs_you,
        unread: query.unread,
        working_section: query.working_section,
    }
}

/// Minutes east of UTC. `Date.getTimezoneOffset` is minutes west.
fn utc_offset_minutes_east() -> i32 {
    use bruhs_models::num::ToIntSaturating;
    let west = js_sys::Date::new_0().get_timezone_offset();
    if !west.is_finite() {
        return 0;
    }
    -west.round().to_i32_saturating()
}

/// What the shell needs in order to walk the same shelves the sidebar paints.
pub struct OrderQuery<'a> {
    pub now_ms: f64,
    pub active: Option<&'a str>,
    pub working: Option<&'a str>,
    pub needs_you: Option<&'a str>,
    pub unread: &'a HashSet<String>,
    pub working_section: bool,
    pub folds: FoldOpen,
}

/// Active, working, unread or recent threads stay up top. A pin does not
/// keep an idle thread out of Settled. ⌘⇧S still settles immediately.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "sidebar unit tests check the settle rule directly"
    )
)]
pub fn is_settled(thread: &Thread, now_ms: f64, active: bool, working: bool, unread: bool) -> bool {
    let now_ms = clock_i64(now_ms);
    bruhs_models::logic::sidebar::is_settled(
        &thread_facts(thread, now_ms, active, working, false, unread),
        now_ms,
    )
}

/// Threads in shelf order, skipping shelves whose fold is closed.
/// ⌘⇧[ ] and ⌘1–9 walk this list.
pub fn sidebar_order(threads: &[Thread], query: &OrderQuery<'_>) -> Vec<Thread> {
    let groups = groups_for(threads, &shelf_clock(query));
    let ids = groups.flattened(query.folds);
    ids.into_iter()
        .filter_map(|id| threads.iter().find(|thread| thread.id == id).cloned())
        .collect()
}

#[derive(PartialEq)]
struct Shelves {
    pinned: Vec<Thread>,
    active: Vec<Thread>,
    working: Vec<Thread>,
    snoozed: Vec<Thread>,
    settled: Vec<Thread>,
}

impl Shelves {
    fn empty(&self) -> bool {
        self.pinned.is_empty()
            && self.active.is_empty()
            && self.working.is_empty()
            && self.snoozed.is_empty()
            && self.settled.is_empty()
    }
}

fn shelve(list: Vec<Thread>, searching: bool, query: &OrderQuery<'_>) -> Shelves {
    if searching {
        return Shelves {
            pinned: Vec::new(),
            active: list,
            working: Vec::new(),
            snoozed: Vec::new(),
            settled: Vec::new(),
        };
    }
    let groups = groups_for(&list, &shelf_clock(query));
    Shelves {
        pinned: take_ids(&list, &groups.pinned),
        active: take_ids(&list, &groups.active),
        working: take_ids(&list, &groups.working),
        snoozed: take_ids(&list, &groups.snoozed),
        settled: take_ids(&list, &groups.settled),
    }
}

fn take_ids(list: &[Thread], ids: &[String]) -> Vec<Thread> {
    ids.iter()
        .filter_map(|id| {
            list.iter()
                .find(|thread| thread.id.as_str() == id.as_str())
                .cloned()
        })
        .collect()
}

/// Epoch-ms of a thread's last activity; legacy rows ("now") sort last.
fn updated_ms(thread: &Thread) -> f64 {
    thread.updated_at.as_f64()
}

/// Threads to show: filtered by project (`Some("")` = general threads),
/// case-insensitive title search, newest activity first.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "sidebar unit tests check project and title filtering"
    )
)]
pub fn visible_threads(threads: &[Thread], filter: Option<&str>, query: &str) -> Vec<Thread> {
    matching_threads(threads.iter(), filter, query)
}

fn visible_workspace_threads(
    threads: &[Thread],
    mode: bruhs_models::SidebarTab,
    filter: Option<&str>,
    query: &str,
) -> Vec<Thread> {
    let filter = if mode == bruhs_models::SidebarTab::Chat {
        None
    } else {
        filter.filter(|name| !name.is_empty())
    };
    matching_threads(
        threads
            .iter()
            .filter(|thread| mode.matches_project(&thread.project_name)),
        filter,
        query,
    )
}

fn matching_threads<'a>(
    threads: impl Iterator<Item = &'a Thread>,
    filter: Option<&str>,
    query: &str,
) -> Vec<Thread> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<Thread> = threads
        .filter(|t| filter.is_none_or(|f| t.project_name == f))
        .filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        updated_ms(b)
            .partial_cmp(&updated_ms(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Distinct project names ordered by most recent activity.
pub fn project_names(threads: &[Thread]) -> Vec<String> {
    let mut seen: Vec<(String, f64)> = Vec::new();
    let mut indices = HashMap::new();
    for t in threads {
        if t.project_name.is_empty() {
            continue;
        }
        let ms = updated_ms(t);
        match indices.get(t.project_name.as_str()).copied() {
            Some(index) => {
                let entry: &mut (String, f64) = &mut seen[index];
                entry.1 = entry.1.max(ms);
            }
            None => {
                indices.insert(t.project_name.as_str(), seen.len());
                seen.push((t.project_name.clone(), ms));
            }
        }
    }
    seen.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    seen.into_iter().map(|(n, _)| n).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CardStatus {
    Working,
    Approval,
    Unread,
    Idle,
}

#[component]
pub fn Sidebar() -> impl IntoView {
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let set_threads = expect_context::<crate::state::SetThreadsSignal>().0;
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let set_active_thread_id = expect_context::<crate::state::SetActiveThreadId>().0;
    let set_messages = expect_context::<crate::state::SetMessagesSignal>().0;
    let PendingPermissionsSignal(pending_permissions) =
        expect_context::<PendingPermissionsSignal>();
    let StreamingThreadId(streaming_thread_id) = expect_context::<StreamingThreadId>();
    let StreamBufferSignal(stream_buffer) = expect_context::<StreamBufferSignal>();
    let UnreadThreads(unread_threads) = expect_context::<UnreadThreads>();
    let SidebarProjectFilter(project_filter) = expect_context::<SidebarProjectFilter>();
    let WorkspaceMode(mode) = expect_context::<WorkspaceMode>();
    let SetProjectPickerOpen(set_project_picker_open) = expect_context::<SetProjectPickerOpen>();
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();
    let dismiss_mobile_navigation = move || {
        let width = web_sys::window()
            .and_then(|window| window.inner_width().ok())
            .and_then(|width| width.as_f64());
        if width.is_some_and(|width| width < 640.0) {
            set_sidebar_open.set(false);
        }
    };

    let (query, set_query) = signal(String::new());
    let filter_open = RwSignal::new(false);
    Effect::new(move |_| {
        if mode.get() == bruhs_models::SidebarTab::Chat && filter_open.get_untracked() {
            filter_open.set(false);
        }
    });

    let names = Memo::new(move |_| threads.with(|ts| project_names(ts)));
    let visible = Memo::new(move |_| {
        let filter = project_filter.get();
        let q = query.get();
        let mode = mode.get();
        threads.with(|ts| visible_workspace_threads(ts, mode, filter.as_deref(), &q))
    });

    // Reset a filter that no longer matches any project (e.g. after deletes).
    Effect::new(move |_| {
        if let Some(f) = project_filter.get() {
            if !f.is_empty() && !names.with(|n| n.contains(&f)) {
                project_filter.set(None);
            }
        }
    });

    let history = expect_context::<crate::thread_history::ThreadHistory>();
    let select_thread = move |id: String| {
        dismiss_mobile_navigation();
        activate_thread(
            history,
            active_thread_id,
            id,
            set_active_thread_id,
            set_messages,
            stream_buffer,
            unread_threads,
        );
        // A thread always opens in the chat, whatever view was showing.
        // Only write when it changes: a same-value set still notifies and
        // would remount the chat page.
        if active_view.get_untracked() != AppView::Chat {
            set_active_view.set(AppView::Chat);
        }
    };

    // Footer view switcher (icon row).
    let nav_icon = move |view: AppView, icon: &'static str, label: &'static str| {
        let on_view = view.clone();
        let is_on = Signal::derive(move || active_view.get() == on_view);
        view! {
            <button
                type="button"
                aria-label=label
                title=label
                aria-pressed=move || is_on.get().to_string()
                class=move || format!(
                    "inline-flex size-8 items-center justify-center rounded-grape transition-colors {}",
                    if is_on.get() {
                        "bg-longan-800/60 text-rambutan-100"
                    } else {
                        "text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-100"
                    }
                )
                on:click=move |_| {
                    set_active_view.set(view.clone());
                    dismiss_mobile_navigation();
                }
            >
                <Icon name=icon size=16 />
            </button>
        }
    };

    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let delete_thread = move |id: String| {
        let Some(original) =
            threads.with_untracked(|ts| ts.iter().find(|thread| thread.id == id).cloned())
        else {
            return;
        };
        set_threads.update(|ts| ts.retain(|t| t.id != id));
        unread_threads.update(|s| {
            s.remove(&id);
        });
        if active_thread_id.get_untracked().as_deref() == Some(id.as_str()) {
            set_active_thread_id.set(None);
            set_messages.set(vec![]);
        }
        catalog.forget(original);
    };

    // New thread in the current project; the folder-plus button
    // and the command bar pick a different project.
    let new_thread = move || {
        dismiss_mobile_navigation();
        if mode.get_untracked() == bruhs_models::SidebarTab::Chat {
            create_thread(
                catalog,
                set_threads,
                set_active_thread_id,
                set_messages,
                String::new(),
                None,
            );
            if active_view.get_untracked() != AppView::Chat {
                set_active_view.set(AppView::Chat);
            }
            return;
        }
        let current = active_thread_id.get_untracked().and_then(|id| {
            threads.with_untracked(|ts| {
                ts.iter()
                    .find(|t| t.id == id && !t.project_name.is_empty())
                    .map(|t| (t.project_name.clone(), t.project_path.clone()))
            })
        });
        match current {
            Some((name, path)) => {
                create_thread(
                    catalog,
                    set_threads,
                    set_active_thread_id,
                    set_messages,
                    name,
                    path,
                );
            }
            None => new_thread_or_pick(
                catalog,
                true,
                set_threads,
                set_active_thread_id,
                set_messages,
                set_project_picker_open,
            ),
        }
        if active_view.get_untracked() != AppView::Chat {
            set_active_view.set(AppView::Chat);
        }
    };

    let filter_label = Signal::derive(move || match project_filter.get() {
        None => "All projects".to_string(),
        Some(f) if f.is_empty() => "All projects".to_string(),
        Some(f) => f,
    });

    // Shelves. A search shows every match in one list.
    let SidebarChrome {
        working_open,
        snoozed_open,
        settled_open,
        working_section,
    } = expect_context::<SidebarChrome>();
    let sidebar_undo = expect_context::<SidebarUndo>();
    let TitleInstructionsSignal(title_instructions) = expect_context::<TitleInstructionsSignal>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let messages = expect_context::<crate::state::MessagesSignal>().0;
    let dragging = RwSignal::new(None::<String>);
    let menu_thread = RwSignal::new(None::<String>);
    let menu_x = RwSignal::new(0.0_f64);
    let menu_y = RwSignal::new(0.0_f64);
    let custom_for = RwSignal::new(None::<String>);
    let custom_when = RwSignal::new(String::new());
    let custom_amount = RwSignal::new(String::new());
    let custom_unit = RwSignal::new("hours".to_string());
    let shelves = Memo::new(move |_| {
        let list = visible.get();
        let searching = query.with(|q| !q.trim().is_empty());
        let now = js_sys::Date::now();
        let active = active_thread_id.get();
        let streaming = streaming_thread_id.get();
        let needs_you = streaming
            .clone()
            .filter(|_| pending_permissions.with(|pending| !pending.is_empty()));
        unread_threads.with(|unread| {
            shelve(
                list,
                searching,
                &OrderQuery {
                    now_ms: now,
                    active: active.as_deref(),
                    working: streaming.as_deref(),
                    needs_you: needs_you.as_deref(),
                    unread,
                    working_section: working_section.get(),
                    folds: FoldOpen {
                        working: working_open.get(),
                        snoozed: snoozed_open.get(),
                        settled: settled_open.get(),
                    },
                },
            )
        })
    });
    let layout = Memo::new(move |_| {
        shelves.with(|shelves| {
            window::Layout::sections(
                &shelves.pinned,
                &shelves.active,
                &shelves.working,
                working_open.get(),
                &shelves.snoozed,
                snoozed_open.get(),
                &shelves.settled,
                settled_open.get(),
                dragging.get().is_some(),
            )
        })
    });
    let viewport = viewport::Viewport::new();
    let focused = RwSignal::new(None::<String>);
    let requested_focus = RwSignal::new(None::<String>);
    let indices = Memo::new(move |_| {
        threads.with(|rows| {
            rows.iter()
                .enumerate()
                .map(|(i, row)| (row.id.clone(), i))
                .collect::<HashMap<_, _>>()
        })
    });
    let slots = Memo::new(move |_| {
        let top = viewport.top.get();
        let height = viewport.height.get();
        let focused = focused.get();
        let requested = requested_focus.get();
        layout.with(|layout| {
            let mut slots = layout.slots(top, height);
            // Wheel scrolling must not unmount the keyboard's current focus target.
            for id in [focused, requested].into_iter().flatten() {
                let present = slots.iter().any(|slot| match slot {
                    window::Slot::Thread(row) => row.id == id,
                    window::Slot::Header(_) => false,
                });
                if !present {
                    if let Some(row) = layout.row(&id).cloned() {
                        slots.push(window::Slot::Thread(row));
                    }
                }
            }
            slots.sort_by(|left, right| left.top().total_cmp(&right.top()));
            slots
        })
    });
    // Thread activation republishes the destination mode even when unchanged.
    // Only an actual filter change should reset a user's scroll position.
    let viewport_filter = Memo::new(move |_| (query.get(), mode.get(), project_filter.get()));
    Effect::new(move |_| {
        viewport_filter.with(|_| ());
        viewport.scroll(0.0);
    });
    Effect::new(move |_| {
        let Some(id) = active_thread_id.get() else {
            return;
        };
        shelves.with_untracked(|shelves| {
            if !settled_open.get_untracked() && shelves.settled.iter().any(|thread| thread.id == id)
            {
                settled_open.set(true);
            }
            if !working_open.get_untracked() && shelves.working.iter().any(|thread| thread.id == id)
            {
                working_open.set(true);
            }
            if !snoozed_open.get_untracked() && shelves.snoozed.iter().any(|thread| thread.id == id)
            {
                snoozed_open.set(true);
            }
        });
        let focus_at_activation = focused.get_untracked();
        request_animation_frame(move || {
            // The sidebar can close while this frame is queued. Its app-owned
            // active ID outlives the sidebar's layout and viewport signals.
            if viewport.node.try_get_untracked().flatten().is_none() {
                return;
            }
            if active_thread_id.try_get_untracked().flatten().as_deref() != Some(&id) {
                return;
            }
            // A later navigation key owns the viewport, whether its focus
            // transfer is still pending or has already completed. Replaying
            // this older reveal can move the virtual window under that focus.
            if requested_focus.try_get_untracked().flatten().is_some()
                || focused.try_get_untracked().flatten() != focus_at_activation
            {
                return;
            }
            let Some(top) = layout.with_untracked(|layout| {
                layout.reveal(
                    &id,
                    viewport.top.get_untracked(),
                    viewport.height.get_untracked(),
                )
            }) else {
                return;
            };
            viewport.scroll(top);
        });
    });

    let persist_changed = move |before: &[Thread], after: &[Thread]| {
        for (original, updated) in before.iter().zip(after.iter()) {
            if original != updated {
                catalog.persist(Some(original.clone()), updated.clone());
            }
        }
    };
    let push_thread_undo = move |previous: Thread, message: &str| {
        sidebar_undo.push(UndoEntry {
            message: message.to_string(),
            restore: UndoRestore::Thread(Box::new(previous)),
        });
    };
    let commit_drop = move |dragged_id: String, onto: SidebarSection, index: usize| {
        dragging.set(None);
        let now = clock_i64(js_sys::Date::now());
        let active = active_thread_id.get_untracked();
        let streaming = streaming_thread_id.get_untracked();
        let needs_you = streaming
            .clone()
            .filter(|_| pending_permissions.with_untracked(|pending| !pending.is_empty()));
        let working_on = working_section.get_untracked();
        let mut undo: Option<(Thread, &'static str)> = None;
        set_threads.update(|threads| {
            let unread = unread_threads.get_untracked();
            let clock = ShelfClock {
                now_ms: now,
                active: active.as_deref(),
                working: streaming.as_deref(),
                needs_you: needs_you.as_deref(),
                unread: &unread,
                working_section: working_on,
            };
            let Some((effect, groups)) = effect_for(threads, &dragged_id, onto, index, &clock)
            else {
                return;
            };
            if matches!(effect, bruhs_models::logic::sidebar::DragEffect::None) {
                return;
            }
            let snapshot = threads
                .iter()
                .find(|thread| thread.id == dragged_id)
                .cloned();
            let before = threads.clone();
            apply_drag(threads, &dragged_id, effect, &groups);
            persist_changed(&before, threads);
            if let Some(previous) = snapshot {
                let message = match effect {
                    bruhs_models::logic::sidebar::DragEffect::UnpinAt(_) => "Thread unpinned",
                    bruhs_models::logic::sidebar::DragEffect::Settle => "Thread settled",
                    _ => "",
                };
                if !message.is_empty() {
                    undo = Some((previous, message));
                }
            }
        });
        if let Some((previous, message)) = undo {
            push_thread_undo(previous, message);
        }
    };
    let drop_id = move |ev: web_sys::DragEvent| -> Option<String> {
        ev.data_transfer()
            .and_then(|data| data.get_data("text/plain").ok())
            .filter(|id| !id.is_empty())
            .or_else(|| dragging.get_untracked())
    };

    // One thread card. Derived signals are `Copy`, so the closures below
    // share them without moving owned ids.
    let card = move |id: String| -> AnyView {
        let id_active = id.clone();
        let id_status = id.clone();
        let id_select = id.clone();
        let id_delete = id.clone();
        let id_live = id.clone();
        let id_position = id.clone();
        let id_focus = id.clone();
        let id_key = id.clone();
        let position = Memo::new(move |_| layout.with(|layout| layout.row(&id_position).cloned()));
        let id_aria = id.clone();
        let is_active =
            Signal::derive(move || active_thread_id.get().as_deref() == Some(id_active.as_str()));
        let status = Signal::derive(move || {
            let streaming = streaming_thread_id.get().as_deref() == Some(id_status.as_str());
            if streaming && pending_permissions.with(|p| !p.is_empty()) {
                CardStatus::Approval
            } else if streaming {
                CardStatus::Working
            } else if unread_threads.with(|s| s.contains(&id_status)) {
                CardStatus::Unread
            } else {
                CardStatus::Idle
            }
        });
        // Title/project/time/branch/model re-read live so renames and
        // activity stamps update in place.
        let live = Memo::new(move |_| {
            indices.with(|index| {
                index
                    .get(&id_live)
                    .and_then(|&i| threads.with(|ts| ts.get(i).cloned()))
            })
        });
        let title = Signal::derive(move || live.get().map(|t| t.title).unwrap_or_default());
        let project = Signal::derive(move || {
            live.get()
                .map(|t| {
                    if t.project_name.is_empty() {
                        "Chat".to_string()
                    } else {
                        t.project_name
                    }
                })
                .unwrap_or_default()
        });
        let time = Signal::derive(move || {
            live.get()
                .map(|t| match t.last_active() {
                    Some(at) => relative_time(at.as_f64(), js_sys::Date::now()),
                    // Legacy rows without a time stored the word "now".
                    None => "now".to_string(),
                })
                .unwrap_or_default()
        });
        let branch = Signal::derive(move || live.get().and_then(|t| t.branch));
        let model = Signal::derive(move || live.get().and_then(|t| t.model));
        let suppress_click = StoredValue::new(false);
        let drag_id = id.clone();
        let drop_card_id = id.clone();
        let menu_id = id.clone();
        let card_class = move || {
            let base = "w-full rounded-lychee px-2.5 py-2.5 text-left transition-colors";
            let unread = status.get() == CardStatus::Unread;
            if is_active.get() {
                format!("{base} bg-longan-800 text-rambutan-100 inset-ring inset-ring-white/10")
            } else if unread {
                format!("{base} text-rambutan-100 hover:bg-longan-800/40")
            } else {
                format!("{base} text-rambutan-200 hover:bg-longan-800/40")
            }
        };
        view! {
            <li class="group/row absolute left-2 right-2 list-none"
                style=move || position.with(|row| row.as_ref().map(|row| format!("top:{}px;height:{}px", row.top, row.height)).unwrap_or_default())
                aria-posinset=move || layout.with(|layout| layout.position(&id_aria))
                aria-setsize=move || layout.with(|layout| layout.count())
                on:focusin=move |_| { focused.set(Some(id_focus.clone())); requested_focus.set(None); }
            >
                <button
                    type="button"
                    data-thread-id=id
                    draggable="true"
                    data-sidebar-section={
                        let value = drag_id.clone();
                        move || layout.with(|layout| layout.row(&value).map_or("active", |row| window::section_key(row.section)))
                    }
                    aria-describedby="thread-navigation-help"
                    class=card_class
                    aria-current=move || if is_active.get() { "page" } else { "false" }
                    on:dragstart=move |ev: web_sys::DragEvent| {
                        suppress_click.set_value(true);
                        if let Some(data) = ev.data_transfer() {
                            let _ = data.set_data("text/plain", &drag_id);
                            data.set_effect_allowed("move");
                        }
                        dragging.set(Some(drag_id.clone()));
                    }
                    on:dragend=move |_| dragging.set(None)
                    on:dragover=move |ev: web_sys::DragEvent| {
                        ev.prevent_default();
                        if let Some(data) = ev.data_transfer() {
                            data.set_drop_effect("move");
                        }
                    }
                    on:drop=move |ev: web_sys::DragEvent| {
                        ev.prevent_default();
                        let Some(dragged) = drop_id(ev) else { return };
                        let Some((section, index)) = layout.with_untracked(|layout| {
                            layout.row(&drop_card_id).map(|row| (row.section, row.index))
                        }) else { return };
                        commit_drop(dragged, section, index);
                    }
                    on:click=move |_| {
                        if suppress_click.get_value() {
                            suppress_click.set_value(false);
                            return;
                        }
                        select_thread(id_select.clone());
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.meta_key() || ev.ctrl_key() || ev.alt_key() { return; }
                        let pending = requested_focus.get_untracked();
                        let target = layout.with_untracked(|layout| layout.navigate(pending.as_deref().unwrap_or(&id_key), &ev.key()).map(str::to_owned));
                        let Some(target) = target else { return };
                        ev.prevent_default();
                        // Keep the actual focused row mounted until the frame
                        // transfers focus; rapid following keys still need it.
                        requested_focus.set(Some(target.clone()));
                        if let Some(top) = layout.with_untracked(|layout| layout.reveal(&target, viewport.top.get_untracked(), viewport.height.get_untracked())) { viewport.scroll(top); }
                        request_animation_frame(move || {
                            use wasm_bindgen::JsCast;
                            let Some(list) = viewport.node.try_get_untracked().flatten() else { return };
                            // A newer key or user focus choice wins over a queued
                            // frame, including focus outside the thread list.
                            if requested_focus.try_get_untracked().flatten().as_deref() != Some(&target)
                                || document().active_element().is_some_and(|element| element.tag_name() != "BODY" && !list.contains(Some(&element)))
                            { return; }
                            // Compare attributes rather than interpolating user-owned IDs into selectors.
                            if let Ok(buttons) = list.query_selector_all("button[data-thread-id]") {
                                for i in 0..buttons.length() {
                                    if let Some(button) = buttons.item(i).and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok()) {
                                        if button.get_attribute("data-thread-id").as_deref() == Some(&target) { let _ = button.focus(); requested_focus.set(None); break; }
                                    }
                                }
                            }
                        });
                    }
                >
                    <span class="flex h-5 items-center gap-1.5">
                        <Icon name="folder" size=14 class="shrink-0 text-rambutan-500" />
                        <span class="min-w-0 flex-1 truncate text-label-sm text-rambutan-500">{move || project.get()}</span>
                        {move || live.get().is_some_and(|t| t.pinned).then(|| view! {
                            <Icon name="pin" size=12 class="shrink-0 text-rambutan-500" />
                        })}
                        <span class="ml-auto flex h-5 shrink-0 items-center justify-end text-label-sm tabular-nums text-rambutan-500 group-hover/row:invisible group-focus-within/row:invisible">
                            {move || match status.get() {
                                CardStatus::Working => view! {
                                    <span class="size-[9px] rounded-orb bg-blueberry-400 animate-status-pulse" role="img" aria-label="Working"></span>
                                }.into_any(),
                                CardStatus::Approval => view! {
                                    <span class="size-[9px] rounded-orb bg-durian-400" role="img" aria-label="Waiting for approval"></span>
                                }.into_any(),
                                CardStatus::Unread => view! {
                                    <span class="size-[9px] rounded-orb bg-pandan-400" role="img" aria-label="Ready"></span>
                                }.into_any(),
                                CardStatus::Idle => view! { <span>{move || time.get()}</span> }.into_any(),
                            }}
                        </span>
                    </span>
                    <span class="mt-1 block truncate text-label">{move || title.get()}</span>
                    {move || {
                        let b = branch.get();
                        let m = model.get();
                        (b.is_some() || m.is_some()).then(|| view! {
                            <span class="mt-1 flex h-4 items-center gap-1.5 text-label-sm text-rambutan-500">
                                {match b {
                                    Some(b) => {
                                        let title = b.clone();
                                        view! {
                                            <Icon name="git-branch" size=12 class="shrink-0" />
                                            <span class="min-w-0 flex-1 truncate" title=title>{b}</span>
                                        }.into_any()
                                    }
                                    None => view! { <span class="flex-1"></span> }.into_any(),
                                }}
                                {m.map(|m| view! {
                                    <img
                                        src=provider_logo_src_for_model(&m)
                                        alt=provider_agent_label_for_model(&m)
                                        title=provider_agent_label_for_model(&m)
                                        class=PROVIDER_LOGO_CLASS
                                    />
                                })}
                            </span>
                        })
                    }}
                </button>
                // Sibling (not nested) so the card stays a single button.
                <button
                    type="button"
                    aria-label="Thread actions"
                    title="Thread actions"
                    class="absolute right-8 top-2 hidden size-6 items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100 group-hover/row:inline-flex group-focus-within/row:inline-flex"
                    on:click=move |ev: web_sys::MouseEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        if let Some(target) = ev.current_target() {
                            if let Some(element) = target.dyn_ref::<web_sys::Element>() {
                                let rect = element.get_bounding_client_rect();
                                menu_x.set(rect.left());
                                menu_y.set(rect.bottom() + 4.0);
                            }
                        }
                        menu_thread.set(Some(menu_id.clone()));
                    }
                >
                    <Icon name="ellipsis" size=12 />
                </button>
                <button
                    type="button"
                    aria-label="Delete thread"
                    title="Delete thread"
                    class="absolute right-2 top-2 hidden size-6 items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-lychee-400 group-hover/row:inline-flex group-focus-within/row:inline-flex"
                    on:click=move |ev| {
                        ev.stop_propagation();
                        delete_thread(id_delete.clone());
                    }
                >
                    <Icon name="x" size=12 />
                </button>
            </li>
        }
        .into_any()
    };

    let shelf_header = move |header: window::Header| -> AnyView {
        let section = header.section;
        // The list reuses a header while threads move around it. Read the
        // current top and count from the layout instead of the first paint.
        let label = Signal::derive(move || {
            let count = layout.with(|layout| {
                layout
                    .headers
                    .iter()
                    .find(|header| header.section == section)
                    .map_or(0, |header| header.count)
            });
            match section {
                SidebarSection::Pinned => "Pinned".to_string(),
                SidebarSection::Active => "Active".to_string(),
                SidebarSection::Working => format!("Working ({count})"),
                SidebarSection::Snoozed => format!("Snoozed ({count})"),
                SidebarSection::Settled => format!("Settled ({count})"),
            }
        });
        let fold = match section {
            SidebarSection::Working => Some(working_open),
            SidebarSection::Snoozed => Some(snoozed_open),
            SidebarSection::Settled => Some(settled_open),
            SidebarSection::Pinned | SidebarSection::Active => None,
        };
        let open = Signal::derive(move || fold.is_none_or(|signal| signal.get()));
        let drop_section = section;
        view! {
            <li
                class="absolute left-2 right-2 list-none px-1 pt-2"
                data-shelf=window::section_key(section)
                style=move || {
                    layout.with(|layout| {
                        layout
                            .headers
                            .iter()
                            .find(|header| header.section == section)
                            .map_or_else(
                                || format!("height:{}px", window::HEADER_HEIGHT),
                                |header| {
                                    format!(
                                        "top:{}px;height:{}px",
                                        header.top,
                                        window::HEADER_HEIGHT
                                    )
                                },
                            )
                    })
                }
                on:dragover=move |ev: web_sys::DragEvent| {
                    ev.prevent_default();
                }
                on:drop=move |ev: web_sys::DragEvent| {
                    ev.prevent_default();
                    let Some(dragged) = drop_id(ev) else { return };
                    commit_drop(dragged, drop_section, 0);
                }
            >
                {match fold {
                    Some(signal) => view! {
                        <button
                            type="button"
                            class="flex min-h-11 w-full items-center gap-2 py-1 text-label-sm text-rambutan-500 transition-colors hover:text-rambutan-300"
                            aria-expanded=move || open.get().to_string()
                            on:click=move |_| signal.update(|value| *value = !*value)
                        >
                            <span class="shrink-0">{move || label.get()}</span>
                            <span class="h-px flex-1 bg-longan-800/80"></span>
                            {move || {
                                let class = format!(
                                    "shrink-0 transition-transform duration-200 {}",
                                    if open.get() { "rotate-180" } else { "" }
                                );
                                view! { <Icon name="chevron-down" size=12 class=class /> }
                            }}
                        </button>
                    }.into_any(),
                    None => view! {
                        <div class="flex min-h-11 w-full items-center gap-2 py-1 text-label-sm text-rambutan-500">
                            <span class="shrink-0">{move || label.get()}</span>
                            <span class="h-px flex-1 bg-longan-800/80"></span>
                        </div>
                    }.into_any(),
                }}
            </li>
        }
        .into_any()
    };

    let snooze_thread_until = move |id: String, until: i64| {
        menu_thread.set(None);
        let mut previous = None;
        set_threads.update(|threads| {
            if let Some(thread) = threads.iter_mut().find(|thread| thread.id == id) {
                let original = thread.clone();
                snooze_until(thread, until);
                catalog.persist(Some(original.clone()), thread.clone());
                previous = Some(original);
            }
        });
        if let Some(previous) = previous {
            push_thread_undo(previous, "Thread snoozed");
        }
    };
    fn pin_on(thread: &mut Thread) {
        crate::state::set_pinned(thread, true);
    }
    fn pin_off(thread: &mut Thread) {
        crate::state::set_pinned(thread, false);
    }
    fn settle_on(thread: &mut Thread) {
        set_settled(thread, true);
    }
    fn settle_off(thread: &mut Thread) {
        set_settled(thread, false);
    }
    fn wake_on(thread: &mut Thread) {
        wake(thread);
    }
    let mutate_thread =
        move |id: String, change: fn(&mut Thread), undo_message: Option<&'static str>| {
            menu_thread.set(None);
            let mut previous = None;
            set_threads.update(|threads| {
                if let Some(thread) = threads.iter_mut().find(|thread| thread.id == id) {
                    let original = thread.clone();
                    change(thread);
                    catalog.persist(Some(original.clone()), thread.clone());
                    previous = Some(original);
                }
            });
            if let (Some(previous), Some(message)) = (previous, undo_message) {
                push_thread_undo(previous, message);
            }
        };

    view! {
        <nav
            aria-label="Threads"
            class="flex h-full w-[272px] shrink-0 flex-col bg-longan-900 inset-ring inset-ring-white/5 text-rambutan-300"
        >
            // Chrome header
            <div class="flex h-[52px] shrink-0 items-center gap-2 px-4">
                <img src="/bruhs.svg" alt="" aria-hidden="true" class="bruhs-brand-logo" />
                <span class="text-label text-rambutan-100">"bruhs.ai"</span>
            </div>

            // Fixed control group: search + new thread, then the
            // project filter + new project session.
            <div class="flex flex-col gap-1 px-2 pb-2">
                <div class="flex items-center gap-1">
                    <label class="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-grape px-2 text-rambutan-400 transition-colors focus-within:bg-longan-800/40 hover:bg-longan-800/40">
                        <Icon name="search" size=14 class="shrink-0 text-rambutan-500" />
                        <input
                            type="search"
                            placeholder="Search"
                            aria-label="Search threads"
                            class="min-w-0 flex-1 bg-transparent text-body-sm text-rambutan-100 placeholder:text-rambutan-500 focus:outline-none"
                            prop:value=move || query.get()
                            on:input=move |ev| set_query.set(event_target_value(&ev))
                        />
                    </label>
                    <button
                        type="button"
                        aria-label="New thread (⌘⇧O)"
                        title="New thread (⌘⇧O)"
                        class="inline-flex size-8 shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100"
                        on:click=move |_| new_thread()
                    >
                        <Icon name="square-pen" size=14 />
                    </button>
                </div>
                <Show when=move || mode.get() == bruhs_models::SidebarTab::Code>
                <div class="flex items-center gap-1">
                    <Menu open=filter_open align=MenuAlign::Start label="Filter threads by project"
                        trigger=move || view! {
                            <button
                                type="button"
                                aria-haspopup="menu"
                                aria-expanded=move || filter_open.get().to_string()
                                class="inline-flex h-7 max-w-[13rem] items-center gap-1.5 rounded-grape px-2 text-label-sm text-rambutan-300 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100"
                                on:click=move |_| filter_open.update(|v| *v = !*v)
                            >
                                <Icon name="folder" size=14 class="shrink-0 text-rambutan-500" />
                                <span class="truncate">{move || filter_label.get()}</span>
                                <Icon name="chevron-down" size=12 class="shrink-0 text-rambutan-500" />
                            </button>
                        }.into_any()
                    >
                        <MenuItem
                            selected=Signal::derive(move || project_filter.get().is_none())
                            on_select=Callback::new(move |_| { project_filter.set(None); filter_open.set(false); })
                        >
                            "All projects"
                        </MenuItem>
                        <For
                            each=move || names.get()
                            key=|n| n.clone()
                            children=move |name| {
                                let n_sel = name.clone();
                                let n_set = name.clone();
                                view! {
                                    <MenuItem
                                        selected=Signal::derive(move || project_filter.get().as_deref() == Some(n_sel.as_str()))
                                        on_select=Callback::new(move |_| { project_filter.set(Some(n_set.clone())); filter_open.set(false); })
                                    >
                                        <Icon name="folder" size=12 class="text-rambutan-500" />
                                        <span class="truncate">{name.clone()}</span>
                                    </MenuItem>
                                }
                            }
                        />
                    </Menu>
                    <button
                        type="button"
                        aria-label="New session in a project"
                        title="New session in a project"
                        class="ml-auto inline-flex size-7 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100"
                        on:click=move |_| set_project_picker_open.set(true)
                    >
                        <Icon name="folder-plus" size=14 />
                    </button>
                </div>
                </Show>
            </div>

            // Thread cards: fresh first, older seen threads under "Settled".
            <span id="thread-navigation-help" class="sr-only">"Use Up and Down to move between threads, Home and End to reach the first or last, and Enter to open. Search includes settled threads."</span>
            <ul node_ref=viewport.node role="list" aria-label="Threads" class="relative min-h-0 flex-1 overflow-y-auto px-2 pb-2"
                data-thread-count=move || layout.with(|layout| layout.count())
                style="overflow-anchor:none"
                on:scroll=move |_| viewport.measure()
                on:focusout=move |_| {
                    request_animation_frame(move || {
                        let Some(list) = viewport.node.try_get_untracked().flatten() else { return };
                        let inside = document().active_element().is_some_and(|element| list.contains(Some(&element)));
                        if !inside { focused.set(None); requested_focus.set(None); }
                    });
                }
            >
                <li aria-hidden="true" role="presentation" style=move || format!("height:{}px;pointer-events:none", layout.with(|layout| layout.height))></li>
                {move || {
                    let empty = shelves.with(Shelves::empty);
                    empty.then(|| view! {
                        <li class="flex flex-col items-center gap-2 px-2 py-6 text-center text-label-sm text-rambutan-500">
                            {move || if let Some(message) = catalog.empty_message() {
                                message
                            } else if query.with(|q| q.trim().is_empty()) {
                                "No threads yet. Press ⌘⇧O to start one."
                            } else {
                                "No threads match."
                            }}
                        </li>
                    })
                }}
                <For
                    each=move || slots.get()
                    key=|slot| slot.key()
                    children=move |slot| match slot {
                        window::Slot::Thread(row) => card(row.id),
                        window::Slot::Header(header) => shelf_header(header),
                    }
                />
            </ul>
            {move || sidebar_undo.current().map(|entry| view! {
                <div role="status" data-sidebar-undo class="mx-2 mb-2 flex min-h-10 items-center gap-2 rounded-grape bg-longan-800 px-3 text-label-sm text-rambutan-100 inset-ring inset-ring-white/10">
                    <span class="min-w-0 flex-1 truncate">{entry.message}</span>
                    <button
                        type="button"
                        class="inline-flex min-h-10 items-center rounded-seed px-2 text-label-sm text-persimmon-400 transition-colors hover:bg-longan-700/60"
                        on:click=move |_| {
                            let Some(entry) = sidebar_undo.take() else { return };
                            match entry.restore {
                                UndoRestore::Draft { text, chips } => {
                                    let crate::state::ChipRetargetGuard(guard) = expect_context();
                                    let crate::state::ComposerChips(composer_chips) = expect_context();
                                    guard.set_value(true);
                                    composer_chips.set(chips);
                                    composer_draft.set(text);
                                }
                                UndoRestore::Thread(previous) => {
                                    let previous = *previous;
                                    set_threads.update(|threads| {
                                        if let Some(thread) = threads.iter_mut().find(|thread| thread.id == previous.id) {
                                            let current = thread.clone();
                                            *thread = previous.clone();
                                            catalog.persist(Some(current), previous);
                                        }
                                    });
                                }
                            }
                        }
                    >
                        "Undo"
                    </button>
                </div>
            })}

            {move || {
                let id = menu_thread.get()?;
                let thread = threads.with(|rows| rows.iter().find(|row| row.id == id).cloned())?;
                let id = thread.id.clone();
                let streaming = streaming_thread_id.get().as_deref() == Some(id.as_str());
                let now = clock_i64(js_sys::Date::now());
                let offset = utc_offset_minutes_east();
                let presets = snooze_presets(now, offset);
                let snoozed = thread.snoozed_until_ms.is_some_and(|until| until > now);
                let pinned = thread.pinned;
                let settled = thread.settled;
                let show_discard = active_thread_id.get().as_deref() == Some(id.as_str())
                    && !composer_draft.with(String::is_empty);
                let left = menu_x.get().min(web_sys::window().and_then(|window| window.inner_width().ok()).and_then(|width| width.as_f64()).unwrap_or(800.0) - 220.0).max(8.0);
                let top = menu_y.get();
                let regenerate_id = id.clone();
                let discard_id = id.clone();
                Some(view! {
                    <div class="fixed inset-0 z-40" on:click=move |_| menu_thread.set(None)></div>
                    <div
                        role="menu"
                        aria-label="Thread actions"
                        data-thread-menu
                        class="fixed z-50 w-52 rounded-grape bg-longan-700 p-1 inset-ring inset-ring-white/15"
                        style=format!("left:{left}px;top:{top}px")
                    >
                        <button type="button" role="menuitem" class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60" on:click={
                            let id = id.clone();
                            move |_| {
                                let id = id.clone();
                                if pinned { mutate_thread(id, pin_off, Some("Thread unpinned")); } else { mutate_thread(id, pin_on, None); }
                            }
                        }>{if pinned { "Unpin" } else { "Pin" }}</button>
                        <button type="button" role="menuitem" class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60" on:click={
                            let id = id.clone();
                            move |_| {
                                let id = id.clone();
                                if settled { mutate_thread(id, settle_off, None); } else { mutate_thread(id, settle_on, Some("Thread settled")); }
                            }
                        }>{if settled { "Unsettle" } else { "Settle" }}</button>
                        {snoozed.then({
                            let id = id.clone();
                            move || view! { <button type="button" role="menuitem" class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60" on:click=move |_| mutate_thread(id.clone(), wake_on, None)>"Wake thread"</button> }
                        })}
                        <div role="separator" class="my-1 h-px bg-white/10"></div>
                        {presets.into_iter().map({
                            let id = id.clone();
                            move |preset| {
                                let id = id.clone();
                                let until = preset.until_ms;
                                view! {
                                    <button type="button" role="menuitem" class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60" on:click=move |_| snooze_thread_until(id.clone(), until)>
                                        {preset.label}
                                    </button>
                                }
                            }
                        }).collect_view()}
                        <button type="button" role="menuitem" class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60" on:click=move |_| {
                            custom_when.set(String::new());
                            custom_amount.set(String::new());
                            custom_unit.set("hours".to_string());
                            custom_for.set(Some(id.clone()));
                            menu_thread.set(None);
                        }>"Custom…"</button>
                        <div role="separator" class="my-1 h-px bg-white/10"></div>
                        <button
                            type="button"
                            role="menuitem"
                            disabled=streaming
                            class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60 disabled:opacity-50"
                            on:click=move |_| {
                                if streaming_thread_id.get_untracked().as_deref() == Some(regenerate_id.as_str()) { return; }
                                menu_thread.set(None);
                                let prompt = if active_thread_id.get_untracked().as_deref() == Some(regenerate_id.as_str()) {
                                    messages.with_untracked(|rows| rows.iter().find(|message| message.role == bruhs_models::agent::Role::User).map(|message| message.text()))
                                } else { None };
                                let project = threads.with_untracked(|rows| rows.iter().find(|row| row.id == regenerate_id).map(|row| row.project_name.clone())).unwrap_or_default();
                                let instructions = title_instructions.with_untracked(|store| instructions_text(&store.global, &store.by_project, &project));
                                set_threads.update(|rows| {
                                    if let Some(thread) = rows.iter_mut().find(|row| row.id == regenerate_id) {
                                        let original = thread.clone();
                                        thread.title = titled(&thread.title, prompt.as_deref(), &instructions);
                                        catalog.persist(Some(original), thread.clone());
                                    }
                                });
                            }
                        >"Regenerate title"</button>
                        {show_discard.then(|| view! {
                            <button type="button" role="menuitem" class="flex min-h-10 w-full items-center rounded-seed px-2.5 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60" on:click=move |_| {
                                menu_thread.set(None);
                                let text = composer_draft.get_untracked();
                                if text.is_empty() { return; }
                                let crate::state::ComposerChips(composer_chips) = expect_context();
                                let chips = composer_chips.get_untracked();
                                sidebar_undo.push(UndoEntry { message: "Draft discarded".into(), restore: UndoRestore::Draft { text, chips } });
                                composer_chips.set(Vec::new());
                                composer_draft.set(String::new());
                                let _ = discard_id;
                            }>"Discard draft"</button>
                        })}
                    </div>
                })
            }}
            <crate::ui::Dialog
                open=Signal::derive(move || custom_for.get().is_some())
                on_close=Callback::new(move |_| custom_for.set(None))
                max_width="max-w-sm".to_string()
                label="Snooze until".to_string()
            >
                <div class="flex flex-col gap-3 p-4">
                    <h2 class="text-label text-rambutan-100">"Snooze until"</h2>
                    <label class="flex flex-col gap-1 text-label-sm text-rambutan-400">
                        "Date and time"
                        <input
                            type="datetime-local"
                            class="min-h-10 rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                            prop:value=move || custom_when.get()
                            on:input=move |ev| custom_when.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="flex flex-col gap-1 text-label-sm text-rambutan-400">
                        "Or a duration"
                        <span class="flex gap-2">
                            <input
                                type="number"
                                min="1"
                                aria-label="Snooze amount"
                                class="min-h-10 w-24 rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                prop:value=move || custom_amount.get()
                                on:input=move |ev| custom_amount.set(event_target_value(&ev))
                            />
                            <select
                                aria-label="Snooze unit"
                                class="min-h-10 flex-1 rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                on:change=move |ev| custom_unit.set(event_target_value(&ev))
                            >
                                <option value="minutes" selected=move || custom_unit.get() == "minutes">"minutes"</option>
                                <option value="hours" selected=move || custom_unit.get() == "hours">"hours"</option>
                                <option value="days" selected=move || custom_unit.get() == "days">"days"</option>
                            </select>
                        </span>
                    </label>
                    <button
                        type="button"
                        class="inline-flex min-h-10 items-center justify-center rounded-grape bg-persimmon-500 px-3 text-label text-longan-950"
                        on:click=move |_| {
                            let Some(id) = custom_for.get_untracked() else { return };
                            let now = js_sys::Date::now();
                            let amount = custom_amount.get_untracked().trim().parse::<u64>().unwrap_or(0);
                            let unit = match custom_unit.get_untracked().as_str() {
                                "minutes" => SnoozeUnit::Minutes,
                                "days" => SnoozeUnit::Days,
                                _ => SnoozeUnit::Hours,
                            };
                            let until = if amount > 0 {
                                snooze_after(clock_i64(now), amount, unit)
                            } else {
                                let parsed = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(&custom_when.get_untracked())).get_time();
                                (parsed.is_finite() && parsed > now).then(|| clock_i64(parsed))
                            };
                            if let Some(until) = until {
                                custom_for.set(None);
                                snooze_thread_until(id, until);
                            }
                        }
                    >
                        "Snooze"
                    </button>
                </div>
            </crate::ui::Dialog>
            // T3's update button: only while a provider CLI or the app is behind.
            <crate::components::updates::SidebarUpdateButton />
            // Footer: the other surfaces as an icon row, settings at the end.
            <div class="flex items-center gap-0.5 px-2 py-2" role="group" aria-label="Views">
                {nav_icon(AppView::Today, "sun", "Today")}
                {nav_icon(AppView::Inbox, "inbox", "Inbox")}
                {nav_icon(AppView::Knowledge, "book-open", "Knowledge")}
                {nav_icon(AppView::Tasks, "list-checks", "Tasks")}
                {nav_icon(AppView::Terminal, "terminal", "Terminal")}
                {nav_icon(AppView::Calendar, "calendar", "Calendar")}
                {nav_icon(AppView::Meetings, "mic", "Meetings")}
                {nav_icon(AppView::People, "users", "People")}
                <span class="flex-1"></span>
                {nav_icon(AppView::Settings, "settings", "Settings")}
            </div>
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, project: &str, title: &str, at: &str) -> Thread {
        Thread {
            id: id.into(),
            title: title.into(),
            project_name: project.into(),
            // The legacy string reading, as stored rows get it.
            updated_at: serde_json::from_value::<Thread>(serde_json::json!({
                "id": id, "title": "", "project_name": "", "updated_at": at
            }))
            .unwrap()
            .updated_at,
            project_path: None,
            session_id: None,
            fresh_session: false,
            branch: None,
            model: None,
            pinned: false,
            settled: false,
            pin_rank: None,
            active_rank: None,
            snoozed_until_ms: None,
            goal: None,
            worktree_path: None,
            effort: None,
            run_locked: false,
            import_context: None,
        }
    }

    #[test]
    fn visible_threads_filters_and_sorts() {
        let all = vec![
            t("a", "bruhs", "Fix login", "100"),
            t("b", "", "Groceries", "300"),
            t("c", "bruhs", "Add tests", "200"),
            t("d", "other", "Legacy", "now"),
        ];
        let ids: Vec<String> = visible_threads(&all, None, "")
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["b", "c", "a", "d"]);
        assert_eq!(visible_threads(&all, Some("bruhs"), "").len(), 2);
        assert_eq!(visible_threads(&all, Some(""), "").len(), 1);
        assert_eq!(visible_threads(&all, None, "TEST")[0].id, "c");
    }

    #[test]
    fn chat_mode_ignores_remembered_project_filter() {
        let rows = vec![
            t("general", "", "Notes", "100"),
            t("project", "Alpha", "Notes", "200"),
        ];
        let visible = visible_workspace_threads(
            &rows,
            bruhs_models::SidebarTab::Chat,
            Some("Alpha"),
            "notes",
        );
        assert_eq!(
            visible
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            ["general"]
        );
    }

    #[test]
    fn code_mode_applies_project_filter_and_excludes_general_threads() {
        let rows = vec![
            t("general", "", "Notes", "300"),
            t("alpha", "Alpha", "Notes", "200"),
            t("beta", "Beta", "Notes", "100"),
        ];
        let visible = visible_workspace_threads(
            &rows,
            bruhs_models::SidebarTab::Code,
            Some("Alpha"),
            "notes",
        );
        assert_eq!(
            visible
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>(),
            ["alpha"]
        );
    }

    #[test]
    fn settled_needs_age_and_nothing_pending() {
        let day = SETTLED_AFTER_MS;
        let now = 10.0 * day;
        let old = t("a", "bruhs", "old", &format!("{}", now - 2.0 * day));
        let recent = t("b", "bruhs", "recent", &format!("{}", now - 3_600_000.0));
        let legacy = t("c", "bruhs", "legacy", "now");
        assert!(is_settled(&old, now, false, false, false));
        assert!(
            !is_settled(&old, now, true, false, false),
            "active stays up top"
        );
        assert!(
            !is_settled(&old, now, false, true, false),
            "working stays up top"
        );
        assert!(
            !is_settled(&old, now, false, false, true),
            "unread stays up top"
        );
        assert!(!is_settled(&recent, now, false, false, false));
        assert!(!is_settled(&legacy, now, false, false, false));
    }

    #[test]
    fn sidebar_order_pins_first_and_settles_last() {
        let day = SETTLED_AFTER_MS;
        let now = 10.0 * day;
        let mut pinned = t("p", "bruhs", "pinned", &format!("{}", now - 3_600_000.0));
        pinned.pinned = true;
        let mut pinned_old = t("old", "bruhs", "old pin", &format!("{}", now - 5.0 * day));
        pinned_old.pinned = true;
        let fresh = t("f", "bruhs", "fresh", &format!("{}", now - 3_600_000.0));
        let mut forced = t(
            "s",
            "bruhs",
            "settled by hand",
            &format!("{}", now - 60_000.0),
        );
        forced.settled = true;
        let aged = t("a", "bruhs", "aged", &format!("{}", now - 3.0 * day));
        let unread = HashSet::new();
        let order = sidebar_order(
            &[
                aged,
                forced.clone(),
                fresh,
                pinned_old.clone(),
                pinned.clone(),
            ],
            &OrderQuery {
                now_ms: now,
                active: None,
                working: None,
                needs_you: None,
                unread: &unread,
                working_section: true,
                folds: bruhs_models::logic::sidebar::FoldOpen {
                    working: true,
                    snoozed: true,
                    settled: true,
                },
            },
        );
        let ids: Vec<&str> = order.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["p", "f", "s", "a", "old"]);
        assert!(is_settled(&forced, now, false, false, false));
        assert!(is_settled(&pinned_old, now, false, false, false));
        assert!(!is_settled(&pinned, now, false, false, false));
    }

    #[test]
    fn project_names_are_distinct_and_recent_first() {
        let all = vec![
            t("a", "bruhs", "x", "100"),
            t("b", "other", "y", "300"),
            t("c", "bruhs", "z", "200"),
            t("d", "", "general", "400"),
        ];
        assert_eq!(
            project_names(&all),
            vec!["other".to_string(), "bruhs".to_string()]
        );
    }
}
