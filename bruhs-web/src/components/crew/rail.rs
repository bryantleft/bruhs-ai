//! The crew rail: every bruh above the thread list, with its face, status and
//! unread count. It sits outside the thread sections, so a bruh never mixes
//! into thread drag and drop.

use std::sync::atomic::{AtomicBool, Ordering};

use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::crew::room::{
    new_room_block, room_rail_rows, RoomRailRow, NEW_ROOM, NO_BRUHS_YET,
};
use bruhs_models::crew::{
    rail_order, service_notice, status_line, BruhState, BruhStatus, ServiceNotice, RAIL_FACE_PX,
};
use bruhs_models::reactive::ReactiveCommand;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::create::CreateBruhDialog;
use super::data::use_crew_snapshot;
use super::face::BruhFace;
use super::room_dialog::RoomDialog;
use super::shortcuts::CrewFocusRequest;
use crate::components::reactive_automations::local_time;
use crate::state::{load_pref, save_pref, ActiveView, AppView, SetActiveView, ThreadNotice};
use crate::ui::Icon;

/// Where the rail's open or closed state is kept.
const OPEN_PREF: &str = "bruhs.crewRailOpen";
/// What to do when the service answering is older and the web can't update it.
const SERVING_OLD_HINT: &str = "Run bruhs-server service install from this build.";
const ROW: &str = "bruhs-control flex min-h-control w-full min-w-0 items-center gap-2 rounded-grape px-2 py-1 text-left transition-colors focus-visible:outline-2 focus-visible:outline-persimmon-400";
const SMALL_ACTION: &str = "bruhs-control inline-flex min-h-control-xs items-center rounded-seed px-2 text-label-sm text-blueberry-300 transition-colors hover:bg-longan-800/50 hover:text-blueberry-200 focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50";

/// The installed service is updated automatically at most once per launch.
static AUTO_UPDATE_TRIED: AtomicBool = AtomicBool::new(false);

/// Open unless the person closed it.
fn rail_open(stored: Option<&str>) -> bool {
    stored != Some("false")
}

/// The installed service's crew protocol. An installed service whose release
/// can't say counts as the oldest.
fn installed_protocol(service: &BackgroundService) -> Option<u32> {
    service
        .installed
        .then(|| service.reactive_protocol.unwrap_or(0))
}

/// Which older service to update, once the serving protocol is known.
fn notice_for(protocol: Option<u32>, service: Option<&BackgroundService>) -> Option<ServiceNotice> {
    service_notice(protocol?, service.and_then(installed_protocol))
}

/// Whether to update the installed service without a click: it is older,
/// and the server that would install itself is a release build. A debug
/// server (a worktree's `cargo run`) waits for Update service.
fn auto_update_wanted(notice: Option<ServiceNotice>, service: Option<&BackgroundService>) -> bool {
    notice == Some(ServiceNotice::InstalledOld)
        && service.is_some_and(|service| !service.debug_build)
}

/// Focus the first bruh, or New bruh when there is none.
fn focus_crew_target() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    for selector in ["[data-crew-row]", "[data-crew-new]:not(:disabled)"] {
        if let Ok(Some(element)) = document.query_selector(selector) {
            if let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() {
                let _ = element.focus();
                return;
            }
        }
    }
}

/// The bruhs in rail order.
fn rail_rows(bruhs: &[BruhState]) -> Vec<BruhState> {
    let mut rows = bruhs.to_vec();
    rail_order(&mut rows);
    rows
}

/// The line under a bruh's name, with its next wake in local time.
fn row_status(state: &BruhState) -> String {
    let wake = state.next_wake_ms.map(local_time);
    status_line(state.status, wake.as_deref())
}

/// Why New room is off. Until the crew has loaded, nobody knows whether it
/// has a bruh, so it is off and titled plainly, as for an unknown service.
fn room_block_for(protocol: Option<u32>, bruhs: Option<usize>) -> Option<&'static str> {
    match bruhs {
        Some(bruhs) => new_room_block(protocol, bruhs),
        None => new_room_block(None, 0),
    }
}

fn unread_text(unread: usize) -> String {
    if unread > 99 {
        "99+".into()
    } else {
        unread.to_string()
    }
}

/// One bruh's row, kept current from `rows` by its id.
fn rail_row(
    agent_id: String,
    rows: Memo<Option<Vec<BruhState>>>,
    active_view: Option<ReadSignal<AppView>>,
    open_bruh: Callback<String>,
) -> impl IntoView {
    let id = StoredValue::new(agent_id);
    let state = Memo::new(move |_| {
        rows.with(|rows| {
            rows.as_ref().and_then(|rows| {
                id.with_value(|id| rows.iter().find(|row| row.agent_id == *id).cloned())
            })
        })
    });
    let look = Signal::derive(move || state.get().map(|state| state.look).unwrap_or_default());
    let status =
        Signal::derive(move || state.get().map_or(BruhStatus::Asleep, |state| state.status));
    let name = move || state.get().map(|state| state.name).unwrap_or_default();
    let line = move || {
        state
            .get()
            .map(|state| row_status(&state))
            .unwrap_or_default()
    };
    let unread = move || state.get().map_or(0, |state| state.unread);
    let current = move || {
        active_view.is_some_and(|view| {
            view.with(|view| {
                matches!(view, AppView::Bruh { agent_id } if id.with_value(|id| id == agent_id))
            })
        })
    };
    view! {
        <li>
            <button
                type="button"
                data-crew-row=move || id.get_value()
                aria-current=move || current().then_some("page")
                title=move || format!("{} {}", name(), line())
                class=move || format!("{ROW} {}", if current() { "bg-longan-800/60" } else { "hover:bg-longan-800/40" })
                on:click=move |_| open_bruh.run(id.get_value())
            >
                <BruhFace size=f64::from(RAIL_FACE_PX) look=look status=status />
                <span class="min-w-0 flex-1">
                    <span class="block truncate text-label text-rambutan-100">{name}</span>
                    <span class="block truncate text-label-sm text-rambutan-500">{line}</span>
                </span>
                <Show when=move || { unread() > 0 }>
                    <span aria-hidden="true" class="shrink-0 rounded-orb bg-blueberry-400 px-1.5 text-label-sm text-longan-950">{move || unread_text(unread())}</span>
                    <span class="sr-only">{move || format!("{} unread", unread())}</span>
                </Show>
            </button>
        </li>
    }
}

/// One room's row, kept current from `rows` by its id.
fn room_row(
    room_id: String,
    rows: Memo<Vec<RoomRailRow>>,
    active_view: Option<ReadSignal<AppView>>,
    open_room: Callback<String>,
) -> impl IntoView {
    let id = StoredValue::new(room_id);
    let row = Memo::new(move |_| {
        rows.with(|rows| id.with_value(|id| rows.iter().find(|row| row.id == *id).cloned()))
    });
    let name = move || row.get().map(|row| row.name).unwrap_or_default();
    let line = move || row.get().map(|row| row.line).unwrap_or_default();
    let current = move || {
        active_view.is_some_and(|view| {
            view.with(|view| {
                matches!(view, AppView::Room { room_id } if id.with_value(|id| id == room_id))
            })
        })
    };
    view! {
        <li>
            <button
                type="button"
                data-crew-room=move || id.get_value()
                aria-current=move || current().then_some("page")
                title=move || format!("{} {}", name(), line())
                class=move || format!("{ROW} {}", if current() { "bg-longan-800/60" } else { "hover:bg-longan-800/40" })
                on:click=move |_| open_room.run(id.get_value())
            >
                // As wide as a bruh's face, so room and bruh names line up.
                <span
                    aria-hidden="true"
                    class="inline-flex shrink-0 items-center justify-center text-rambutan-400"
                    style=format!("width:{RAIL_FACE_PX}px;height:{RAIL_FACE_PX}px")
                >
                    <Icon name="users" size=16 />
                </span>
                <span class="min-w-0 flex-1">
                    <span class="block truncate text-label text-rambutan-100">{name}</span>
                    <span class="block truncate text-label-sm text-rambutan-500">{line}</span>
                </span>
            </button>
        </li>
    }
}

#[component]
pub fn CrewRail(
    /// Runs after a bruh's page opens, such as to close a phone's sidebar.
    #[prop(optional)]
    on_open: Option<Callback<()>>,
) -> impl IntoView {
    let data = use_crew_snapshot();
    let active_view = use_context::<ActiveView>().map(|view| view.0);
    let set_view = use_context::<SetActiveView>().map(|set| set.0);
    let open = RwSignal::new(rail_open(load_pref(OPEN_PREF).as_deref()));
    let creating = RwSignal::new(false);
    let creating_room = RwSignal::new(false);
    let rows = Memo::new(move |_| {
        data.snapshot
            .with(|snapshot| snapshot.as_ref().map(|snapshot| rail_rows(&snapshot.bruhs)))
    });
    let rooms = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .map(|snapshot| room_rail_rows(&snapshot.rooms, &snapshot.bruhs))
                .unwrap_or_default()
        })
    });
    let any_paused = Memo::new(move |_| {
        rows.with(|rows| {
            rows.as_ref()
                .is_some_and(|rows| rows.iter().any(|bruh| bruh.status == BruhStatus::Paused))
        })
    });
    let any_awake = Memo::new(move |_| {
        rows.with(|rows| {
            rows.as_ref().is_some_and(|rows| {
                rows.iter()
                    .any(|bruh| !matches!(bruh.status, BruhStatus::Paused | BruhStatus::Off))
            })
        })
    });
    let ready = Signal::derive(move || data.crew_ready());
    // Stop all, as ⌘. stops the bruhs: it works on an older service too.
    let stopping = RwSignal::new(false);
    let notice_bar = use_context::<ThreadNotice>();
    let stop_all = move |_| {
        stopping.set(true);
        super::shortcuts::stop_all(data.protocol(), move |outcome| {
            let _ = stopping.try_set(false);
            match outcome {
                Ok(message) => {
                    if let Some(notice_bar) = notice_bar {
                        notice_bar.flash(message);
                    }
                }
                Err(error) => {
                    let _ = data.error.try_set(Some(error));
                }
            }
            data.reload.run(());
        });
    };

    // The installed service, read whenever the serving protocol changes.
    let service = RwSignal::new(None::<BackgroundService>);
    let read_for = StoredValue::new(None::<u32>);
    Effect::new(move |_| {
        let Some(protocol) = data.protocol() else {
            return;
        };
        if read_for.get_value() == Some(protocol) {
            return;
        }
        read_for.set_value(Some(protocol));
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(status) = crate::api::background_service().await {
                let _ = service.try_set(Some(status));
            }
        });
    });
    let notice =
        Memo::new(move |_| service.with(|service| notice_for(data.protocol(), service.as_ref())));

    // An older installed service is updated once per launch; the notice and
    // its button stay for when that fails.
    let updating = RwSignal::new(false);
    let update_error = RwSignal::new(None::<String>);
    let update = Callback::new(move |()| {
        if updating.get_untracked() {
            return;
        }
        updating.set(true);
        update_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::set_background_service(BackgroundServiceAction::Install).await;
            match result {
                Ok(status) => {
                    let _ = service.try_set(Some(status));
                }
                Err(error) => {
                    let _ = update_error.try_set(Some(error.to_string()));
                }
            }
            let _ = updating.try_set(false);
        });
    });
    Effect::new(move |_| {
        if service.with(|service| auto_update_wanted(notice.get(), service.as_ref()))
            && !AUTO_UPDATE_TRIED.swap(true, Ordering::AcqRel)
        {
            update.run(());
        }
    });

    // ⌘⇧B: open the rail and focus its first row, or New bruh with no rows.
    if let Some(CrewFocusRequest(request)) = use_context::<CrewFocusRequest>() {
        Effect::new(move |previous: Option<u32>| {
            let count = request.get();
            if previous.is_some_and(|previous| previous != count) {
                open.set(true);
                save_pref(OPEN_PREF, "true");
                request_animation_frame(focus_crew_target);
            }
            count
        });
    }
    let toggle = move |_| {
        let next = !open.get_untracked();
        open.set(next);
        save_pref(OPEN_PREF, if next { "true" } else { "false" });
    };
    let open_bruh = Callback::new(move |agent_id: String| {
        if let Some(set_view) = set_view {
            set_view.set(AppView::Bruh { agent_id });
        }
        if let Some(on_open) = on_open {
            on_open.run(());
        }
    });
    let open_room = Callback::new(move |room_id: String| {
        if let Some(set_view) = set_view {
            set_view.set(AppView::Room { room_id });
        }
        if let Some(on_open) = on_open {
            on_open.run(());
        }
    });
    let new_title = Signal::derive(move || data.service_reason().unwrap_or("New bruh"));
    let room_block = Memo::new(move |_| {
        let bruhs = data
            .snapshot
            .with(|snapshot| snapshot.as_ref().map(|snapshot| snapshot.bruhs.len()));
        room_block_for(data.protocol(), bruhs)
    });

    view! {
        <section aria-label="Crew" data-crew-rail="" class="flex min-h-0 shrink-0 flex-col px-2 pb-2">
            <div class="flex items-center gap-1 px-1">
                <button
                    type="button"
                    class="bruhs-control flex min-h-control min-w-0 flex-1 items-center gap-2 py-1 text-label-sm text-rambutan-500 transition-colors hover:text-rambutan-300 focus-visible:outline-2 focus-visible:outline-persimmon-400"
                    aria-expanded=move || open.get().to_string()
                    aria-controls="crew-rail-body"
                    on:click=toggle
                >
                    <span class="shrink-0">"Crew"</span>
                    <span class="h-px flex-1 bg-longan-800/80"></span>
                    {move || {
                        let class = format!(
                            "shrink-0 transition-transform duration-200 {}",
                            if open.get() { "rotate-180" } else { "" }
                        );
                        view! { <Icon name="chevron-down" size=16 class=class /> }
                    }}
                </button>
                <button
                    type="button"
                    aria-label="New bruh"
                    title=move || new_title.get()
                    data-crew-new=""
                    disabled=move || !ready.get()
                    class="bruhs-control inline-flex size-control-sm shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50"
                    on:click=move |_| creating.set(true)
                >
                    <Icon name="user-plus" size=16 />
                </button>
                <button
                    type="button"
                    aria-label=NEW_ROOM
                    title=move || room_block.get().unwrap_or(NEW_ROOM)
                    data-crew-new-room=""
                    disabled=move || room_block.with(Option::is_some)
                    class="bruhs-control inline-flex size-control-sm shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50"
                    on:click=move |_| creating_room.set(true)
                >
                    <Icon name="plus" size=16 />
                </button>
            </div>
            <Show when=move || open.get()>
                <div id="crew-rail-body" class="flex min-h-0 flex-col gap-1">
                    {move || (!updating.get()).then(|| notice.get()).flatten().map(|notice| view! {
                        <div data-crew-notice="" role="status" class="mx-1 space-y-1 rounded-grape bg-longan-950/60 p-2 text-label-sm text-rambutan-300 inset-ring inset-ring-longan-700/50">
                            <p>{notice.message()}</p>
                            {match notice {
                                ServiceNotice::InstalledOld => view! {
                                    <button type="button" class=SMALL_ACTION disabled=move || updating.get() on:click=move |_| update.run(())>"Update service"</button>
                                    {move || update_error.get().map(|message| view! { <p class="break-words text-lychee-200">{message}</p> })}
                                }.into_any(),
                                ServiceNotice::ServingOld => view! { <p class="text-rambutan-400">{SERVING_OLD_HINT}</p> }.into_any(),
                            }}
                        </div>
                    })}
                    <Show when=move || any_paused.get() || any_awake.get()>
                        <div class="flex flex-wrap gap-1 px-1">
                            <Show when=move || any_awake.get()>
                                <button
                                    type="button"
                                    data-crew-stop-all=""
                                    class=SMALL_ACTION
                                    disabled=move || stopping.get()
                                    on:click=stop_all
                                >"Stop all"</button>
                            </Show>
                            <Show when=move || any_paused.get()>
                                <button
                                    type="button"
                                    data-crew-resume-all=""
                                    class=SMALL_ACTION
                                    disabled=move || data.busy.get() || !ready.get()
                                    on:click=move |_| data.send.run(ReactiveCommand::ResumeAll)
                                >"Resume all"</button>
                            </Show>
                        </div>
                    </Show>
                    {move || match rows.with(|rows| rows.as_ref().map(Vec::is_empty)) {
                        None => data.error.get().map(|_| view! {
                            <div class="flex items-center gap-2 px-1 text-label-sm text-rambutan-500">
                                <span>"Couldn't load the crew."</span>
                                <button type="button" class=SMALL_ACTION on:click=move |_| data.reload.run(())>"Retry"</button>
                            </div>
                        }.into_any()),
                        Some(true) => Some(view! {
                            <p class="px-1 text-label-sm text-rambutan-500">{NO_BRUHS_YET}</p>
                        }.into_any()),
                        Some(false) => None,
                    }}
                    {move || rows.with(|rows| rows.is_some()).then(|| data.error.get()).flatten().map(|message| view! {
                        <p role="alert" class="mx-1 break-words text-label-sm text-lychee-200">{message}</p>
                    })}
                    // One scroll region for both lists, rooms under the bruhs.
                    <div data-crew-lists="" class="max-h-[40vh] min-h-0 space-y-0.5 overflow-y-auto">
                        <ul role="list" aria-label="Bruhs" class="m-0 list-none space-y-0.5 p-0">
                            <For
                                each=move || rows.get().unwrap_or_default()
                                key=|bruh| bruh.agent_id.clone()
                                children=move |bruh: BruhState| rail_row(bruh.agent_id, rows, active_view, open_bruh)
                            />
                        </ul>
                        // No empty list for a screen reader to announce.
                        <Show when=move || rooms.with(|rooms| !rooms.is_empty())>
                            <ul role="list" aria-label="Rooms" class="m-0 list-none space-y-0.5 p-0">
                                <For
                                    each=move || rooms.get()
                                    key=|room| room.id.clone()
                                    children=move |room: RoomRailRow| room_row(room.id, rooms, active_view, open_room)
                                />
                            </ul>
                        </Show>
                    </div>
                </div>
            </Show>
            <CreateBruhDialog open=creating on_open=on_open.unwrap_or_else(|| Callback::new(|()| {})) />
            <RoomDialog open=creating_room on_open=on_open.unwrap_or_else(|| Callback::new(|()| {})) />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::room::{Room, RoomState};
    use bruhs_models::crew::{BruhAccess, BruhLook, CREW_PROTOCOL};
    use bruhs_models::EpochMs;

    fn service(installed: bool, protocol: Option<u32>) -> BackgroundService {
        BackgroundService {
            supported: true,
            installed,
            loaded: installed,
            binary: None,
            detail: String::new(),
            serving: false,
            service_pid: None,
            responder_pid: None,
            log_path: None,
            reactive_protocol: protocol,
            debug_build: false,
        }
    }

    fn bruh(name: &str, status: BruhStatus) -> BruhState {
        BruhState {
            agent_id: format!("id-{name}"),
            name: name.into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status,
            next_wake_ms: None,
            unread: 0,
            computer: false,
            vault: Vec::new(),
            apps: Vec::new(),
            mail: Vec::new(),
        }
    }

    #[test]
    fn the_rail_stays_open_unless_closed() {
        assert!(rail_open(None));
        assert!(rail_open(Some("true")));
        assert!(!rail_open(Some("false")));
    }

    #[test]
    fn an_installed_service_that_cannot_say_counts_as_old() {
        assert_eq!(installed_protocol(&service(false, None)), None);
        assert_eq!(installed_protocol(&service(true, None)), Some(0));
        assert_eq!(
            installed_protocol(&service(true, Some(CREW_PROTOCOL))),
            Some(CREW_PROTOCOL)
        );
    }

    #[test]
    fn the_notice_waits_for_the_serving_protocol() {
        let old_install = service(true, None);
        assert_eq!(notice_for(None, Some(&old_install)), None);
        assert_eq!(notice_for(None, None), None);
        assert_eq!(notice_for(Some(0), None), Some(ServiceNotice::ServingOld));
        assert_eq!(
            notice_for(Some(CREW_PROTOCOL), Some(&old_install)),
            Some(ServiceNotice::InstalledOld)
        );
        assert_eq!(
            notice_for(
                Some(CREW_PROTOCOL),
                Some(&service(true, Some(CREW_PROTOCOL)))
            ),
            None
        );
        assert_eq!(
            notice_for(Some(CREW_PROTOCOL), Some(&service(false, None))),
            None
        );
    }

    #[test]
    fn only_a_release_server_updates_the_service_without_a_click() {
        let release = service(true, None);
        let debug = BackgroundService {
            debug_build: true,
            ..release.clone()
        };
        let old = Some(ServiceNotice::InstalledOld);
        assert!(auto_update_wanted(old, Some(&release)));
        assert!(!auto_update_wanted(old, Some(&debug)));
        assert!(!auto_update_wanted(old, None));
        assert!(!auto_update_wanted(
            Some(ServiceNotice::ServingOld),
            Some(&release)
        ));
        assert!(!auto_update_wanted(None, Some(&release)));
    }

    #[test]
    fn rows_put_whoever_needs_you_first_and_say_when_a_sleeper_wakes() {
        let rows = rail_rows(&[
            bruh("Off", BruhStatus::Off),
            bruh("Zed", BruhStatus::Asleep),
            bruh("Ann", BruhStatus::NeedsYou),
            bruh("Bo", BruhStatus::Paused),
        ]);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Ann", "Zed", "Bo", "Off"]);

        let mut sleeper = bruh("Zed", BruhStatus::Asleep);
        assert_eq!(row_status(&sleeper), "Asleep");
        sleeper.next_wake_ms = Some(EpochMs::saturating_from_i64(1_000));
        assert_eq!(
            row_status(&sleeper),
            format!("Wakes {}", local_time(EpochMs::saturating_from_i64(1_000)))
        );
    }

    fn room(id: &str, name: &str, working: &[&str]) -> RoomState {
        RoomState {
            room: Room {
                id: id.into(),
                name: name.into(),
                members: working.iter().map(|id| (*id).to_string()).collect(),
                lead: None,
            },
            last_seq: 0,
            working: working.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    #[test]
    fn rooms_sit_under_the_bruhs_by_name_with_their_status_line() {
        let mut ada = bruh("Ada", BruhStatus::Working);
        ada.agent_id = "x".into();
        let rows = room_rail_rows(
            &[room("b", "beta", &[]), room("a", "Alpha", &["x"])],
            &[ada],
        );
        let shown: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|row| (row.id.as_str(), row.name.as_str(), row.line.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("a", "Alpha", "Ada is working"),
                ("b", "beta", "No members")
            ]
        );
    }

    #[test]
    fn new_room_waits_for_the_crew_before_asking_for_a_bruh() {
        assert_eq!(room_block_for(Some(CREW_PROTOCOL), None), Some(NEW_ROOM));
        assert_eq!(
            room_block_for(Some(CREW_PROTOCOL), Some(0)),
            Some(bruhs_models::crew::room::ADD_A_BRUH_FIRST)
        );
        assert_eq!(room_block_for(Some(CREW_PROTOCOL), Some(1)), None);
    }

    #[test]
    fn the_unread_pill_caps_at_ninety_nine() {
        assert_eq!(unread_text(3), "3");
        assert_eq!(unread_text(99), "99");
        assert_eq!(unread_text(100), "99+");
    }
}
