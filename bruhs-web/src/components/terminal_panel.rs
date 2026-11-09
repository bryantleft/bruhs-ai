//! Terminal panel shared by the chat drawer (⌘J) and the
//! full-height Terminal view. Sessions are app-global: `window.bruhsTerminal`
//! (index.html) owns each xterm instance, its container element, and its
//! WebSocket to the server PTY for the life of the page, so a session
//! survives switching threads and views. The shells themselves live in the
//! server (`bruhs_core::terminal`), so a reload only re-attaches: the tab
//! list is persisted and reconciled in `state.rs`, and the xterm for a
//! restored tab is created on first attach. This component only decides
//! which session is attached to the mount on screen.

use bruhs_models::num::ToIntSaturating;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::{create_terminal_session, delete_terminal_session, BASE_URL};
use crate::components::banner::{BannerKind, BannerStackSignal};
use crate::state::{
    clamp_terminal_height, load_pref, next_active_after_close, save_pref, terminal_title,
    ActiveProject, ActiveProjectSignal, ActiveTerminalId, ProjectPickerMode,
    ProjectPickerModeSignal, RecentProjectsSignal, SetProjectPickerOpen, TerminalOpen,
    TerminalSession, TerminalSessions, TerminalStatusTick, TERMINAL_DEFAULT_HEIGHT,
};
use crate::ui::{Icon, Menu, MenuAlign, MenuItem, MenuSide};

const HEIGHT_PREF: &str = "bruhs.terminalHeight";
const XTERM_POLL_MS: u32 = 200;
/// How long after the bundle is requested it may take to load before the
/// terminal reports it unavailable instead of waiting forever.
const XTERM_LOAD_TIMEOUT_MS: f64 = 10_000.0;
static XTERM_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static XTERM_REQUESTED_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static XTERM_LOAD_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
const RECENT_PROJECTS_IN_MENU: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalPanelMode {
    /// Under the chat transcript; hidden, not unmounted, when toggled off.
    Drawer,
    /// The Terminal view: full height, own empty state.
    Page,
}

/// Percent-encode a query value (everything but RFC 3986 unreserved bytes).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// WebSocket URL that attaches to a server session: `http` → `ws`,
/// `https` → `wss`.
pub fn terminal_ws_url(base_url: &str, session_id: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!(
        "{ws_base}/api/terminal/ws?session={}",
        url_encode(session_id)
    )
}

fn glue() -> Option<JsValue> {
    let window = web_sys::window()?;
    let g = js_sys::Reflect::get(&window, &"bruhsTerminal".into()).ok()?;
    (!g.is_undefined() && !g.is_null()).then_some(g)
}

fn call(glue: &JsValue, name: &str, args: &[&JsValue]) -> Option<JsValue> {
    let f = js_sys::Reflect::get(glue, &name.into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    match args {
        [] => f.call0(glue),
        [a] => f.call1(glue, a),
        [a, b] => f.call2(glue, a, b),
        _ => return None,
    }
    .ok()
}

fn cite_terminal_selection(id: &str, title: &str) {
    let Some(g) = glue() else {
        return;
    };
    let excerpt = call(&g, "selection", &[&JsValue::from_str(id)])
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    let excerpt = excerpt.trim().to_string();
    if excerpt.is_empty() {
        return;
    }
    let label = format!("Terminal excerpt, {title}");
    crate::components::mention_editor::insert_chip_at_caret(
        bruhs_models::logic::context_chip::ChipKind::Terminal { label, excerpt },
    );
}

fn glue_ready() -> bool {
    glue()
        .and_then(|g| call(&g, "ready", &[]))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Milliseconds since the page started loading.
fn page_age_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// Whether the xterm bundle is usable, still loading, or never arrived.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum XtermLoad {
    Ready,
    Loading,
    Failed,
}

fn xterm_load(ready: bool, elapsed_ms: f64) -> XtermLoad {
    if ready {
        XtermLoad::Ready
    } else if elapsed_ms < XTERM_LOAD_TIMEOUT_MS {
        XtermLoad::Loading
    } else {
        XtermLoad::Failed
    }
}

/// Milliseconds since `request_xterm_bundle`, or 0 before that request.
fn xterm_elapsed_ms() -> f64 {
    use std::sync::atomic::Ordering;
    if !XTERM_REQUESTED.load(Ordering::Relaxed) {
        return 0.0;
    }
    let started = f64::from_bits(XTERM_REQUESTED_AT.load(Ordering::Relaxed));
    (page_age_ms() - started).max(0.0)
}

fn xterm_state() -> XtermLoad {
    xterm_load(glue_ready(), xterm_elapsed_ms())
}

/// Ask the page for xterm.js once. The load clock starts here, not at page
/// open, so a terminal opened late still gets the full wait.
fn request_xterm_bundle() {
    use std::sync::atomic::Ordering;
    if !XTERM_REQUESTED.swap(true, Ordering::Relaxed) {
        XTERM_REQUESTED_AT.store(page_age_ms().to_bits(), Ordering::Relaxed);
    }
    let already_started = XTERM_LOAD_STARTED.swap(true, Ordering::Relaxed);
    #[cfg(target_arch = "wasm32")]
    if !already_started {
        wasm_bindgen_futures::spawn_local(async {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Ok(loader) = js_sys::Reflect::get(&window, &JsValue::from_str("ensureXterm"))
            else {
                return;
            };
            let Ok(loader) = loader.dyn_into::<js_sys::Function>() else {
                return;
            };
            if let Ok(promise) = loader.call0(&JsValue::NULL) {
                if let Ok(promise) = promise.dyn_into::<js_sys::Promise>() {
                    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                }
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = already_started;
}

thread_local! {
    static STATUS_TICK: std::cell::Cell<Option<RwSignal<u32>>> = const { std::cell::Cell::new(None) };
}

fn bump_status_tick() {
    STATUS_TICK.with(|cell| {
        if let Some(tick) = cell.get() {
            let _ = tick.try_update(|value| *value = value.wrapping_add(1));
        }
    });
}

/// Spawn a shell on the server, then record the tab and open its xterm +
/// socket. Safe to call before the xterm bundle has loaded: the xterm is
/// created once `ready()` says yes.
pub fn open_terminal_session(
    sessions: RwSignal<Vec<TerminalSession>>,
    active: RwSignal<Option<String>>,
    cwd: Option<String>,
    project_name: Option<String>,
) {
    leptos::task::spawn_local(async move {
        match create_terminal_session(cwd.as_deref(), project_name.as_deref(), 120, 32).await {
            Ok(info) => {
                let session = TerminalSession {
                    id: info.id.clone(),
                    cwd: info.cwd.clone().or(cwd),
                    title: if info.title.is_empty() {
                        terminal_title(info.cwd.as_deref(), project_name.as_deref())
                    } else {
                        info.title.clone()
                    },
                    project_name: info.project_name.clone().or(project_name),
                };
                if sessions.try_update(|s| s.push(session)).is_none() {
                    return;
                }
                let _ = active.try_set(Some(info.id.clone()));
                start_when_ready(info.id.clone(), terminal_ws_url(BASE_URL, &info.id));
            }
            Err(err) => {
                web_sys::console::error_1(&format!("terminal: {err}").into());
            }
        }
    });
}

/// Make sure a restored tab has its xterm + socket. `false` while the
/// bundle is still loading.
fn ensure_runtime(g: &JsValue, id: &str) -> bool {
    let has = call(g, "has", &[&JsValue::from_str(id)])
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if has {
        return true;
    }
    if !glue_ready() {
        return false;
    }
    call(
        g,
        "create",
        &[
            &JsValue::from_str(id),
            &JsValue::from_str(&terminal_ws_url(BASE_URL, id)),
        ],
    );
    true
}

/// Create the session's xterm once the bundle is ready; gives up (the
/// status then says the terminal is unavailable) if it never loads.
fn start_when_ready(id: String, url: String) {
    match xterm_state() {
        XtermLoad::Ready => {
            bump_status_tick();
            if let Some(g) = glue() {
                call(
                    &g,
                    "create",
                    &[&JsValue::from_str(&id), &JsValue::from_str(&url)],
                );
            }
        }
        XtermLoad::Loading => {
            request_xterm_bundle();
            gloo_timers::callback::Timeout::new(XTERM_POLL_MS, move || start_when_ready(id, url))
                .forget();
        }
        XtermLoad::Failed => {
            bump_status_tick();
            log::error!("terminal: the xterm library did not load");
        }
    }
}

/// Drop the xterm + socket and kill the server-side shell. A shell the
/// server could not kill keeps running; say so, since the tab is gone.
fn close_session_runtime(id: &str, banners: Option<BannerStackSignal>) {
    if let Some(g) = glue() {
        call(&g, "close", &[&JsValue::from_str(id)]);
    }
    let id = id.to_string();
    leptos::task::spawn_local(async move {
        match delete_terminal_session(&id).await {
            // 404: the shell had already exited.
            Err(error) if !error.is_status(404) => {
                let message = format!(
                    "The shell could not be closed: {error} It may still be running; its tab returns after a reload."
                );
                log::warn!("terminal {id}: {message}");
                if let Some(banners) = banners {
                    banners.push(BannerKind::Warning, message, Some(8_000));
                }
            }
            _ => {}
        }
    });
}

/// What a terminal tab is doing, as `window.bruhsTerminal.status()` reports
/// it, plus the case where xterm itself never loaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TerminalStatus {
    Live,
    Ended,
    Failed,
    /// No xterm instance yet (the shell is being created).
    Starting,
    Connecting,
    /// The xterm library did not load, so no terminal can open.
    Unavailable,
}

impl TerminalStatus {
    fn parse(status: &str) -> Self {
        match status {
            "live" => Self::Live,
            "ended" => Self::Ended,
            "failed" => Self::Failed,
            "missing" => Self::Starting,
            _ => Self::Connecting,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::Ended => "Session ended",
            Self::Failed => "Connection failed",
            Self::Starting => "Starting shell…",
            Self::Connecting => "Connecting…",
            Self::Unavailable => "Terminal failed to load",
        }
    }
}

fn session_status(id: &str) -> TerminalStatus {
    let reported = glue()
        .and_then(|g| call(&g, "status", &[&JsValue::from_str(id)]))
        .and_then(|v| v.as_string());
    match (reported, xterm_state()) {
        (_, XtermLoad::Failed) => TerminalStatus::Unavailable,
        (Some(status), _) => TerminalStatus::parse(&status),
        (None, _) => TerminalStatus::Starting,
    }
}

/// Install the JS → Rust status callback once per page: bumps a tick so
/// status labels re-render.
fn install_status_hook(tick: RwSignal<u32>) {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    STATUS_TICK.with(|cell| cell.set(Some(tick)));
    if INSTALLED.with(|c| c.get()) {
        return;
    }
    let Some(g) = glue() else {
        return;
    };
    let cb = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_id: JsValue, _status: JsValue| {
        let _ = tick.try_update(|t| *t = t.wrapping_add(1));
    });
    call(&g, "onStatus", &[cb.as_ref().unchecked_ref()]);
    cb.forget();
    INSTALLED.with(|c| c.set(true));
}

/// Install the JS → Rust title callback once per page: a shell's OSC 0/2
/// title becomes the tab label (persisted with the tab).
fn install_title_hook(sessions: RwSignal<Vec<TerminalSession>>) {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if INSTALLED.with(|c| c.get()) {
        return;
    }
    let Some(g) = glue() else {
        return;
    };
    let cb = Closure::<dyn FnMut(JsValue, JsValue)>::new(move |id: JsValue, title: JsValue| {
        let (Some(id), Some(title)) = (id.as_string(), title.as_string()) else {
            return;
        };
        if title.trim().is_empty() {
            return;
        }
        let _ = sessions.try_update(|list| {
            if let Some(session) = list.iter_mut().find(|s| s.id == id) {
                session.title = title.clone();
            }
        });
    });
    call(&g, "onTitle", &[cb.as_ref().unchecked_ref()]);
    cb.forget();
    INSTALLED.with(|c| c.set(true));
}

type Listener = Closure<dyn Fn(web_sys::Event)>;

#[component]
pub fn TerminalPanel(
    mode: TerminalPanelMode,
    /// `(name, path)` of the surrounding context (the chat's project); the
    /// "+" menu offers it first and ⌘J defaults to it.
    #[prop(into)]
    context_project: Signal<Option<(String, String)>>,
) -> impl IntoView {
    let TerminalSessions(sessions) = expect_context::<TerminalSessions>();
    let ActiveTerminalId(active) = expect_context::<ActiveTerminalId>();
    let TerminalOpen(open) = expect_context::<TerminalOpen>();
    let TerminalStatusTick(tick) = expect_context::<TerminalStatusTick>();
    let RecentProjectsSignal(recent_projects) = expect_context::<RecentProjectsSignal>();
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();
    let SetProjectPickerOpen(set_picker_open) = expect_context::<SetProjectPickerOpen>();
    let ProjectPickerModeSignal(picker_mode) = expect_context::<ProjectPickerModeSignal>();

    install_status_hook(tick);
    install_title_hook(sessions);
    let banners = use_context::<BannerStackSignal>();
    // Re-render the status once the xterm load deadline passes, so a bundle
    // that never arrives shows as unavailable rather than "Starting…".
    if xterm_state() == XtermLoad::Loading {
        let wait = (XTERM_LOAD_TIMEOUT_MS - xterm_elapsed_ms())
            .max(0.0)
            .to_u32_saturating()
            .saturating_add(XTERM_POLL_MS);
        gloo_timers::callback::Timeout::new(wait, move || {
            let _ = tick.try_update(|t| *t = t.wrapping_add(1));
        })
        .forget();
    }

    let mount = NodeRef::<Div>::new();
    let attached = StoredValue::new(None::<String>);
    let new_menu_open = RwSignal::new(false);
    let is_drawer = mode == TerminalPanelMode::Drawer;
    let visible = Signal::derive(move || !is_drawer || open.get());

    let height = RwSignal::new(
        load_pref(HEIGHT_PREF)
            .and_then(|s| s.parse::<f64>().ok())
            .map_or(TERMINAL_DEFAULT_HEIGHT, clamp_terminal_height),
    );

    // Attach the active session's terminal to our mount; detach the previous.
    // Deferred callers may outlive this panel (view switch), so every
    // reactive read here is the non-panicking `try_*` form.
    let attach_active = move || {
        let Some(Some(el)) = mount.try_get_untracked() else {
            return;
        };
        let Some(g) = glue() else {
            return;
        };
        let Some(next) = active.try_get_untracked() else {
            return;
        };
        let previous = attached.try_get_value().flatten();
        if previous != next {
            if let Some(prev) = previous {
                call(&g, "detach", &[&JsValue::from_str(&prev)]);
            }
        }
        if let Some(id) = next {
            let el_js: JsValue = (*el).clone().into();
            if !ensure_runtime(&g, &id) {
                return;
            }
            call(&g, "attach", &[&JsValue::from_str(&id), &el_js]);
            let _ = attached.try_set_value(Some(id));
        } else {
            let _ = attached.try_set_value(None);
        }
    };
    Effect::new(move |_| {
        active.track();
        sessions.track();
        tick.track();
        if !visible.get() {
            return;
        }
        // Next tick: the mount must be laid out before xterm measures it.
        gloo_timers::callback::Timeout::new(0, attach_active).forget();
        // The bundle may still be loading; keep trying briefly.
        gloo_timers::callback::Timeout::new(XTERM_POLL_MS * 3, attach_active).forget();
    });
    on_cleanup(move || {
        if let (Some(id), Some(g)) = (attached.try_get_value().flatten(), glue()) {
            call(&g, "detach", &[&JsValue::from_str(&id)]);
        }
    });

    let create_for = move |cwd: Option<String>, name: Option<String>| {
        open_terminal_session(sessions, active, cwd, name);
        new_menu_open.set(false);
        if is_drawer {
            open.set(true);
        }
    };
    let choose_folder = move || {
        new_menu_open.set(false);
        picker_mode.set(ProjectPickerMode::NewTerminal);
        set_picker_open.set(true);
    };
    let close_session = move |id: String| {
        let next = sessions
            .with_untracked(|s| next_active_after_close(s, &id, active.get_untracked().as_deref()));
        close_session_runtime(&id, banners);
        sessions.update(|s| s.retain(|t| t.id != id));
        if attached.get_value().as_deref() == Some(id.as_str()) {
            attached.set_value(None);
        }
        active.set(next);
    };
    let select_session = move |id: String| active.set(Some(id));

    // Menu entries: context project, active project, recents, home, picker.
    let menu_projects = Signal::derive(move || {
        let mut out: Vec<ActiveProject> = Vec::new();
        if let Some((name, path)) = context_project.get() {
            out.push(ActiveProject { path, name });
        }
        if let Some(p) = active_project.get() {
            if !out.iter().any(|o| o.path == p.path) {
                out.push(p);
            }
        }
        for p in recent_projects
            .get()
            .into_iter()
            .take(RECENT_PROJECTS_IN_MENU)
        {
            if !out.iter().any(|o| o.path == p.path) {
                out.push(p);
            }
        }
        out
    });

    // ── Drawer resize ────────────────────────────────────────────────────────
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
        if let Some(h) = height.try_get_untracked() {
            save_pref(HEIGHT_PREF, &h.round().to_string());
        }
        gloo_timers::callback::Timeout::new(0, attach_active).forget();
    };
    let start_drag = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        stop_drag();
        let Some(window) = web_sys::window() else {
            return;
        };
        let start_y = f64::from(ev.client_y());
        let start_h = height.get_untracked();
        let on_move = Listener::new(move |raw: web_sys::Event| {
            if let Ok(pe) = raw.dyn_into::<web_sys::PointerEvent>() {
                let dy = start_y - f64::from(pe.client_y());
                let _ = height.try_set(clamp_terminal_height(start_h + dy));
            }
        });
        let on_up = Listener::new(move |_raw: web_sys::Event| {
            gloo_timers::callback::Timeout::new(0, stop_drag).forget();
        });
        let _ = window
            .add_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref());
        let _ =
            window.add_event_listener_with_callback("pointerup", on_up.as_ref().unchecked_ref());
        listeners.set_value(Some((on_move, on_up)));
    };

    let new_menu = move || {
        view! {
            <Menu open=new_menu_open align=MenuAlign::End side=if is_drawer { MenuSide::Top } else { MenuSide::Bottom } label="New terminal" class="min-w-[16rem]"
                trigger=move || view! {
                    <button
                        type="button"
                        data-terminal-new=""
                        aria-label="New terminal"
                        title="New terminal"
                        aria-haspopup="menu"
                        aria-expanded=move || new_menu_open.get().to_string()
                        class="inline-flex h-6 items-center gap-1 rounded-grape px-1.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 cursor-pointer"
                        on:click=move |_| new_menu_open.update(|v| *v = !*v)
                    >
                        <Icon name="plus" size=12 />
                        <span>"New"</span>
                    </button>
                }
            >
                {move || menu_projects.get().into_iter().map(|p| {
                    let path = p.path.clone();
                    let name = p.name.clone();
                    let desc = p.path.clone();
                    view! {
                        <MenuItem description=desc on_select=Callback::new(move |_| create_for(Some(path.clone()), Some(name.clone())))>
                            <span class="flex items-center gap-2"><Icon name="folder" size=12 class="text-rambutan-500" />{p.name}</span>
                        </MenuItem>
                    }
                }).collect_view()}
                <MenuItem description="~" on_select=Callback::new(move |_| create_for(None, None))>
                    <span class="flex items-center gap-2"><Icon name="terminal" size=12 class="text-rambutan-500" />"Home"</span>
                </MenuItem>
                <MenuItem on_select=Callback::new(move |_| choose_folder())>
                    <span class="flex items-center gap-2"><Icon name="folder-open" size=12 class="text-rambutan-500" />"Choose folder…"</span>
                </MenuItem>
            </Menu>
        }
    };

    let tabs = move || {
        view! {
            <div role="tablist" aria-label="Terminals" class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
                {move || sessions.get().into_iter().map(|s| {
                    let id = s.id.clone();
                    let id_close = s.id.clone();
                    let id_is = s.id.clone();
                    let is_active = Signal::derive(move || active.get().as_deref() == Some(id_is.as_str()));
                    view! {
                        <div
                            data-terminal-tab=s.id
                            class=move || format!(
                                "group/tab flex h-6 shrink-0 items-center gap-1 rounded-grape pl-2 pr-1 text-label-sm transition-colors {}",
                                if is_active.get() { "bg-longan-800 text-rambutan-100" } else { "text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100" }
                            )
                        >
                            <button
                                type="button"
                                role="tab"
                                aria-selected=move || is_active.get().to_string()
                                class="max-w-[10rem] truncate cursor-pointer"
                                title=s.cwd.clone().unwrap_or_else(|| "~".to_string())
                                on:click=move |_| select_session(id.clone())
                            >
                                {s.title.clone()}
                            </button>
                            <button
                                type="button"
                                aria-label="Close terminal"
                                class="inline-flex size-4 items-center justify-center rounded-seed text-rambutan-500 opacity-60 transition-opacity hover:text-lychee-300 group-hover/tab:opacity-100 cursor-pointer"
                                on:click=move |ev| { ev.stop_propagation(); close_session(id_close.clone()); }
                            >
                                <Icon name="x" size=10 />
                            </button>
                        </div>
                    }
                }).collect_view()}
            </div>
        }
    };

    let status = move || {
        tick.track();
        active.get().map(|id| session_status(&id))
    };

    let empty_state = move || {
        view! {
            <div data-terminal-empty="" class="flex h-full flex-col items-center justify-center gap-4 p-6 text-center">
                <Icon name="terminal" size=18 class="text-rambutan-400" />
                <div>
                    <h3 class="text-label text-rambutan-100">"Open a terminal"</h3>
                    <p class="mt-1 text-label-sm text-rambutan-500">"A login shell on this machine. Pick where it starts."</p>
                </div>
                <div class="flex w-full max-w-xs flex-col gap-1">
                    {move || menu_projects.get().into_iter().map(|p| {
                        let path = p.path.clone();
                        let name = p.name.clone();
                        let path_attr = p.path.clone();
                        view! {
                            <button
                                type="button"
                                data-terminal-empty-project=path_attr
                                class="flex w-full items-center justify-between gap-3 rounded-grape bg-longan-900 px-3 py-1.5 text-left text-label-sm text-rambutan-200 inset-ring inset-ring-white/5 transition-colors hover:bg-longan-800/70 cursor-pointer"
                                on:click=move |_| create_for(Some(path.clone()), Some(name.clone()))
                            >
                                <span class="truncate">{p.name}</span>
                                <span class="truncate text-rambutan-500">{p.path}</span>
                            </button>
                        }
                    }).collect_view()}
                    <button
                        type="button"
                        data-terminal-empty-home=""
                        class="flex w-full items-center justify-between gap-3 rounded-grape bg-longan-900 px-3 py-1.5 text-left text-label-sm text-rambutan-200 inset-ring inset-ring-white/5 transition-colors hover:bg-longan-800/70 cursor-pointer"
                        on:click=move |_| create_for(None, None)
                    >
                        <span>"Home"</span>
                        <span class="text-rambutan-500">"~"</span>
                    </button>
                    <button
                        type="button"
                        class="mt-1 text-label-sm text-rambutan-400 transition-colors hover:text-rambutan-100 cursor-pointer"
                        on:click=move |_| choose_folder()
                    >
                        "Choose a folder…"
                    </button>
                </div>
            </div>
        }
    };

    let has_sessions = Signal::derive(move || !sessions.with(|s| s.is_empty()));

    view! {
        <section
            data-terminal-panel=""
            data-terminal-mode=if is_drawer { "drawer" } else { "page" }
            aria-label="Terminal"
            class=move || format!(
                "relative flex flex-col bg-longan-950 {}",
                if is_drawer { "shrink-0 border-t border-longan-800/70" } else { "h-full min-h-0 flex-1" }
            )
            class:hidden=move || !visible.get()
            style=move || if is_drawer { format!("height:{}px", height.get().round()) } else { String::new() }
        >
            {is_drawer.then(|| view! {
                <div
                    role="separator"
                    aria-orientation="horizontal"
                    aria-label="Resize terminal"
                    class="absolute inset-x-0 top-0 z-10 h-1 cursor-row-resize transition-colors hover:bg-persimmon-400/40"
                    on:pointerdown=start_drag
                ></div>
            })}
            <div class="flex h-8 shrink-0 items-center gap-2 px-2 text-label-sm">
                <Icon name="terminal" size=12 class="shrink-0 text-rambutan-400" />
                {tabs}
                {new_menu}
                {move || {
                    let id = active.get()?;
                    let title = sessions.with(|items| {
                        items
                            .iter()
                            .find(|session| session.id == id)
                            .map(|session| session.title.clone())
                    })?;
                    Some(view! {
                        <button
                            type="button"
                            data-terminal-cite=""
                            aria-label="Cite in composer"
                            title="Cite the selection in the composer"
                            class="inline-flex h-6 shrink-0 items-center rounded-grape px-1.5 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 cursor-pointer"
                            on:click=move |_| cite_terminal_selection(&id, &title)
                        >
                            "Cite"
                        </button>
                    })
                }}
                <span data-terminal-status="" class=move || match status() {
                    Some(TerminalStatus::Live) => "shrink-0 text-pandan-400",
                    Some(TerminalStatus::Unavailable) => "shrink-0 text-lychee-300",
                    _ => "shrink-0 text-rambutan-500",
                }>
                    {move || status().map_or("", TerminalStatus::label)}
                </span>
                {is_drawer.then(|| view! {
                    <button
                        type="button"
                        aria-label="Close terminal"
                        title="Hide terminal (⌘J)"
                        class="inline-flex size-6 shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 cursor-pointer"
                        on:click=move |_| open.set(false)
                    >
                        <Icon name="x" size=12 />
                    </button>
                })}
            </div>
            <div class="relative min-h-0 flex-1">
                <div
                    node_ref=mount
                    data-terminal-mount=""
                    class="h-full w-full overflow-hidden px-2 pb-1"
                    class:hidden=move || !has_sessions.get()
                    on:click=move |_| {
                        if let (Some(id), Some(g)) = (active.get_untracked(), glue()) {
                            call(&g, "focus", &[&JsValue::from_str(&id)]);
                        }
                    }
                ></div>
                {move || (!has_sessions.get()).then(empty_state)}
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_swaps_scheme_and_escapes() {
        assert_eq!(
            terminal_ws_url("http://127.0.0.1:3001", "abc-123"),
            "ws://127.0.0.1:3001/api/terminal/ws?session=abc-123"
        );
        assert_eq!(
            terminal_ws_url("https://api.bruhs.localhost/", "a b"),
            "wss://api.bruhs.localhost/api/terminal/ws?session=a%20b"
        );
    }

    #[test]
    fn url_encode_keeps_unreserved() {
        assert_eq!(url_encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(url_encode("é&="), "%C3%A9%26%3D");
    }

    #[test]
    fn status_labels() {
        let label = |s: &str| TerminalStatus::parse(s).label();
        assert_eq!(label("live"), "Live");
        assert_eq!(label("ended"), "Session ended");
        assert_eq!(label("failed"), "Connection failed");
        assert_eq!(label("missing"), "Starting shell…");
        assert_eq!(label("connecting"), "Connecting…");
        assert_eq!(
            TerminalStatus::Unavailable.label(),
            "Terminal failed to load"
        );
    }

    /// The wait for the xterm bundle is bounded: past the deadline a bundle
    /// that never loaded is a visible failure, not an endless "Starting…".
    #[test]
    fn the_xterm_wait_is_bounded() {
        assert_eq!(xterm_load(true, 60_000.0), XtermLoad::Ready);
        assert_eq!(xterm_load(false, 500.0), XtermLoad::Loading);
        assert_eq!(xterm_load(false, XTERM_LOAD_TIMEOUT_MS), XtermLoad::Failed);
    }
}
