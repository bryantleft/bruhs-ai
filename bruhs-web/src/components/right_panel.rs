//! Right panel shell: tab bar (Diff · Tasks · Preview · Files), drag-to-resize
//! handle, launcher when no surface is open. Surfaces render inside.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::api::branch_pull;
use crate::components::diff_panel::DiffPanel;
use crate::components::file_attachments::FileViewer;
use crate::components::files_panel::FilesPanel;
use crate::components::preview_panel::PreviewPanel;
use crate::components::tasks_panel::TasksSurface;
use crate::state::{
    clamp_panel_width, ActiveThreadId, BranchPullSignal, OpenFileSignal, PanelSurface,
    RightPanelSignal, RightPanelWidth, ShownBranchPull, ThreadsSignal, Turns,
};
use crate::ui::{Icon, Kbd};
use bruhs_models::logic::branch_pull::{
    branch_pull_key, github_pull_url, pull_state_label, should_open_branch_pull,
};

type Listener = Closure<dyn Fn(web_sys::Event)>;

fn viewport_width() -> f64 {
    web_sys::window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(1440.0)
}

#[component]
pub fn RightPanel() -> impl IntoView {
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();
    let RightPanelWidth(width) = expect_context::<RightPanelWidth>();
    let OpenFileSignal(open_file) = expect_context::<OpenFileSignal>();
    let BranchPullSignal(branch_pull) = expect_context::<BranchPullSignal>();

    let surface = Signal::derive(move || panel.get().surface);

    // ── Drag to resize ───────────────────────────────────────────────────────
    let listeners = StoredValue::new_local(None::<(Listener, Listener)>);
    let stop_drag = move || {
        if let Some((on_move, on_up)) = listeners.try_update_value(|l| l.take()).flatten() {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "pointermove",
                    on_move.as_ref().unchecked_ref(),
                );
                let _ = window.remove_event_listener_with_callback(
                    "pointerup",
                    on_up.as_ref().unchecked_ref(),
                );
            }
        }
    };
    let start_drag = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        stop_drag();
        let Some(window) = web_sys::window() else {
            return;
        };
        let on_move = Listener::new(move |raw: web_sys::Event| {
            if let Ok(pe) = raw.dyn_into::<web_sys::PointerEvent>() {
                let vw = viewport_width();
                let requested = vw - f64::from(pe.client_x());
                width.set(clamp_panel_width(requested, vw));
            }
        });
        let on_up = Listener::new(move |_raw: web_sys::Event| {
            // Detach on the next tick so this closure is not dropped while
            // it is still executing.
            gloo_timers::callback::Timeout::new(0, stop_drag).forget();
        });
        let _ = window
            .add_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref());
        let _ =
            window.add_event_listener_with_callback("pointerup", on_up.as_ref().unchecked_ref());
        listeners.set_value(Some((on_move, on_up)));
    };

    // Keep the stored width valid if the window shrinks. The flex layout in
    // `shell.rs` also lets the panel give way to the chat column's minimum
    // width, so a stale preference never crushes the transcript.
    Effect::new(move |_| {
        let vw = viewport_width();
        let w = width.get();
        let clamped = clamp_panel_width(w, vw);
        if (clamped - w).abs() > 0.5 {
            width.set(clamped);
        }
    });

    let close = move || panel.update(|p| p.open = false);

    view! {
        <aside
            aria-label="Right panel"
            class="relative flex h-full w-full flex-col bg-longan-900 inset-ring inset-ring-white/5 text-rambutan-300 md:w-[var(--panel-w)] md:max-w-full"
            style=move || format!("--panel-w:{}px", width.get().round())
        >
            <div
                role="separator"
                aria-orientation="vertical"
                aria-label="Resize panel"
                class="absolute inset-y-0 left-0 z-10 hidden w-1 cursor-col-resize transition-colors hover:bg-persimmon-400/40 md:block"
                on:pointerdown=start_drag
            ></div>

            // Tab bar
            <div class="flex h-[52px] shrink-0 items-center gap-1 pl-2 pr-3">
                <PanelTab surface=PanelSurface::Diff icon="file-diff" label="Diff" />
                <PanelTab surface=PanelSurface::Tasks icon="activity" label="Tasks" />
                <PanelTab surface=PanelSurface::Preview icon="globe" label="Preview" />
                <PanelTab surface=PanelSurface::Files icon="folder" label="Files" />
                {move || {
                    branch_pull.get().map(|pull| {
                        let label = format!("#{}", pull.number);
                        view! {
                            <button
                                type="button"
                                role="tab"
                                data-panel-tab="pull"
                                aria-selected=move || (panel.get().surface == Some(PanelSurface::Pull)).to_string()
                                class=move || {
                                    let base = "inline-flex h-6 max-w-36 shrink-0 items-center gap-1.5 rounded-grape pl-1.5 pr-2 text-label-sm transition-colors";
                                    if panel.get().surface == Some(PanelSurface::Pull) {
                                        format!("{base} bg-longan-800 text-rambutan-100")
                                    } else {
                                        format!("{base} text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100")
                                    }
                                }
                                on:click=move |_| panel.update(|p| p.show(PanelSurface::Pull))
                            >
                                <Icon name="git-branch" size=12 class="shrink-0" />
                                <span class="truncate">{label}</span>
                            </button>
                        }
                    })
                }}
                {move || open_file.get().map(|file| {
                    let label = file.label;
                    view! {
                        <button
                            type="button"
                            role="tab"
                            data-panel-tab="file"
                            aria-selected=move || (panel.get().surface == Some(PanelSurface::File)).to_string()
                            class="inline-flex h-6 max-w-36 shrink-0 items-center gap-1.5 rounded-grape pl-1.5 pr-2 text-label-sm text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100"
                            on:click=move |_| panel.update(|p| p.show(PanelSurface::File))
                        >
                            <Icon name="file" size=12 class="shrink-0" />
                            <span class="truncate">{label}</span>
                        </button>
                    }
                })}
                <span class="flex-1"></span>
                <button
                    type="button"
                    aria-label="Close panel"
                    title="Close panel"
                    class="inline-flex size-6 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100"
                    on:click=move |_| close()
                >
                    <Icon name="x" size=14 />
                </button>
            </div>

            // Surface
            <div class="min-h-0 flex-1 overflow-hidden">
                {move || match surface.get() {
                    None => view! { <Launcher /> }.into_any(),
                    Some(PanelSurface::Diff) => view! { <DiffPanel /> }.into_any(),
                    Some(PanelSurface::Tasks) => view! { <TasksSurface /> }.into_any(),
                    Some(PanelSurface::Preview) => view! { <PreviewPanel /> }.into_any(),
                    Some(PanelSurface::Files) => view! { <FilesPanel /> }.into_any(),
                    Some(PanelSurface::Pull) => view! { <BranchPullPanel /> }.into_any(),
                    Some(PanelSurface::File) => view! { <FileViewer /> }.into_any(),
                }}
            </div>
        </aside>
    }
}

#[component]
fn PanelTab(surface: PanelSurface, icon: &'static str, label: &'static str) -> impl IntoView {
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();
    let active = Signal::derive(move || panel.get().surface == Some(surface));
    view! {
        <button
            type="button"
            role="tab"
            data-panel-tab=label.to_ascii_lowercase()
            aria-selected=move || active.get().to_string()
            class=move || {
                let base = "inline-flex h-6 max-w-36 shrink-0 items-center gap-1.5 rounded-grape pl-1.5 pr-2 text-label-sm transition-colors";
                if active.get() {
                    format!("{base} bg-longan-800 text-rambutan-100")
                } else {
                    format!("{base} text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100")
                }
            }
            on:click=move |_| panel.update(|p| p.show(surface))
        >
            <Icon name=icon size=12 class="shrink-0" />
            <span class="truncate">{label}</span>
        </button>
    }
}

/// The "Open a surface" grid, shown when the panel is open with no
/// surface selected. Single-key shortcuts work while the grid has focus.
#[component]
fn Launcher() -> impl IntoView {
    let RightPanelSignal(panel) = expect_context::<RightPanelSignal>();
    let on_key = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "d" | "D" => panel.update(|p| p.show(PanelSurface::Diff)),
        "t" | "T" => panel.update(|p| p.show(PanelSurface::Tasks)),
        "p" | "P" => panel.update(|p| p.show(PanelSurface::Preview)),
        "f" | "F" => panel.update(|p| p.show(PanelSurface::Files)),
        _ => {}
    };
    let card = "relative flex flex-col items-start rounded-lychee bg-longan-800/40 p-3 text-left inset-ring inset-ring-white/5 transition-colors hover:bg-longan-800/70 focus-visible:bg-longan-800/70";
    view! {
        <div
            role="group"
            aria-label="Open a surface"
            tabindex="0"
            class="flex h-full items-center justify-center p-6 focus:outline-none"
            on:keydown=on_key
        >
            <div class="w-full max-w-sm">
                <h3 class="text-center text-label text-rambutan-100">"Open a surface"</h3>
                <p class="mt-1 text-center text-label-sm text-rambutan-500">"Pick a panel or press its key."</p>
                <div class="mt-5 grid grid-cols-2 gap-2">
                    <button type="button" class=card on:click=move |_| panel.update(|p| p.show(PanelSurface::Diff))>
                        <Kbd class="absolute right-3 top-3">"D"</Kbd>
                        <span class="flex items-center gap-2 pe-8">
                            <Icon name="file-diff" size=16 class="text-rambutan-400" />
                            <span class="text-label text-rambutan-100">"Diff"</span>
                        </span>
                        <span class="mt-1.5 text-label-sm leading-relaxed text-rambutan-400">"File changes from this turn or the working tree."</span>
                    </button>
                    <button type="button" class=card on:click=move |_| panel.update(|p| p.show(PanelSurface::Tasks))>
                        <Kbd class="absolute right-3 top-3">"T"</Kbd>
                        <span class="flex items-center gap-2 pe-8">
                            <Icon name="activity" size=16 class="text-rambutan-400" />
                            <span class="text-label text-rambutan-100">"Tasks"</span>
                        </span>
                        <span class="mt-1.5 text-label-sm leading-relaxed text-rambutan-400">"Background shells the agent started."</span>
                    </button>
                    <button type="button" class=card on:click=move |_| panel.update(|p| p.show(PanelSurface::Preview))>
                        <Kbd class="absolute right-3 top-3">"P"</Kbd>
                        <span class="flex items-center gap-2 pe-8">
                            <Icon name="globe" size=16 class="text-rambutan-400" />
                            <span class="text-label text-rambutan-100">"Preview"</span>
                        </span>
                        <span class="mt-1.5 text-label-sm leading-relaxed text-rambutan-400">"A local dev server in a frame."</span>
                    </button>
                    <button type="button" class=card on:click=move |_| panel.update(|p| p.show(PanelSurface::Files))>
                        <Kbd class="absolute right-3 top-3">"F"</Kbd>
                        <span class="flex items-center gap-2 pe-8">
                            <Icon name="folder" size=16 class="text-rambutan-400" />
                            <span class="text-label text-rambutan-100">"Files"</span>
                        </span>
                        <span class="mt-1.5 text-label-sm leading-relaxed text-rambutan-400">"The project tree."</span>
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn BranchPullPanel() -> impl IntoView {
    let BranchPullSignal(branch_pull) = expect_context::<BranchPullSignal>();
    view! {
        <div data-branch-pull class="flex h-full min-h-0 flex-col gap-3 p-4" aria-label="Pull request">
            {move || match branch_pull.get() {
                None => view! {
                    <p class="text-label-sm text-rambutan-400">"No pull request for this branch."</p>
                }.into_any(),
                Some(pull) => {
                    let state = pull_state_label(&pull.state);
                    let heading = format!("#{} {}", pull.number, pull.title);
                    let href = github_pull_url(&pull.url).map(str::to_string);
                    view! {
                        <div class="min-w-0">
                            <p class="text-label-sm text-rambutan-500">{state}</p>
                            <h2 class="mt-1 text-label text-rambutan-100">{heading}</h2>
                        </div>
                        {href.map(|href| view! {
                            <a
                                href=href
                                target="_blank"
                                rel="noreferrer"
                                class="inline-flex h-8 items-center self-start rounded-grape px-2 text-label-sm text-rambutan-100 inset-ring inset-ring-white/10 hover:bg-longan-800/60"
                            >
                                "Open on GitHub"
                            </a>
                        })}
                    }.into_any()
                }
            }}
        </div>
    }
}

fn active_branch() -> Option<(String, String, String)> {
    let id = expect_context::<ActiveThreadId>().0.get_untracked()?;
    expect_context::<ThreadsSignal>()
        .0
        .with_untracked(|threads| {
            let thread = threads.iter().find(|thread| thread.id == id)?;
            let path = thread
                .worktree_path
                .clone()
                .filter(|path| !path.is_empty())
                .or_else(|| thread.project_path.clone().filter(|path| !path.is_empty()))?;
            let branch = thread.branch.clone().filter(|branch| !branch.is_empty())?;
            Some((thread.id.clone(), path, branch))
        })
}

/// Looks up the GitHub pull request for the active thread's branch and opens
/// the panel once per thread, branch, and number when `gh` is signed in.
#[component]
pub fn BranchPullWatch() -> impl IntoView {
    let threads = expect_context::<ThreadsSignal>().0;
    let active = expect_context::<ActiveThreadId>().0;
    let turns = expect_context::<Turns>();
    let panel = expect_context::<RightPanelSignal>().0;
    let BranchPullSignal(shown) = expect_context::<BranchPullSignal>();
    let opened = StoredValue::new(None::<String>);
    let ticket = StoredValue::new(0u32);

    Effect::new(move |_| {
        let _ = threads.get();
        let _ = active.get();
        // A turn starting or ending may have changed the branch.
        turns.track();
        let mine = ticket.with_value(|ticket| ticket.wrapping_add(1));
        ticket.set_value(mine);
        let Some((thread_id, path, branch)) = active_branch() else {
            shown.set(None);
            return;
        };
        leptos::task::spawn_local(async move {
            let result = branch_pull(&path, &branch).await;
            if ticket.with_value(|ticket| *ticket) != mine {
                return;
            }
            let Ok(lookup) = result else {
                return;
            };
            // A missing `gh` or a timeout sets `error` and does not open the panel.
            // A signed-out `gh` leaves `error` empty. Either way the panel stays put.
            let _lookup_error = lookup.error;
            let Some(pull) = lookup.pull else {
                shown.set(None);
                panel.update(|panel| {
                    if panel.surface == Some(PanelSurface::Pull) {
                        panel.surface = None;
                    }
                });
                return;
            };
            let key = branch_pull_key(&thread_id, &branch, pull.number);
            let open = opened.with_value(|opened| {
                should_open_branch_pull(
                    lookup.signed_in,
                    Some(pull.number),
                    opened.as_deref(),
                    &key,
                )
            });
            shown.set(Some(ShownBranchPull {
                key: key.clone(),
                number: pull.number,
                title: pull.title,
                url: pull.url,
                state: pull.state,
            }));
            if open {
                opened.set_value(Some(key));
                panel.update(|panel| panel.show(PanelSurface::Pull));
            }
        });
    });

    view! { <span class="hidden" data-branch-pull-watch></span> }
}
