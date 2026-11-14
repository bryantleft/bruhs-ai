//! Normal-platform workspace restoration with an explicit temporary auth program.
use super::auth_fixture::AuthFixture;
use super::clocks::CpuClock;
use super::runner::{self, Config};
use super::{FixtureMode, Service};
use crate::assets;
use crate::views::workspace::Workspace;
use gpui::*;
use gpui_component::Root;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

struct Run {
    config: Config,
    original: bool,
    workspace: Option<Entity<Workspace>>,
    window: Option<AnyWindowHandle>,
    setup_started: Instant,
    started: Option<Instant>,
    cpu_start: Option<CpuClock>,
    workspace_ms: f64,
    root_ms: f64,
    auth_ms: f64,
    auth_cpu: Option<Value>,
    collector: Option<gpui::profiler::FrameTimingCollector>,
    sample: Option<Value>,
    snapshot: Option<Value>,
    completion: Rc<RefCell<Option<Result<Value, String>>>>,
}

type SharedRun = Rc<RefCell<Run>>;

struct EmptyWindow;

impl Render for EmptyWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn start_when_active(run: SharedRun, window: &mut Window, cx: &mut App) {
    if !window.is_window_active() {
        assert!(
            run.borrow().setup_started.elapsed() < Duration::from_secs(300),
            "Native startup window did not activate"
        );
        window.activate_window();
        window.on_next_frame(move |window, cx| start_when_active(run, window, cx));
        return;
    }
    let started = Instant::now();
    let cpu_start = run.borrow().config.cpu.then(CpuClock::read);
    {
        let mut state = run.borrow_mut();
        state.started = Some(started);
        state.cpu_start = cpu_start;
    }
    let workspace = cx.new(|cx| Workspace::new(window, cx));
    let workspace_ms = started.elapsed().as_secs_f64() * 1_000.0;
    window.replace_root(cx, |window, cx| Root::new(workspace.clone(), window, cx));
    let root_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let mut state = run.borrow_mut();
    state.workspace = Some(workspace);
    state.workspace_ms = workspace_ms;
    state.root_ms = root_ms;
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn fake_program(directory: &Path) -> Result<String, String> {
    let path = directory.join("auth");
    // A newly executable file adds a first-launch cost unrelated to the app's
    // normal, already-installed CLI. Use the existing interpreter instead.
    let source = format!("[ \"$# $1\" = '1 status' ] || exit 99\nprintf '.\\n' >> {}\nprintf '%s\\n' '{{\"loggedIn\":true,\"email\":\"fixture@example.invalid\"}}'\n",quote(&directory.join("auth-calls")));
    fs::write(&path, source).map_err(|e| e.to_string())?;
    std::env::set_current_dir(directory).map_err(|e| e.to_string())?;
    Ok("/bin/sh".into())
}

fn delivered(run: SharedRun, cx: &mut App) {
    let ready = Instant::now();
    {
        let mut state = run.borrow_mut();
        state.auth_ms = ready
            .duration_since(state.started.expect("startup began"))
            .as_secs_f64()
            * 1_000.0;
        state.auth_cpu = state
            .cpu_start
            .as_ref()
            .map(|start| CpuClock::read().since(start));
    }
    cx.defer(move |cx| {
        let handle = run.borrow().window.expect("restored native window");
        handle
            .update(cx, |_, window, _| {
                runner::after_two_frames(window, move |window, cx| finish_frame(run, window, cx));
            })
            .expect("schedule the native startup frames");
    });
}

fn finish_frame(run: SharedRun, window: &mut Window, cx: &mut App) {
    let finished = Instant::now();
    let cpu_end = run.borrow().config.cpu.then(CpuClock::read);
    assert!(
        window.is_window_active(),
        "Native startup fixture must stay active"
    );
    let (workspace, mut sample, snapshot) = {
        let mut state = run.borrow_mut();
        let started = state.started.expect("startup began");
        let fixture = cx.global::<AuthFixture>();
        let expected_processes = if state.original {
            state.config.threads
        } else {
            1
        };
        assert_eq!(fixture.requested.get(), state.config.threads);
        assert_eq!(fixture.started.get(), expected_processes);
        assert_eq!(fixture.delivered.get(), state.config.threads);
        let calls = fs::read_to_string(state.config.directory.join("auth-calls"))
            .expect("fake CLI counter")
            .lines()
            .count();
        assert_eq!(calls, expected_processes, "actual fake CLI invocations");
        let workspace = state
            .workspace
            .as_ref()
            .expect("restored workspace")
            .clone();
        let snapshot = workspace
            .read(cx)
            .native_verify_startup_auth(state.config.threads, cx);
        let mut sample = json!({
            "workspace_ms":state.workspace_ms,"root_ms":state.root_ms,
            "all_auth_foreground_ms":state.auth_ms,
            "file_to_following_native_callback_ms":finished.duration_since(started).as_secs_f64()*1_000.0,
            "auth_requests":fixture.requested.get(),"auth_processes":calls,"auth_deliveries":fixture.delivered.get(),
            "viewport_width":f64::from(f32::from(window.viewport_size().width)),
            "viewport_height":f64::from(f32::from(window.viewport_size().height)),
            "scale_factor":f64::from(window.scale_factor()),
        });
        if let (Some(before), Some(after)) = (&state.cpu_start, cpu_end) {
            sample["auth_cpu"] = state.auth_cpu.clone().expect("auth CPU interval");
            sample["frame_cpu"] = after.since(before);
        }
        if let Some(collector) = &mut state.collector {
            let frames = collector
                .collect_unseen()
                .into_iter()
                .filter(|frame| {
                    frame.window_id == window.window_handle().window_id()
                        && frame.draw_start >= started
                        && frame.draw_end <= finished
                })
                .collect::<Vec<_>>();
            assert!(
                !frames.is_empty(),
                "A native draw must complete before the final callback"
            );
            sample["frames"] = json!(frames.iter().map(|frame| json!({"draw_ms":frame.draw_duration().as_secs_f64()*1_000.0,"since_start_draw_end_ms":frame.draw_end.duration_since(started).as_secs_f64()*1_000.0,"invalidations":frame.invalidations})).collect::<Vec<_>>());
        }
        (workspace, sample, snapshot)
    };
    sample["snapshot_sha256"] = json!(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).expect("snapshot bytes"))
    ));
    {
        let mut state = run.borrow_mut();
        state.sample = Some(sample);
        state.snapshot = Some(snapshot);
    }
    // Save acknowledgement and complete-model validation are outside the startup clocks.
    workspace.update(cx, |workspace, cx| {
        workspace.native_request_fixture_save(cx)
    });
    finish_save(run, window, cx);
}

fn finish_save(run: SharedRun, window: &mut Window, cx: &mut App) {
    let workspace = run.borrow().workspace.as_ref().expect("workspace").clone();
    if !workspace.read(cx).native_save_settled() {
        let state = run.borrow();
        assert!(
            state.started.expect("startup began").elapsed() < Duration::from_secs(300),
            "Native startup save did not settle"
        );
        let handle = state.window.expect("native window");
        drop(state);
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(25))
                .await;
            handle
                .update(cx, |_, window, cx| finish_save(run, window, cx))
                .expect("native save window");
        })
        .detach();
        return;
    }
    let state = run.borrow();
    let snapshot = state.snapshot.as_ref().expect("validated startup snapshot");
    assert_eq!(&workspace.read(cx).native_snapshot(cx), snapshot);
    let disk: Value = serde_json::from_slice(
        &fs::read(state.config.directory.join("desktop-state.json"))
            .expect("acknowledged temporary state"),
    )
    .expect("saved JSON");
    assert_eq!(
        &disk, snapshot,
        "complete acknowledged state must match the startup model"
    );
    let mode = cx.global::<FixtureMode>();
    let blocked = Service::ALL.map(|service| mode.blocked[service.index()].get());
    assert_eq!(
        blocked[0], 0,
        "only the explicit fake auth program bypasses startup blocking"
    );
    assert!(blocked[1..].iter().all(|&count| count > 0));
    let result = json!({"threads":state.config.threads,"strategy":if state.original {"original"} else {"shared"},"family":if state.config.cpu {"cpu"} else {"wall"},"sample":state.sample,"blocked_services":{"auth":blocked[0],"meetings":blocked[1],"preview_scan":blocked[2],"update_poll":blocked[3]},"scope":"File read/Workspace construction in an already active empty native window through all auth foreground delivery and two following native callbacks with an intervening draw/present opportunity. Platform,runtime,font and blank-window setup precede timing. Fake CLI shell includes a counter-file append and printf output;no real account/provider calls. CPU counters cover app foreground/process CPU,excluding child CPU. Save acknowledgement and full validation follow timing;no physical input,GPU completion,OS-cold,first-interactive or total app-launch latency claim."});
    println!("NATIVE_STARTUP_AUTH_RESULT {result}");
    *state.completion.borrow_mut() = Some(Ok(result));
    window.remove_window();
    cx.quit();
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let original = match args.first().map(String::as_str) {
        Some("original") => true,
        Some("shared") => false,
        _ => return Err("--startup-auth requires original or shared".into()),
    };
    let config = Config::parse(&args[1..])?;
    if config.samples != 1 || config.warmups != 0 {
        return Err(
            "Startup auth measures one restoration per process;use --samples 1 --warmups 0".into(),
        );
    }
    runner::prepare_input(&config)?;
    let program = fake_program(&config.directory)?;
    let completed = Rc::new(RefCell::new(None));
    let completion = completed.clone();
    gpui_platform::application()
        .with_assets(assets::DesktopAssets)
        .run(move |cx| {
            runner::initialize(&config, cx);
            let state = Rc::new(RefCell::new(Run {
                collector: config.cpu.then(gpui::profiler::FrameTimingCollector::new),
                config,
                original,
                workspace: None,
                window: None,
                setup_started: Instant::now(),
                started: None,
                cpu_start: None,
                workspace_ms: 0.0,
                root_ms: 0.0,
                auth_ms: 0.0,
                auth_cpu: None,
                sample: None,
                snapshot: None,
                completion,
            }));
            let callback = state.clone();
            cx.set_global(AuthFixture {
                program,
                original,
                expected: state.borrow().config.threads,
                requested: Cell::new(0),
                started: Cell::new(0),
                delivered: Cell::new(0),
                on_delivered: RefCell::new(Some(Box::new(move |cx| delivered(callback, cx)))),
            });
            let canceled = state.borrow().completion.clone();
            cx.activate(true);
            let handle = cx
                .open_window(
                    WindowOptions {
                        titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                        window_min_size: Some(size(px(640.0), px(480.0))),
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(1280.0), px(900.0)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    move |window, cx| {
                        window.set_window_title(
                            "bruhs - Native auth startup fixture (temporary data)",
                        );
                        window.on_window_should_close(cx, move |_, cx| {
                            if canceled.borrow().is_none() {
                                *canceled.borrow_mut() = Some(Err(
                                    "Native startup fixture closed before completion".into(),
                                ));
                            }
                            cx.quit();
                            true
                        });
                        cx.new(|_| EmptyWindow)
                    },
                )
                .expect("normal native startup window");
            state.borrow_mut().window = Some(handle.into());
            handle
                .update(cx, |_, window, _| {
                    window.activate_window();
                    runner::after_two_frames(window, move |window, cx| {
                        start_when_active(state, window, cx)
                    });
                })
                .expect("activate the empty native window");
        });
    completed
        .borrow_mut()
        .take()
        .ok_or("Native app ended without startup completion")??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn startup_auth_requires_an_explicit_strategy() {
        assert!(matches!(run(&[]), Err(error) if error.contains("original or shared")));
        assert!(
            matches!(run(&["other".into()]), Err(error) if error.contains("original or shared"))
        );
    }

    #[test]
    fn startup_auth_rejects_repeated_restorations_before_writing_state() {
        let directory = tempfile::Builder::new()
            .prefix("bruhs-native-perf-")
            .tempdir()
            .unwrap();
        let args = [
            "shared".into(),
            "--fixture-dir".into(),
            directory.path().to_str().unwrap().into(),
            "--threads".into(),
            "1".into(),
            "--samples".into(),
            "2".into(),
            "--warmups".into(),
            "0".into(),
        ];
        assert!(matches!(run(&args), Err(error) if error.contains("one restoration per process")));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
