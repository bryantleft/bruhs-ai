//! Thread sidebar on the panel tone: search up top, three-line thread
//! cards (project, title, then branch and provider), in-flight and recent
//! threads first, everything older and already seen folded under "Settled".

mod viewport;
mod window;

use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::file_attachments::transfer_has_files;
use crate::components::provider::{provider_agent_label_for_model, provider_logo_src_for_model};
use crate::format::relative_time;
use crate::state::{
    activate_thread, apply_drag, effect_for, groups_for, instructions_text, new_thread_in_context,
    set_settled, snooze_until, thread_facts, titled, wake, ActiveView, AppView, ComposerDraft,
    Drafts, PendingPermissions, PendingPermissionsSignal, SetActiveView, SetProjectPickerOpen,
    SetSidebarOpen, ShelfClock, SidebarChrome, SidebarUndo, Thread, TitleInstructionsSignal, Turns,
    UndoEntry, UndoRestore, UnreadThreads,
};
use crate::ui::{BrandMark, DateTimePicker, Dropdown, DropdownOption, Icon};

use bruhs_models::logic::menu::{menu_origin, menu_step, MenuRect};
use bruhs_models::logic::sidebar::{
    custom_snooze_until, is_thread_menu_key, snooze_presets, thread_menu, FoldOpen, SidebarSection,
    SnoozeUnit, ThreadMenuEntry, ThreadMenuFacts, SNOOZE_CUSTOM_TITLE,
};

/// An idle thread without unread messages moves to Settled after this interval.
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
    /// Threads with a running turn.
    pub working: &'a [String],
    /// Threads whose turn waits on an approval.
    pub needs_you: &'a [String],
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
    let by_id = threads
        .iter()
        .map(|thread| (thread.id.as_str(), thread))
        .collect::<HashMap<_, _>>();
    ids.into_iter()
        .filter_map(|id| by_id.get(id.as_str()).map(|thread| (**thread).clone()))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarAppGroup {
    Workspace,
    ToolsAndAgents,
}

#[derive(Clone, Copy)]
struct SidebarAppSpec {
    slot: usize,
    group: SidebarAppGroup,
    label: &'static str,
    icon: &'static str,
    test_id: &'static str,
}

impl SidebarAppSpec {
    fn view(self) -> AppView {
        match self.slot {
            0 => AppView::Today,
            1 => AppView::Inbox,
            2 => AppView::Tasks,
            3 => AppView::Calendar,
            4 => AppView::Meetings,
            5 => AppView::Knowledge,
            6 => AppView::People,
            7 => AppView::Terminal,
            8 => AppView::Workbench,
            _ => AppView::Automations,
        }
    }

    fn is_automations(self) -> bool {
        self.slot == 9
    }
}

const SIDEBAR_APP_SPECS: [SidebarAppSpec; 10] = [
    SidebarAppSpec {
        slot: 0,
        group: SidebarAppGroup::Workspace,
        label: "Today",
        icon: "sun",
        test_id: "sidebar-nav-today",
    },
    SidebarAppSpec {
        slot: 1,
        group: SidebarAppGroup::Workspace,
        label: "Inbox",
        icon: "inbox",
        test_id: "sidebar-nav-inbox",
    },
    SidebarAppSpec {
        slot: 2,
        group: SidebarAppGroup::Workspace,
        label: "Tasks",
        icon: "list-checks",
        test_id: "sidebar-nav-tasks",
    },
    SidebarAppSpec {
        slot: 3,
        group: SidebarAppGroup::Workspace,
        label: "Calendar",
        icon: "calendar",
        test_id: "sidebar-nav-calendar",
    },
    SidebarAppSpec {
        slot: 4,
        group: SidebarAppGroup::Workspace,
        label: "Meetings",
        icon: "mic",
        test_id: "sidebar-nav-meetings",
    },
    SidebarAppSpec {
        slot: 5,
        group: SidebarAppGroup::Workspace,
        label: "Knowledge",
        icon: "book-open",
        test_id: "sidebar-nav-knowledge",
    },
    SidebarAppSpec {
        slot: 6,
        group: SidebarAppGroup::Workspace,
        label: "People",
        icon: "users",
        test_id: "sidebar-nav-people",
    },
    SidebarAppSpec {
        slot: 7,
        group: SidebarAppGroup::ToolsAndAgents,
        label: "Terminal",
        icon: "terminal",
        test_id: "sidebar-nav-terminal",
    },
    SidebarAppSpec {
        slot: 8,
        group: SidebarAppGroup::ToolsAndAgents,
        label: "Workbench",
        icon: "wrench",
        test_id: "sidebar-nav-workbench",
    },
    SidebarAppSpec {
        slot: 9,
        group: SidebarAppGroup::ToolsAndAgents,
        label: "Agent workflows",
        icon: "bot",
        test_id: "sidebar-nav-agent-workflows",
    },
];

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
    let by_id = list
        .iter()
        .map(|thread| (thread.id.as_str(), thread))
        .collect::<HashMap<_, _>>();
    Shelves {
        pinned: take_ids(&by_id, &groups.pinned),
        active: take_ids(&by_id, &groups.active),
        working: take_ids(&by_id, &groups.working),
        snoozed: take_ids(&by_id, &groups.snoozed),
        settled: take_ids(&by_id, &groups.settled),
    }
}

fn take_ids(by_id: &HashMap<&str, &Thread>, ids: &[String]) -> Vec<Thread> {
    ids.iter()
        .filter_map(|id| by_id.get(id.as_str()).map(|thread| (**thread).clone()))
        .collect()
}

/// Epoch-ms of a thread's last activity; legacy rows ("now") sort last.
fn updated_ms(thread: &Thread) -> f64 {
    thread.updated_at.as_f64()
}

/// Threads to show: a case-insensitive search of titles and project names
/// (typing a project's name lists its threads), newest activity first.
pub fn visible_threads(threads: &[Thread], query: &str) -> Vec<Thread> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<Thread> = threads
        .iter()
        .filter(|t| {
            q.is_empty()
                || t.title.to_lowercase().contains(&q)
                || t.project_name.to_lowercase().contains(&q)
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        updated_ms(b)
            .partial_cmp(&updated_ms(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
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
    let turns = expect_context::<Turns>();
    let UnreadThreads(unread_threads) = expect_context::<UnreadThreads>();
    let roles = expect_context::<crate::state::RolesSignal>().0;
    let SetProjectPickerOpen(set_project_picker_open) = expect_context::<SetProjectPickerOpen>();
    let ActiveView(active_view) = expect_context::<ActiveView>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SetSidebarOpen(set_sidebar_open) = expect_context::<SetSidebarOpen>();
    let dismiss_mobile_navigation = move || {
        if crate::state::phone_width() {
            set_sidebar_open.set(false);
        }
    };

    let (query, set_query) = signal(String::new());
    let visible = Memo::new(move |_| {
        let q = query.get();
        threads.with(|ts| visible_threads(ts, &q))
    });

    let history = expect_context::<crate::thread_history::ThreadHistory>();
    let drafts = expect_context::<Drafts>();
    let select_thread = move |id: String| {
        dismiss_mobile_navigation();
        activate_thread(
            history,
            active_thread_id,
            id,
            set_active_thread_id,
            set_messages,
            turns,
            unread_threads,
        );
        // A thread always opens in the chat, whatever view was showing.
        // Only write when it changes: a same-value set still notifies and
        // would remount the chat page.
        if active_view.get_untracked() != AppView::Chat {
            set_active_view.set(AppView::Chat);
        }
    };

    // Footer utility button (Settings).
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
                    "inline-flex size-control shrink-0 items-center justify-center rounded-grape transition-colors cursor-pointer {}",
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

    // Unread agent updates count: surfaces on the collapsed Apps trigger and
    // inside the Agent workflows row when the Apps popover is open.
    let agent_updates = use_context::<crate::agent_updates::AgentUpdates>();
    let unread_agent_updates =
        Signal::derive(move || agent_updates.map_or(0, |updates| updates.unread.get()));
    let apps_menu_open = RwSignal::new(false);
    let apps_menu_at = RwSignal::new(None::<(f64, f64)>);
    let apps_trigger_ref = NodeRef::<leptos::html::Button>::new();
    let apps_menu_ref = NodeRef::<leptos::html::Div>::new();

    let catalog = expect_context::<crate::thread_catalog::ThreadCatalog>();
    let prompt_links = use_context::<crate::prompt_links::PromptLinks>();
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
        if let Some(links) = prompt_links {
            links.forget(&id);
        }
    };

    // New thread in the current project; the folder-plus button
    // and the command bar pick a different project.
    let new_thread = move || {
        dismiss_mobile_navigation();
        new_thread_in_context();
        if active_view.get_untracked() != AppView::Chat {
            set_active_view.set(AppView::Chat);
        }
    };

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
    let background_tasks = use_context::<crate::state::BackgroundTasksSignal>().map(|s| s.0);
    let dragging = RwSignal::new(None::<String>);
    let menu_thread = RwSignal::new(None::<String>);
    // The open thread menu's top-left corner, measured from its "…" button.
    // `None` until the first measure, which lands before the first paint.
    let menu_at = RwSignal::new(None::<(f64, f64)>);
    let menu_ref = NodeRef::<leptos::html::Div>::new();
    // The card whose "…" takes focus back once the list settles.
    let menu_return = RwSignal::new(None::<String>);
    let window_resized = RwSignal::new(0_u32);
    let custom_for = RwSignal::new(None::<String>);
    let custom_when = RwSignal::new(String::new());
    let custom_amount = RwSignal::new(String::new());
    let custom_unit = RwSignal::new(SnoozeUnit::Hours);
    let shelves = Memo::new(move |_| {
        let list = visible.get();
        let searching = query.with(|q| !q.trim().is_empty());
        let now = js_sys::Date::now();
        let active = active_thread_id.get();
        let mut streaming = turns.running();
        if let Some(bg) = background_tasks {
            bg.with(|ts| {
                for task in ts {
                    if task.status == bruhs_models::api::TaskStatus::Running {
                        let tid = task.thread_id.as_str();
                        if !streaming.iter().any(|existing| existing == tid) {
                            streaming.push(tid.to_string());
                        }
                    }
                }
            });
        }
        let needs_you = pending_permissions.with(PendingPermissions::threads);
        unread_threads.with(|unread| {
            shelve(
                list,
                searching,
                &OrderQuery {
                    now_ms: now,
                    active: active.as_deref(),
                    working: &streaming,
                    needs_you: &needs_you,
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
    let renaming_thread = RwSignal::new(None::<String>);
    let rename_draft = RwSignal::new(String::new());
    let start_rename = move |id: String| {
        if renaming_thread.with_untracked(Option::is_some) {
            return;
        }
        let title = threads.with_untracked(|rows| {
            rows.iter()
                .find(|thread| thread.id == id)
                .map(|thread| thread.title.clone())
        });
        if let Some(title) = title {
            rename_draft.set(title);
            renaming_thread.set(Some(id));
        }
    };
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
        // An open menu follows its card, so the card stays mounted too.
        let menu = menu_thread.get();
        layout.with(|layout| {
            let mut slots = layout.slots(top, height);
            // Wheel scrolling must not unmount the keyboard's current focus target.
            for id in [focused, requested, menu, renaming_thread.get()]
                .into_iter()
                .flatten()
            {
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
    // The thread menu works like desktop's. It hangs under its card's "…"
    // button (above it near the window's bottom) and follows the card. The
    // arrow keys, Home and End move through it; Escape, Tab and every action
    // close it and return focus to the button.
    let menu_button = move |id: &str| -> Option<web_sys::HtmlElement> {
        let list = viewport.node.try_get_untracked().flatten()?;
        let buttons = list
            .query_selector_all("button[data-thread-menu-for]")
            .ok()?;
        // Compare attributes rather than interpolating user-owned IDs into selectors.
        (0..buttons.length())
            .filter_map(|i| buttons.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
            .find(|button| button.get_attribute("data-thread-menu-for").as_deref() == Some(id))
    };
    let menu_items = move || -> Vec<web_sys::HtmlElement> {
        let Some(menu) = menu_ref.try_get_untracked().flatten() else {
            return Vec::new();
        };
        let Ok(items) = menu.query_selector_all("[role=menuitem]:not([disabled])") else {
            return Vec::new();
        };
        (0..items.length())
            .filter_map(|i| items.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
            .collect()
    };
    // `refocus` returns focus to the card's "…" button. It takes focus at
    // once, so Tab moves on from it. An action that moves the card (Pin
    // reorders the list) drops that focus, so the next frame gives it back;
    // the button stays shown until then.
    let close_menu = move |refocus: bool| {
        let Some(id) = menu_thread.try_get_untracked().flatten() else {
            return;
        };
        if refocus {
            if let Some(button) = menu_button(&id) {
                let _ = button.focus();
            }
            menu_return.set(Some(id.clone()));
            request_animation_frame(move || {
                if menu_return.try_get_untracked().flatten().as_deref() != Some(id.as_str()) {
                    return;
                }
                let dropped = document()
                    .active_element()
                    .is_none_or(|element| element.tag_name() == "BODY");
                if dropped {
                    if let Some(button) = menu_button(&id) {
                        let _ = button.focus();
                    }
                }
                menu_return.set(None);
            });
        }
        menu_thread.set(None);
        menu_at.set(None);
    };
    let place_menu = move || {
        let Some(id) = menu_thread.try_get_untracked().flatten() else {
            return;
        };
        let Some(menu) = menu_ref.try_get_untracked().flatten() else {
            return;
        };
        // The card left the list (a fold closed, the search changed), so
        // its menu closes too.
        let Some(button) = menu_button(&id) else {
            close_menu(false);
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let length = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
            value.ok().and_then(|value| value.as_f64()).unwrap_or(0.0)
        };
        let trigger = button.get_bounding_client_rect();
        let own = menu.get_bounding_client_rect();
        let at = menu_origin(
            MenuRect {
                left: trigger.left(),
                top: trigger.top(),
                width: trigger.width(),
                height: trigger.height(),
            },
            (own.width(), own.height()),
            (length(window.inner_width()), length(window.inner_height())),
        );
        if menu_at.get_untracked() != Some(at) {
            menu_at.set(Some(at));
        }
    };
    let open_menu = move |id: String| {
        apps_menu_open.set(false);
        apps_menu_at.set(None);
        menu_at.set(None);
        menu_thread.set(Some(id));
        // The menu mounts before the next frame. Place it and focus its
        // first row there, before it paints.
        request_animation_frame(move || {
            place_menu();
            if let Some(first) = menu_items().first() {
                let _ = first.focus();
            }
        });
    };
    let apps_menu_items = move || -> Vec<web_sys::HtmlElement> {
        let Some(menu) = apps_menu_ref.try_get_untracked().flatten() else {
            return Vec::new();
        };
        let Ok(items) = menu.query_selector_all("[role=menuitem]:not([disabled])") else {
            return Vec::new();
        };
        (0..items.length())
            .filter_map(|i| items.item(i)?.dyn_into::<web_sys::HtmlElement>().ok())
            .collect()
    };
    let close_apps_menu = move |refocus: bool| {
        if !apps_menu_open.try_get_untracked().unwrap_or(false) {
            return;
        }
        apps_menu_open.set(false);
        apps_menu_at.set(None);
        if refocus {
            if let Some(button) = apps_trigger_ref.try_get_untracked().flatten() {
                let _ = button.focus();
            }
        }
    };
    let place_apps_menu = move || {
        if !apps_menu_open.try_get_untracked().unwrap_or(false) {
            return;
        }
        let Some(menu) = apps_menu_ref.try_get_untracked().flatten() else {
            return;
        };
        let Some(button) = apps_trigger_ref.try_get_untracked().flatten() else {
            close_apps_menu(false);
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let length = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
            value.ok().and_then(|value| value.as_f64()).unwrap_or(0.0)
        };
        let trigger = button.get_bounding_client_rect();
        let own = menu.get_bounding_client_rect();
        let at = menu_origin(
            MenuRect {
                left: trigger.left(),
                top: trigger.top(),
                width: trigger.width(),
                height: trigger.height(),
            },
            (own.width(), own.height()),
            (length(window.inner_width()), length(window.inner_height())),
        );
        if apps_menu_at.get_untracked() != Some(at) {
            apps_menu_at.set(Some(at));
        }
    };
    let toggle_apps_menu = move || {
        if apps_menu_open.get_untracked() {
            close_apps_menu(true);
            return;
        }
        close_menu(false);
        apps_menu_at.set(None);
        apps_menu_open.set(true);
        let focus_slot = SIDEBAR_APP_SPECS
            .iter()
            .find(|spec| active_view.get_untracked() == spec.view())
            .map_or(0, |spec| spec.slot);
        request_animation_frame(move || {
            place_apps_menu();
            let items = apps_menu_items();
            if let Some(item) = items.get(focus_slot).or_else(|| items.first()) {
                let _ = item.focus();
            }
        });
    };
    let resized = window_event_listener(leptos::ev::resize, move |_| {
        window_resized.update(|count| *count = count.wrapping_add(1));
    });
    on_cleanup(move || resized.remove());
    // Follow the card when the list scrolls or reflows and when the window
    // resizes.
    Effect::new(move |_| {
        viewport.top.track();
        viewport.height.track();
        layout.track();
        window_resized.track();
        if menu_thread.with(Option::is_some) {
            request_animation_frame(place_menu);
        }
        if apps_menu_open.get() {
            request_animation_frame(place_apps_menu);
        }
    });

    // Thread activation republishes the destination mode even when unchanged.
    // Only an actual search change should reset a user's scroll position.
    let viewport_filter = Memo::new(move |_| query.get());
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
        let streaming = turns.running_untracked();
        let needs_you = pending_permissions.with_untracked(PendingPermissions::threads);
        let working_on = working_section.get_untracked();
        let mut undo: Option<(Thread, &'static str)> = None;
        set_threads.update(|threads| {
            let unread = unread_threads.get_untracked();
            let clock = ShelfClock {
                now_ms: now,
                active: active.as_deref(),
                working: &streaming,
                needs_you: &needs_you,
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
        let transfer = ev.data_transfer();
        if transfer_has_files(transfer.as_ref()) {
            return None;
        }
        transfer
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
        let id_context = id.clone();
        let position = Memo::new(move |_| layout.with(|layout| layout.row(&id_position).cloned()));
        let id_aria = id.clone();
        let is_active =
            Signal::derive(move || active_thread_id.get().as_deref() == Some(id_active.as_str()));
        let status = Signal::derive(move || {
            let streaming = turns.is_running(&id_status);
            let running_bg = background_tasks.is_some_and(|bg| {
                bg.with(|ts| {
                    ts.iter().any(|task| {
                        task.status == bruhs_models::api::TaskStatus::Running
                            && task.thread_id.as_str() == id_status
                    })
                })
            });
            if streaming && pending_permissions.with(|p| p.has_for(&id_status)) {
                CardStatus::Approval
            } else if streaming || running_bg {
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
        let title = Signal::derive(move || {
            live.get()
                .map(|t| bruhs_models::logic::text::plain_title(&t.title).into_owned())
                .unwrap_or_default()
        });
        let rename_id = StoredValue::new(id.clone());
        let rename_input = NodeRef::<leptos::html::Input>::new();
        let row_button = NodeRef::<leptos::html::Button>::new();
        let is_renaming = Signal::derive(move || {
            renaming_thread.with(|value| value.as_ref() == Some(&rename_id.get_value()))
        });
        Effect::new(move |_| {
            if is_renaming.get() {
                if let Some(input) = rename_input.get() {
                    input.set_value(&rename_draft.get_untracked());
                    let _ = input.focus();
                    input.select();
                }
            }
        });
        let finish_rename = move |save: bool, restore_focus: bool| {
            if !is_renaming.get_untracked() {
                return;
            }
            renaming_thread.set(None);
            if save {
                let id = rename_id.get_value();
                let value = rename_draft.get_untracked();
                set_threads.update(|threads| {
                    if let Some(thread) = threads.iter_mut().find(|thread| thread.id == id) {
                        if let Some(title) =
                            bruhs_models::logic::text::resolve_rename(&value, &thread.title)
                        {
                            let original = thread.clone();
                            thread.title = title;
                            catalog.persist(Some(original), thread.clone());
                        }
                    }
                });
            }
            if restore_focus {
                if let Some(button) = row_button.get_untracked() {
                    let _ = button.focus();
                }
            }
        };
        let project = Signal::derive(move || {
            live.get()
                .map(|t| {
                    if t.project_name.is_empty() {
                        "No project".to_string()
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
        // The role's id keeps the line; its current name fills it. A deleted
        // role leaves the name out.
        let has_role = Signal::derive(move || live.get().is_some_and(|t| t.role.is_some()));
        let role = Signal::derive(move || {
            let id = live.get()?.role?;
            roles.with(|book| book.find(&id).map(|role| role.name.clone()))
        });
        let suppress_click = StoredValue::new(false);
        let drag_id = id.clone();
        let drop_card_id = id.clone();
        let file_drop_id = id.clone();
        let files_over = RwSignal::new(false);
        let menu_id = id.clone();
        let id_menu = id.clone();
        let id_return = id.clone();
        let menu_open =
            Memo::new(move |_| menu_thread.with(|open| open.as_deref() == Some(id_menu.as_str())));
        // While its menu is open, or its "…" is taking focus back, the card
        // shows its buttons as on hover.
        let menu_shown = Memo::new(move |_| {
            menu_open.get() || menu_return.with(|card| card.as_deref() == Some(id_return.as_str()))
        });
        let card_class = move || {
            let base = "w-full rounded-lychee px-2.5 py-2.5 text-left transition-colors";
            let unread = status.get() == CardStatus::Unread;
            if files_over.get() {
                format!(
                    "{base} bg-longan-800 text-rambutan-100 inset-ring inset-ring-persimmon-400"
                )
            } else if is_active.get() {
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
                on:dragover=move |ev: web_sys::DragEvent| {
                    let transfer = ev.data_transfer();
                    if !transfer_has_files(transfer.as_ref()) { return; }
                    ev.prevent_default();
                    if let Some(transfer) = transfer { transfer.set_drop_effect("copy"); }
                    if !files_over.get_untracked() { files_over.set(true); }
                }
                on:dragleave=move |ev: web_sys::DragEvent| {
                    if !files_over.get_untracked() { return; }
                    let row = ev.current_target().and_then(|target| target.dyn_into::<web_sys::Node>().ok());
                    let next = ev.related_target().and_then(|target| target.dyn_into::<web_sys::Node>().ok());
                    if !row.is_some_and(|row| row.contains(next.as_ref())) { files_over.set(false); }
                }
                on:drop=move |ev: web_sys::DragEvent| {
                    files_over.set(false);
                    let transfer = ev.data_transfer();
                    if !transfer_has_files(transfer.as_ref()) { return; }
                    ev.prevent_default();
                    ev.stop_propagation();
                    let Some(files) = transfer.and_then(|transfer| transfer.files()).filter(|files| files.length() != 0) else { return };
                    if live.get_untracked().is_none() { return; }
                    select_thread(file_drop_id.clone());
                    drafts.attach_dropped_files(file_drop_id.clone(), files);
                }
            >
                <button
                    type="button"
                    node_ref=row_button
                    data-thread-id=id
                    draggable="true"
                    data-sidebar-section={
                        let value = drag_id.clone();
                        move || layout.with(|layout| layout.row(&value).map_or("active", |row| window::section_key(row.section)))
                    }
                    aria-describedby="thread-navigation-help"
                    aria-keyshortcuts="F2 Shift+F10"
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
                        let transfer = ev.data_transfer();
                        if transfer_has_files(transfer.as_ref()) { return; }
                        ev.prevent_default();
                        if let Some(data) = transfer {
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
                    on:click=move |ev: web_sys::MouseEvent| {
                        if suppress_click.get_value() {
                            suppress_click.set_value(false);
                            return;
                        }
                        if ev.detail() == 2
                            && !ev.meta_key() && !ev.ctrl_key() && !ev.alt_key() && !ev.shift_key()
                            && is_active.get_untracked()
                        {
                            ev.prevent_default();
                            start_rename(id_select.clone());
                        } else {
                            select_thread(id_select.clone());
                        }
                    }
                    on:contextmenu=move |ev: web_sys::MouseEvent| {
                        ev.prevent_default();
                        ev.stop_propagation();
                        open_menu(id_context.clone());
                    }
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.meta_key() || ev.ctrl_key() || ev.alt_key() { return; }
                        if ev.key() == "F2" && !ev.shift_key() {
                            ev.prevent_default();
                            ev.stop_propagation();
                            if !ev.repeat() { start_rename(rename_id.get_value()); }
                            return;
                        }
                        if is_thread_menu_key(&ev.key(), ev.shift_key()) {
                            ev.prevent_default();
                            ev.stop_propagation();
                            if !ev.repeat() { open_menu(id_key.clone()); }
                            return;
                        }
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
                        <Icon name="folder" size=12 class="shrink-0 text-rambutan-500" />
                        <span class="min-w-0 flex-1 truncate text-label-sm text-rambutan-500">{move || project.get()}</span>
                        // The row's buttons cover this spot on hover, like the time.
                        {move || live.get().is_some_and(|t| t.pinned).then(|| {
                            let class = if menu_shown.get() {
                                "invisible shrink-0 text-rambutan-500"
                            } else {
                                "shrink-0 text-rambutan-500 group-hover/row:invisible group-focus-within/row:invisible"
                            };
                            view! { <Icon name="pin" size=12 class=class /> }
                        })}
                        <span class=move || if menu_shown.get() {
                            "invisible ml-auto flex h-5 shrink-0 items-center justify-end text-label-sm tabular-nums text-rambutan-500"
                        } else {
                            "ml-auto flex h-5 shrink-0 items-center justify-end text-label-sm tabular-nums text-rambutan-500 group-hover/row:invisible group-focus-within/row:invisible"
                        }>
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
                    <span class=move || if is_renaming.get() {
                        "invisible mt-1 block truncate text-label leading-[21px]"
                    } else {
                        "mt-1 block truncate text-label leading-[21px]"
                    }>{move || title.get()}</span>
                    {move || {
                        let b = branch.get();
                        let m = model.get();
                        let r = role.get();
                        (b.is_some() || m.is_some() || has_role.get()).then(|| view! {
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
                                {r.map(|r| {
                                    let title = format!("Role: {r}");
                                    view! {
                                        <span data-thread-role="" class="max-w-[6rem] shrink-0 truncate text-rambutan-400" title=title>{r}</span>
                                    }
                                })}
                                {m.map(|m| view! {
                                    <span role="img" aria-label=provider_agent_label_for_model(&m)
                                        title=provider_agent_label_for_model(&m) class="shrink-0">
                                        <BrandMark src=provider_logo_src_for_model(&m) />
                                    </span>
                                })}
                            </span>
                        })
                    }}
                </button>
                {move || is_renaming.get().then(|| view! {
                    <div class="pointer-events-none absolute inset-0 px-2.5 py-2.5">
                        <div class="h-5"></div>
                        <input
                            node_ref=rename_input
                            type="text"
                            aria-label="Thread title"
                            class="pointer-events-auto mt-1 block h-[21px] w-full rounded-seed bg-longan-900 px-1 text-label text-rambutan-100 inset-ring inset-ring-persimmon-400 focus:outline-none"
                            on:input=move |ev| rename_draft.set(event_target_value(&ev))
                            on:blur=move |_| finish_rename(true, false)
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                ev.stop_propagation();
                                if ev.is_composing() { return; }
                                match ev.key().as_str() {
                                    "Enter" => { ev.prevent_default(); finish_rename(true, true); }
                                    "Escape" => { ev.prevent_default(); finish_rename(false, true); }
                                    _ => {}
                                }
                            }
                        />
                    </div>
                })}
                // Sibling (not nested) so the card stays a single button.
                <button
                    type="button"
                    aria-label="Thread actions"
                    title="Thread actions"
                    aria-haspopup="menu"
                    aria-expanded=move || menu_open.get().to_string()
                    data-thread-menu-for=menu_id.clone()
                    class=move || if menu_shown.get() {
                        "absolute right-9 top-2 inline-flex size-control-xs items-center justify-center rounded-seed bg-longan-700/60 text-rambutan-100 transition-colors"
                    } else {
                        "absolute right-9 top-2 hidden size-control-xs items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-rambutan-100 group-hover/row:inline-flex group-focus-within/row:inline-flex"
                    }
                    on:click=move |ev: web_sys::MouseEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        if menu_open.get_untracked() {
                            close_menu(true);
                        } else {
                            open_menu(menu_id.clone());
                        }
                    }
                >
                    <Icon name="ellipsis" size=16 />
                </button>
                <button
                    type="button"
                    aria-label="Delete thread"
                    title="Delete thread"
                    class=move || if menu_shown.get() {
                        "absolute right-2 top-2 inline-flex size-control-xs items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-lychee-400"
                    } else {
                        "absolute right-2 top-2 hidden size-control-xs items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-700/60 hover:text-lychee-400 group-hover/row:inline-flex group-focus-within/row:inline-flex"
                    }
                    on:click=move |ev| {
                        ev.stop_propagation();
                        delete_thread(id_delete.clone());
                    }
                >
                    <Icon name="x" size=16 />
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
                            class="bruhs-control flex min-h-control w-full items-center gap-2 py-1 text-label-sm text-rambutan-500 transition-colors hover:text-rambutan-300"
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
                                view! { <Icon name="chevron-down" size=16 class=class /> }
                            }}
                        </button>
                    }.into_any(),
                    None => view! {
                        <div class="flex min-h-control w-full items-center gap-2 py-1 text-label-sm text-rambutan-500">
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
    let crate::state::ComposerChips(composer_chips) = expect_context();
    let regenerate_title = move |id: String| {
        if turns.is_running_untracked(&id) {
            return;
        }
        let prompt = if active_thread_id.get_untracked().as_deref() == Some(id.as_str()) {
            messages.with_untracked(|rows| {
                rows.iter()
                    .find(|message| message.role == bruhs_models::agent::Role::User)
                    .map(|message| message.text())
            })
        } else {
            None
        };
        let project = threads
            .with_untracked(|rows| {
                rows.iter()
                    .find(|row| row.id == id)
                    .map(|row| row.project_name.clone())
            })
            .unwrap_or_default();
        let instructions = title_instructions
            .with_untracked(|store| instructions_text(&store.global, &store.by_project, &project));
        set_threads.update(|rows| {
            if let Some(thread) = rows.iter_mut().find(|row| row.id == id) {
                let original = thread.clone();
                thread.title = titled(&thread.title, prompt.as_deref(), &instructions);
                catalog.persist(Some(original), thread.clone());
            }
        });
    };
    let discard_draft = move || {
        let text = composer_draft.get_untracked();
        if text.is_empty() {
            return;
        }
        let chips = composer_chips.get_untracked();
        sidebar_undo.push(UndoEntry {
            message: "Draft discarded".into(),
            restore: UndoRestore::Draft { text, chips },
        });
        composer_chips.set(Vec::new());
        composer_draft.set(String::new());
    };
    let run_menu_row = move |id: String, row: ThreadMenuEntry| match row {
        ThreadMenuEntry::Pin { pinned: true } => {
            mutate_thread(id, pin_off, Some("Thread unpinned"));
        }
        ThreadMenuEntry::Pin { pinned: false } => mutate_thread(id, pin_on, None),
        ThreadMenuEntry::Settle { settled: true } => mutate_thread(id, settle_off, None),
        ThreadMenuEntry::Settle { settled: false } => {
            mutate_thread(id, settle_on, Some("Thread settled"));
        }
        ThreadMenuEntry::Wake => mutate_thread(id, wake_on, None),
        // Times count from the click, not from when the menu opened.
        ThreadMenuEntry::Snooze(preset) => {
            let now = clock_i64(js_sys::Date::now());
            let until = snooze_presets(now, utc_offset_minutes_east())
                .into_iter()
                .find(|fresh| fresh.when == preset.when)
                .map(|fresh| fresh.until_ms);
            if let Some(until) = until {
                snooze_thread_until(id, until);
            }
        }
        ThreadMenuEntry::SnoozeCustom => {
            custom_when.set(String::new());
            custom_amount.set(String::new());
            custom_unit.set(SnoozeUnit::Hours);
            custom_for.set(Some(id));
        }
        ThreadMenuEntry::Rename => start_rename(id),
        ThreadMenuEntry::RegenerateTitle { .. } => regenerate_title(id),
        ThreadMenuEntry::DiscardDraft => discard_draft(),
        ThreadMenuEntry::Separator => {}
    };
    let menu_row = move |id: String, row: ThreadMenuEntry| -> AnyView {
        if row == ThreadMenuEntry::Separator {
            return view! { <div role="separator" class="my-1 h-px bg-white/10"></div> }.into_any();
        }
        let label = row.label();
        let enabled = row.enabled();
        view! {
            <button
                type="button"
                role="menuitem"
                tabindex="-1"
                disabled=!enabled
                class="flex min-h-control w-full items-center rounded-seed px-2 text-left text-label-sm text-rambutan-200 hover:bg-longan-800/60 disabled:opacity-50"
                on:click=move |_| {
                    close_menu(row != ThreadMenuEntry::Rename);
                    run_menu_row(id.clone(), row.clone());
                }
            >
                {label}
            </button>
        }
        .into_any()
    };

    view! {
        <nav
            aria-label="Threads"
            tabindex="-1"
            data-workspace-focus="sidebar"
            class="flex h-full w-[248px] shrink-0 flex-col bg-longan-900 inset-ring inset-ring-white/5 text-rambutan-300 focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-persimmon-400"
        >
            // Chrome header
            <div class="flex h-11 shrink-0 items-center gap-2 px-4">
                <img src="/bruhs.svg" alt="" aria-hidden="true" class="bruhs-brand-logo" />
                <span class="text-label text-rambutan-100">"bruhs"</span>
            </div>

            // One control row: search, then choose project folder and new
            // thread. Search matches project names too, so it replaces the
            // old project filter.
            <div class="flex items-center gap-1 px-2 pb-2">
                <label class="flex h-control min-w-0 flex-1 items-center gap-2 rounded-grape px-2 text-rambutan-400 transition-colors focus-within:bg-longan-800/40 hover:bg-longan-800/40">
                    <Icon name="search" size=16 class="shrink-0 text-rambutan-500" />
                    <input
                        type="search"
                        placeholder="Search"
                        aria-label="Search threads and projects"
                        class="min-w-0 flex-1 bg-transparent text-body-sm text-rambutan-100 placeholder:text-rambutan-500 focus:outline-none"
                        prop:value=move || query.get()
                        on:input=move |ev| set_query.set(event_target_value(&ev))
                    />
                </label>
                <button
                    type="button"
                    aria-label="Choose project folder"
                    title="Choose project folder"
                    class="bruhs-control inline-flex size-control shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100"
                    on:click=move |_| set_project_picker_open.set(true)
                >
                    <Icon name="folder-plus" size=16 />
                </button>
                <button
                    type="button"
                    aria-label="New thread (⌘⇧O)"
                    title="New thread (⌘⇧O)"
                    class="bruhs-control inline-flex size-control shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100"
                    on:click=move |_| new_thread()
                >
                    <Icon name="square-pen" size=16 />
                </button>
            </div>

            // The crew, above the threads and outside their sections, so a
            // bruh never joins thread drag and drop.
            <crate::components::crew::rail::CrewRail on_open=Callback::new(move |()| dismiss_mobile_navigation()) />

            // Thread cards: fresh first, older seen threads under "Settled".
            <span id="thread-navigation-help" class="sr-only">"Use Up and Down to move between threads, Home and End to reach the first or last, Enter to open, F2 to rename, and Shift+F10 for thread actions. Search includes settled threads."</span>
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
                        class="bruhs-control inline-flex min-h-control items-center rounded-seed px-2 text-label-sm text-persimmon-400 transition-colors hover:bg-longan-700/60"
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
                // The rows change only when this thread's facts do, so focus
                // keeps its row while other threads update.
                let facts = Memo::new({
                    let id = id.clone();
                    move |_| {
                        let now = clock_i64(js_sys::Date::now());
                        threads.with(|rows| {
                            rows.iter().find(|row| row.id == id).map(|thread| ThreadMenuFacts {
                                pinned: thread.pinned,
                                settled: thread.settled,
                                snoozed: thread.snoozed_until_ms.is_some_and(|until| until > now),
                                title_action_disabled: turns.is_running(&id),
                                has_draft: active_thread_id.get().as_deref() == Some(id.as_str())
                                    && !composer_draft.with(String::is_empty),
                            })
                        })
                    }
                });
                // A thread deleted elsewhere takes its menu with it.
                Effect::new(move |_| {
                    if facts.with(Option::is_none) {
                        close_menu(false);
                    }
                });
                let rows = Memo::new(move |_| {
                    let Some(facts) = facts.get() else {
                        return Vec::new();
                    };
                    let mut separators = 0;
                    thread_menu(facts, clock_i64(js_sys::Date::now()), utc_offset_minutes_east())
                        .into_iter()
                        .map(|row| {
                            let key = if row == ThreadMenuEntry::Separator {
                                separators += 1;
                                format!("---{separators}")
                            } else {
                                format!("{}:{}", row.label(), row.enabled())
                            };
                            (key, row)
                        })
                        .collect::<Vec<_>>()
                });
                let style = move || match menu_at.get() {
                    Some((left, top)) => format!("left:{left}px;top:{top}px"),
                    // Placed in the next frame, before paint. It can take
                    // focus meanwhile.
                    None => "left:0;top:0;opacity:0".to_string(),
                };
                Some(view! {
                    <div
                        class="fixed inset-0 z-40"
                        aria-hidden="true"
                        on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                        on:click=move |_| close_menu(true)
                    ></div>
                    <div
                        node_ref=menu_ref
                        role="menu"
                        aria-label="Thread actions"
                        data-thread-menu
                        class="fixed z-50 w-52 rounded-grape bg-longan-700 p-1 inset-ring inset-ring-white/15"
                        style=style
                        // A click keeps focus where it is, so the row runs
                        // before a focus change could close the menu.
                        on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.meta_key() || ev.ctrl_key() || ev.alt_key() {
                                return;
                            }
                            match ev.key().as_str() {
                                "Escape" => {
                                    ev.prevent_default();
                                    ev.stop_propagation();
                                    close_menu(true);
                                }
                                // Focus goes back to the button first, so the
                                // browser's own Tab moves on from there.
                                "Tab" => close_menu(true),
                                key => {
                                    let items = menu_items();
                                    let focused = document().active_element();
                                    let current = items.iter().position(|item| {
                                        focused.as_ref() == Some(item.unchecked_ref::<web_sys::Element>())
                                    });
                                    let next = menu_step(current, items.len(), key)
                                        .and_then(|next| items.get(next));
                                    if let Some(next) = next {
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        let _ = next.focus();
                                    }
                                }
                            }
                        }
                        on:focusout=move |ev: web_sys::FocusEvent| {
                            // Focus moving elsewhere (a shortcut opened the
                            // command bar) closes the menu, so its backdrop
                            // cannot swallow the next click.
                            let Some(next) = ev
                                .related_target()
                                .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                            else {
                                return;
                            };
                            let inside = menu_ref
                                .get_untracked()
                                .is_some_and(|menu| menu.contains(Some(&next)));
                            if !inside {
                                close_menu(false);
                            }
                        }
                    >
                        <For
                            each=move || rows.get()
                            key=|(key, _)| key.clone()
                            children=move |(_, row)| menu_row(id.clone(), row)
                        />
                    </div>
                })
            }}
            <crate::ui::Dialog
                open=Signal::derive(move || custom_for.get().is_some())
                on_close=Callback::new(move |_| custom_for.set(None))
                max_width="max-w-sm".to_string()
                label=SNOOZE_CUSTOM_TITLE.to_string()
            >
                <div data-snooze-custom class="flex flex-col gap-3 p-4">
                    <h2 class="text-h4 text-rambutan-100">{SNOOZE_CUSTOM_TITLE}</h2>
                    <div class="flex flex-col gap-1">
                        <span class="text-label-sm text-rambutan-400">"Date and time"</span>
                        <DateTimePicker
                            label="Snooze until"
                            value=Signal::derive(move || custom_when.get())
                            on_change=Callback::new(move |when: String| custom_when.set(when))
                        />
                    </div>
                    <div class="flex flex-col gap-1">
                        <span class="text-label-sm text-rambutan-400">"Or a duration"</span>
                        <span class="flex gap-2">
                            <input
                                type="number"
                                min="1"
                                aria-label="Snooze amount"
                                class="min-h-control w-24 rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                prop:value=move || custom_amount.get()
                                on:input=move |ev| custom_amount.set(event_target_value(&ev))
                            />
                            <Dropdown
                                options=Signal::derive(|| {
                                    SnoozeUnit::ALL
                                        .map(|unit| DropdownOption::new(unit.label(), unit.label()))
                                        .to_vec()
                                })
                                value=Signal::derive(move || custom_unit.get().label().to_string())
                                on_change=Callback::new(move |unit: String| {
                                    custom_unit.set(SnoozeUnit::from_label(&unit));
                                })
                                label="Snooze unit"
                                class="flex-1"
                            />
                        </span>
                    </div>
                    <button
                        type="button"
                        class="bruhs-control inline-flex min-h-control items-center justify-center rounded-grape bg-persimmon-500 px-3 text-label text-longan-950"
                        on:click=move |_| {
                            let Some(id) = custom_for.get_untracked() else { return };
                            let now = clock_i64(js_sys::Date::now());
                            // `YYYY-MM-DDTHH:MM` without an offset is local time.
                            let chosen = custom_when.with_untracked(|when| {
                                let at = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(when)).get_time();
                                at.is_finite().then(|| clock_i64(at))
                            });
                            let amount = custom_amount.get_untracked();
                            if let Some(until) = custom_snooze_until(now, &amount, custom_unit.get_untracked(), chosen) {
                                custom_for.set(None);
                                snooze_thread_until(id, until);
                            }
                        }
                    >
                        "Snooze"
                    </button>
                </div>
            </crate::ui::Dialog>
            // Show the update button while a provider CLI or the app is behind.
            <crate::components::updates::SidebarUpdateButton />
            // Single-row sidebar footer dock matching desktop: labeled `Apps`
            // view-switcher popover on the left, Settings on the right.
            {move || {
                let active_app = Signal::derive(move || {
                    let current = active_view.get();
                    SIDEBAR_APP_SPECS
                        .iter()
                        .copied()
                        .find(|spec| current == spec.view())
                });
                let trigger_highlighted =
                    Signal::derive(move || apps_menu_open.get() || active_app.get().is_some());
                let trigger_aria = Signal::derive(move || {
                    let label = active_app.get().map_or("Apps", |spec| spec.label);
                    let unread = unread_agent_updates.get();
                    match unread {
                        0 if active_app.get().is_some() => format!("{label}, switch app"),
                        0 => label.to_string(),
                        1 => format!("{label}, 1 unread update"),
                        count => format!("{label}, {count} unread updates"),
                    }
                });
                let apps_menu_style = move || match apps_menu_at.get() {
                    Some((left, top)) => format!("left:{left}px;top:{top}px"),
                    None => "left:0;top:0;opacity:0".to_string(),
                };
                let app_row = move |spec: SidebarAppSpec| {
                    let is_on = Signal::derive(move || active_view.get() == spec.view());
                    let aria_label = Signal::derive(move || {
                        if spec.is_automations() {
                            crate::agent_updates::AgentUpdates::nav_label(
                                unread_agent_updates.get(),
                            )
                        } else {
                            spec.label.to_string()
                        }
                    });
                    view! {
                        <button
                            type="button"
                            role="menuitem"
                            tabindex="-1"
                            data-testid=spec.test_id
                            aria-label=move || aria_label.get()
                            aria-current=move || is_on.get().then_some("page")
                            class=move || format!(
                                "flex h-[30px] w-full items-center justify-between gap-2 rounded-seed px-2 text-left text-label-sm transition-colors cursor-pointer focus-visible:outline-1 focus-visible:outline-persimmon-400 {}",
                                if is_on.get() {
                                    "bg-longan-800/75 text-rambutan-100 font-medium"
                                } else {
                                    "text-rambutan-200 hover:bg-longan-800/55"
                                }
                            )
                            on:click=move |_| {
                                close_apps_menu(false);
                                set_active_view.set(spec.view());
                                dismiss_mobile_navigation();
                            }
                        >
                            <span class="flex min-w-0 flex-1 items-center gap-2">
                                <Icon
                                    name=spec.icon
                                    size=15
                                    class=if is_on.get_untracked() {
                                        "text-rambutan-100"
                                    } else {
                                        "text-rambutan-400"
                                    }
                                />
                                <span class="min-w-0 truncate">{spec.label}</span>
                            </span>
                            <span class="flex shrink-0 items-center gap-1.5">
                                <Show when=move || { spec.is_automations() && unread_agent_updates.get() > 0 }>
                                    <span
                                        aria-hidden="true"
                                        data-testid="sidebar-agent-workflows-badge"
                                        class="inline-flex h-[16px] min-w-[16px] items-center justify-center rounded-orb bg-blueberry-400 px-1 text-[11px] font-semibold leading-[16px] text-longan-950"
                                    >
                                        {move || {
                                            let count = unread_agent_updates.get();
                                            if count > 99 { "99+".to_string() } else { count.to_string() }
                                        }}
                                    </span>
                                </Show>
                                <Show when=move || is_on.get()>
                                    <Icon name="check" size=13 class="text-persimmon-400" />
                                </Show>
                            </span>
                        </button>
                    }
                };
                view! {
                    <div
                        class="flex shrink-0 items-center justify-between gap-1.5 border-t border-longan-800/60 px-2 py-2"
                        role="group"
                        aria-label="Views"
                        data-testid="sidebar-footer"
                    >
                        <div class="relative min-w-0 flex-1">
                            <button
                                node_ref=apps_trigger_ref
                                type="button"
                                data-testid="sidebar-apps-trigger"
                                aria-haspopup="menu"
                                aria-expanded=move || apps_menu_open.get().to_string()
                                aria-label=move || trigger_aria.get()
                                title=move || trigger_aria.get()
                                class=move || format!(
                                    "flex h-control w-full items-center justify-between gap-1.5 rounded-grape border px-2 text-label-sm font-medium transition-colors cursor-pointer focus-visible:outline-1 focus-visible:outline-persimmon-400 {}",
                                    if trigger_highlighted.get() {
                                        "bg-longan-800/60 border-longan-700/75 text-rambutan-100"
                                    } else {
                                        "bg-longan-800/35 border-longan-800/70 text-rambutan-200 hover:bg-longan-800/55 hover:border-longan-700/65 hover:text-rambutan-100"
                                    }
                                )
                                on:click=move |_| toggle_apps_menu()
                            >
                                <span class="flex min-w-0 flex-1 items-center gap-1.5">
                                    {move || {
                                        let icon_name = active_app
                                            .get()
                                            .map_or("layout-dashboard", |spec| spec.icon);
                                        let icon_class = if trigger_highlighted.get() {
                                            "text-rambutan-100"
                                        } else {
                                            "text-rambutan-400"
                                        };
                                        view! { <Icon name=icon_name size=15 class=icon_class /> }
                                    }}
                                    <span class="min-w-0 truncate">
                                        {move || active_app.get().map_or("Apps", |spec| spec.label)}
                                    </span>
                                </span>
                                <span class="flex shrink-0 items-center gap-1">
                                    <Show when=move || { unread_agent_updates.get() > 0 }>
                                        <span
                                            aria-hidden="true"
                                            data-testid="sidebar-apps-unread-badge"
                                            class="inline-flex h-[16px] min-w-[16px] items-center justify-center rounded-orb bg-blueberry-400 px-1 text-[11px] font-semibold leading-[16px] text-longan-950"
                                        >
                                            {move || {
                                                let count = unread_agent_updates.get();
                                                if count > 99 { "99+".to_string() } else { count.to_string() }
                                            }}
                                        </span>
                                    </Show>
                                    <Icon
                                        name="chevron-down"
                                        size=12
                                        class="text-rambutan-500"
                                    />
                                </span>
                            </button>
                            <Show when=move || apps_menu_open.get()>
                                <div
                                    class="fixed inset-0 z-40"
                                    aria-hidden="true"
                                    on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                                    on:click=move |_| close_apps_menu(true)
                                ></div>
                                <div
                                    node_ref=apps_menu_ref
                                    role="menu"
                                    aria-label="Apps"
                                    data-sidebar-apps-menu
                                    data-testid="sidebar-apps-menu"
                                    class="fixed z-50 flex w-[232px] flex-col gap-px rounded-lychee bg-longan-700 p-1.5 shadow-lg inset-ring inset-ring-white/15"
                                    style=apps_menu_style
                                    on:mousedown=|ev: web_sys::MouseEvent| ev.prevent_default()
                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                        if ev.meta_key() || ev.ctrl_key() || ev.alt_key() {
                                            return;
                                        }
                                        match ev.key().as_str() {
                                            "Escape" => {
                                                ev.prevent_default();
                                                ev.stop_propagation();
                                                close_apps_menu(true);
                                            }
                                            "Tab" => close_apps_menu(true),
                                            key => {
                                                let items = apps_menu_items();
                                                let focused = document().active_element();
                                                let current = items.iter().position(|item| {
                                                    focused.as_ref()
                                                        == Some(item.unchecked_ref::<web_sys::Element>())
                                                });
                                                let next = menu_step(current, items.len(), key)
                                                    .and_then(|next| items.get(next));
                                                if let Some(next) = next {
                                                    ev.prevent_default();
                                                    ev.stop_propagation();
                                                    let _ = next.focus();
                                                }
                                            }
                                        }
                                    }
                                    on:focusout=move |ev: web_sys::FocusEvent| {
                                        let Some(next) = ev
                                            .related_target()
                                            .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                                        else {
                                            return;
                                        };
                                        let inside = apps_menu_ref
                                            .try_get_untracked()
                                            .flatten()
                                            .is_some_and(|menu| menu.contains(Some(&next)));
                                        if !inside {
                                            close_apps_menu(false);
                                        }
                                    }
                                >
                                    <div class="px-2 pt-1 pb-0.5 text-[10px] font-semibold tracking-wider text-rambutan-500">
                                        "WORKSPACE"
                                    </div>
                                    {SIDEBAR_APP_SPECS
                                        .iter()
                                        .copied()
                                        .filter(|spec| spec.group == SidebarAppGroup::Workspace)
                                        .map(app_row)
                                        .collect_view()}
                                    <div role="separator" class="mx-1 my-1 h-px bg-white/10"></div>
                                    <div class="px-2 pt-1 pb-0.5 text-[10px] font-semibold tracking-wider text-rambutan-500">
                                        "TOOLS & AGENTS"
                                    </div>
                                    {SIDEBAR_APP_SPECS
                                        .iter()
                                        .copied()
                                        .filter(|spec| spec.group == SidebarAppGroup::ToolsAndAgents)
                                        .map(app_row)
                                        .collect_view()}
                                </div>
                            </Show>
                        </div>
                        <div class="flex shrink-0 items-center gap-0.5">
                            {nav_icon(AppView::Usage, "activity", "Usage")}
                            {nav_icon(AppView::Settings, "settings", "Settings")}
                        </div>
                    </div>
                }
            }}
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
            session_provider: None,
            fresh_session: false,
            branch: None,
            model: None,
            composer_model: None,
            pinned: false,
            settled: false,
            pin_rank: None,
            active_rank: None,
            snoozed_until_ms: None,
            goal: None,
            worktree_path: None,
            linked_pull: None,
            effort: None,
            run_locked: false,
            role: None,
            instructions_stamp: None,
            import_context: None,
            import_key: None,
        }
    }

    #[test]
    #[ignore = "manual release timing: cargo test -p bruhs-web --release sidebar_navigation_cost -- --ignored --nocapture"]
    fn sidebar_navigation_cost() {
        use std::{hint::black_box, time::Instant};
        let threads = (0..10_000)
            .map(|index| {
                t(
                    &format!("thread-{index}"),
                    "bruhs",
                    "Thread",
                    &index.to_string(),
                )
            })
            .collect::<Vec<_>>();
        let unread = HashSet::new();
        let query = OrderQuery {
            now_ms: 10_000.0,
            active: None,
            working: &[],
            needs_you: &[],
            unread: &unread,
            working_section: false,
            folds: FoldOpen {
                working: true,
                snoozed: true,
                settled: true,
            },
        };
        black_box(sidebar_order(&threads, &query));
        let mut samples = (0..31)
            .map(|_| {
                let start = Instant::now();
                black_box(sidebar_order(black_box(&threads), black_box(&query)));
                start.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        println!(
            "sidebar navigation / 10,000 threads: p50 {:.3} ms, p95 {:.3} ms",
            samples[15].as_secs_f64() * 1000.0,
            samples[29].as_secs_f64() * 1000.0
        );
    }

    #[test]
    fn visible_threads_search_titles_and_projects_newest_first() {
        let all = vec![
            t("a", "bruhs", "Fix login", "100"),
            t("b", "", "Groceries", "300"),
            t("c", "bruhs", "Add tests", "200"),
            t("d", "other", "Legacy", "now"),
        ];
        let ids = |query| {
            visible_threads(&all, query)
                .into_iter()
                .map(|t| t.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(""), ["b", "c", "a", "d"]);
        assert_eq!(ids("TEST"), ["c"]);
        assert_eq!(
            ids("Bruhs"),
            ["c", "a"],
            "a project's name lists its threads"
        );
        assert!(ids("nothing").is_empty());
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
            is_settled(&old, now, true, false, false),
            "selecting a settled thread keeps it in place"
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
                working: &[],
                needs_you: &[],
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
    fn sidebar_apps_popover_matches_desktop_groups() {
        let workspace: Vec<&str> = SIDEBAR_APP_SPECS
            .iter()
            .filter(|spec| spec.group == SidebarAppGroup::Workspace)
            .map(|spec| spec.label)
            .collect();
        let tools: Vec<&str> = SIDEBAR_APP_SPECS
            .iter()
            .filter(|spec| spec.group == SidebarAppGroup::ToolsAndAgents)
            .map(|spec| spec.label)
            .collect();
        assert_eq!(
            workspace,
            [
                "Today",
                "Inbox",
                "Tasks",
                "Calendar",
                "Meetings",
                "Knowledge",
                "People",
            ]
        );
        assert_eq!(tools, ["Terminal", "Workbench", "Agent workflows"]);
        for (idx, spec) in SIDEBAR_APP_SPECS.iter().enumerate() {
            assert_eq!(spec.slot, idx);
            assert_eq!(spec.is_automations(), spec.view() == AppView::Automations);
        }
    }
}
