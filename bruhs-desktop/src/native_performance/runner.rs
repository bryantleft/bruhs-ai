use super::{clocks::CpuClock, FixtureMode, Service};
use crate::views::workspace::{
    native_fixture::{self, NativeAction},
    Workspace,
};
use crate::{app_state::DesktopState, assets, theme};
use gpui::*;
use gpui_component::zoom::px;
use gpui_component::Root;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct Config {
    pub(super) directory: PathBuf,
    pub(super) threads: usize,
    pub(super) samples: usize,
    pub(super) warmups: usize,
    pub(super) cpu: bool,
}

impl Config {
    pub(super) fn parse(args: &[String]) -> Result<Self, String> {
        let mut directory = None;
        let mut threads = None;
        let mut samples = 30;
        let mut warmups = 5;
        let mut cpu = false;
        let mut iter = args.iter();
        while let Some(flag) = iter.next() {
            match flag.as_str() {
                "--cpu" => cpu = true,
                "--fixture-dir" => {
                    directory = Some(PathBuf::from(
                        iter.next().ok_or("Missing fixture directory")?,
                    ))
                }
                "--threads" | "--samples" | "--warmups" => {
                    let value: usize = iter
                        .next()
                        .ok_or("Missing numeric option")?
                        .parse()
                        .map_err(|_| "Invalid numeric option")?;
                    match flag.as_str() {
                        "--threads" => threads = Some(value),
                        "--samples" => samples = value,
                        _ => warmups = value,
                    }
                }
                _ => return Err(format!("Unknown native fixture option {flag}")),
            }
        }
        let directory = directory
            .ok_or("--fixture-dir is required")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let temporary = std::env::temp_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if directory.parent() != Some(temporary.as_path())
            || !directory
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("bruhs-native-perf-"))
        {
            return Err(
                "Use a new bruhs-native-perf-* directory directly in the OS temporary directory"
                    .into(),
            );
        }
        if fs::read_dir(&directory)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        {
            return Err("The native fixture directory must be empty".into());
        }
        let threads = threads.ok_or("--threads is required")?;
        if !(1..=2048).contains(&threads) || !(1..=100).contains(&samples) || warmups > 10 {
            return Err("Native fixture counts exceed their supported bounds".into());
        }
        Ok(Self {
            directory,
            threads,
            samples,
            warmups,
            cpu,
        })
    }
}

struct Run {
    config: Config,
    workspace: Entity<Workspace>,
    window: WindowHandle<Root>,
    initial: Value,
    action_index: usize,
    iteration: usize,
    rows: BTreeMap<String, Vec<Value>>,
    collector: Option<gpui::profiler::FrameTimingCollector>,
    completion: Rc<RefCell<Option<Result<Value, String>>>>,
    began: Instant,
}

type SharedRun = Rc<RefCell<Run>>;

// This dependency invokes each callback before that request's draw. A second
// callback is required to observe the intervening native draw/present opportunity.
pub(super) fn after_two_frames(
    window: &Window,
    callback: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    window.on_next_frame(move |window, _| window.on_next_frame(callback));
}

fn prepare(run: SharedRun, window: &mut Window, cx: &mut App) {
    let (workspace, action) = {
        let state = run.borrow();
        (
            state.workspace.clone(),
            NativeAction::ALL[state.action_index],
        )
    };
    workspace.update(cx, |workspace, cx| {
        workspace.prepare_native_action(action, window, cx)
    });
    after_two_frames(window, move |_, cx| {
        let handle = run.borrow().window;
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2))
                .await;
            handle
                .update(cx, |_, window, cx| start_sample(run, window, cx))
                .expect("native fixture window");
        })
        .detach();
    });
}

fn start_sample(run: SharedRun, window: &mut Window, cx: &mut App) {
    let (workspace, action, cpu) = {
        let mut state = run.borrow_mut();
        if let Some(collector) = &mut state.collector {
            collector.collect_unseen();
        }
        (
            state.workspace.clone(),
            NativeAction::ALL[state.action_index],
            state.config.cpu,
        )
    };
    assert!(
        window.is_window_active(),
        "Native fixture must stay active during measured input"
    );
    let epoch = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("epoch")
            .as_nanos(),
    )
    .expect("epoch nanoseconds");
    let started = Instant::now();
    let cpu_start = cpu.then(CpuClock::read);
    workspace.update(cx, |workspace, cx| {
        workspace.dispatch_native_action(action, window, cx)
    });
    let dispatch_ms = started.elapsed().as_secs_f64() * 1_000.0;
    after_two_frames(window, move |window, cx| {
        let finished = Instant::now();
        let cpu_end = cpu.then(CpuClock::read);
        let (initial, workspace, iteration, warmups, window_id) = {
            let state = run.borrow();
            (
                state.initial.clone(),
                state.workspace.clone(),
                state.iteration,
                state.config.warmups,
                state.window.window_id(),
            )
        };
        assert!(window.is_window_active());
        let validation = workspace
            .read(cx)
            .verify_native_action(action, &initial, cx);
        let mut sample = json!({"iteration":iteration,"warmup":iteration<warmups,"unix_start_ns":epoch,
            "dispatch_ms":dispatch_ms,"handler_to_following_callback_ms":finished.duration_since(started).as_secs_f64()*1_000.0,
            "validation":validation,"viewport_width":f64::from(f32::from(window.viewport_size().width)),"viewport_height":f64::from(f32::from(window.viewport_size().height)),"scale_factor":f64::from(window.scale_factor())});
        if let (Some(before), Some(after)) = (cpu_start, cpu_end) {
            sample["cpu"] = after.since(&before);
        }
        {
            let mut state = run.borrow_mut();
            if let Some(collector) = &mut state.collector {
                let frames = collector
                    .collect_unseen()
                    .into_iter()
                    .filter(|f| {
                        f.window_id == window_id
                            && f.draw_start >= started
                            && f.draw_end <= finished
                    })
                    .collect::<Vec<_>>();
                assert!(
                    !frames.is_empty(),
                    "A native draw must complete between input and the final callback"
                );
                sample["frames"] = json!(frames.iter().map(|f|json!({"draw_ms":f.draw_duration().as_secs_f64()*1_000.0,
                    "input_to_draw_end_ms":f.draw_end.duration_since(started).as_secs_f64()*1_000.0,"invalidations":f.invalidations})).collect::<Vec<_>>());
            }
            state
                .rows
                .entry(action.label().into())
                .or_default()
                .push(sample);
            state.iteration += 1;
            if state.iteration == state.config.samples + state.config.warmups {
                println!(
                    "NATIVE_ACTION_COMPLETE {}",
                    json!({"action":action.label(),"samples":state.config.samples,"warmups":state.config.warmups})
                );
                state.iteration = 0;
                state.action_index += 1;
            }
        }
        if run.borrow().action_index == NativeAction::ALL.len() {
            finish(run, window, cx);
        } else {
            prepare(run, window, cx);
        }
    });
}

fn finish(run: SharedRun, window: &mut Window, cx: &mut App) {
    let (workspace, handle, began) = {
        let s = run.borrow();
        (s.workspace.clone(), s.window, s.began)
    };
    if !workspace.read(cx).native_save_settled() {
        assert!(
            began.elapsed() < Duration::from_secs(300),
            "Native fixture save did not settle"
        );
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(25))
                .await;
            handle
                .update(cx, |_, window, cx| finish(run, window, cx))
                .expect("native save window");
        })
        .detach();
        return;
    }
    let state = run.borrow();
    let final_snapshot = workspace.read(cx).native_snapshot(cx);
    let path = state.config.directory.join("desktop-state.json");
    let disk: Value = serde_json::from_slice(&fs::read(&path).expect("temporary saved state"))
        .expect("saved fixture JSON");
    assert_eq!(
        disk, final_snapshot,
        "final temporary save must equal the complete native snapshot"
    );
    let mode = cx.global::<FixtureMode>();
    let blocked = Service::ALL.map(|service| mode.blocked[service.index()].get());
    assert!(blocked[0] >= u64::try_from(state.config.threads).expect("thread count"));
    assert!(blocked[1..].iter().all(|&count| count > 0));
    let result = json!({"threads":state.config.threads,"samples":state.config.samples,"warmups":state.config.warmups,
        "family":if state.config.cpu {"cpu"} else {"wall"},"rows":state.rows,"blocked_services":{"auth":blocked[0],"meetings":blocked[1],"preview_scan":blocked[2],"update_poll":blocked[3]},
        "final_snapshot_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&final_snapshot).expect("snapshot bytes"))),
        "scope":"Normal native GPUI platform,embedded Monaspace fonts,actual Workspace/Root,temporary persistence,in-memory credentials/connections. Synthetic input/action dispatch through the following native frame callback after an intervening draw/present opportunity. CPU family additionally enables frame tracing/counters. No OS input dispatch or physical presentation timing;GPU work and save acknowledgement are not included as separate accepted latency boundaries."});
    // Native shutdown may terminate the process without returning from Application::run.
    println!("NATIVE_PERFORMANCE_RESULT {result}");
    *state.completion.borrow_mut() = Some(Ok(result));
    window.remove_window();
    cx.quit();
}

pub(super) fn prepare_input(config: &Config) -> Result<(), String> {
    let path = config.directory.join("desktop-state.json");
    let bytes = native_fixture::saved_bytes(config.threads, "/synthetic/native-performance")
        .map_err(|e| e.to_string())?;
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    std::env::set_var("BRUHS_DESKTOP_STATE_FILE", &path);
    // The fixture never addresses the user's local server or daemon, even if a
    // future view attempts a request outside the disabled startup paths.
    std::env::set_var("BRUHS_SERVER_URL", "http://127.0.0.1:0");
    std::env::set_var("BRUHS_CAPTURE_URL", "http://127.0.0.1:0");
    std::env::set_var(
        "BRUHS_CAPTURE_TOKEN_FILE",
        config.directory.join("no-capture-token"),
    );
    if std::env::var_os("BRUHS_DESKTOP_SMOKE_PROMPT").is_some() {
        return Err("Clear the real-provider smoke prompt before native fixtures".into());
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        std::process::exit(101)
    }));
    println!(
        "NATIVE_FIXTURE_INPUT {}",
        json!({"input_bytes":bytes.len(),"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"cfg_test":cfg!(test),"binary":option_env!("CARGO_BIN_NAME"),"threads":config.threads})
    );
    Ok(())
}

pub(super) fn initialize(config: &Config, cx: &mut App) {
    gpui_component::init(cx);
    crate::motion::init(cx);
    cx.text_system()
        .add_fonts(assets::embedded_fonts())
        .expect("embedded native fonts");
    let names = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|n| n.starts_with("Monaspace"))
        .collect::<Vec<_>>();
    assert!(
        !names.is_empty(),
        "The native text backend must expose embedded Monaspace families"
    );
    gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
    theme::apply_bruhs_theme(cx);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("bruhs-native-fixture")
        .build()
        .expect("fixture runtime");
    let calendar = bruhs_core::google_calendar::GoogleCalendarHandle::new(
        None,
        bruhs_core::google_calendar::tokens::TokenStore::in_memory(),
        bruhs_core::google_calendar::cursor::CalendarFiles::at(config.directory.join("calendar")),
    );
    let app = bruhs_core::AppState::builder()
        .thread_waits(true)
        .secret_store(bruhs_core::secrets::SecretStore::in_memory())
        .connections(bruhs_core::connections::Connections::in_memory())
        .google_calendar(Arc::new(calendar))
        .build();
    cx.set_global(DesktopState {
        app: Arc::new(app),
        runtime: Arc::new(runtime),
    });
    cx.set_global(FixtureMode::default());
    gpui::profiler::set_frame_trace_enabled(config.cpu);
    println!(
        "NATIVE_PLATFORM_READY {}",
        json!({"font_families":names,"frame_trace":config.cpu,"scope":"Native platform;GPUI test-support absence is verified from linked Cargo artifacts before launch."})
    );
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    if args.first().is_some_and(|arg| arg == "--startup-auth") {
        return super::startup_auth::run(&args[1..]);
    }
    let config = Config::parse(args)?;
    prepare_input(&config)?;
    let completed = Rc::new(RefCell::new(None));
    let completion = completed.clone();
    gpui_platform::application()
        .with_assets(assets::DesktopAssets)
        .run(move |cx| {
            initialize(&config, cx);
            let captured = Rc::new(RefCell::new(None));
            let capture = captured.clone();
            let canceled = completion.clone();
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
                    |window, cx| {
                        window.set_window_title(
                            "bruhs - Native performance fixture (temporary data)",
                        );
                        window.on_window_should_close(cx, move |_, cx| {
                            if canceled.borrow().is_none() {
                                *canceled.borrow_mut() = Some(Err(
                                    "Native fixture window closed before completion".into(),
                                ));
                            }
                            cx.quit();
                            true
                        });
                        let workspace = cx.new(|cx| Workspace::new(window, cx));
                        *capture.borrow_mut() = Some(workspace.clone());
                        cx.new(|cx| Root::new(workspace, window, cx))
                    },
                )
                .expect("native fixture window");
            let workspace = captured.borrow_mut().take().expect("native workspace");
            let initial = workspace.read(cx).native_snapshot(cx);
            let state = Rc::new(RefCell::new(Run {
                collector: config.cpu.then(gpui::profiler::FrameTimingCollector::new),
                config,
                workspace,
                window: handle,
                initial,
                action_index: 0,
                iteration: 0,
                rows: BTreeMap::new(),
                completion,
                began: Instant::now(),
            }));
            handle
                .update(cx, |_, window, cx| prepare(state, window, cx))
                .expect("prepare native fixture");
        });
    completed
        .borrow_mut()
        .take()
        .ok_or("Native application ended without a completed fixture")??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Config;

    fn args(directory: &std::path::Path) -> Vec<String> {
        vec![
            "--fixture-dir".into(),
            directory.to_str().expect("fixture path").into(),
            "--threads".into(),
            "1".into(),
        ]
    }

    #[test]
    fn refuses_an_existing_directory_containing_state() {
        let temporary = tempfile::Builder::new()
            .prefix("bruhs-native-perf-")
            .tempdir()
            .expect("fixture directory");
        std::fs::write(temporary.path().join("desktop-state.json"), b"existing")
            .expect("fixture file");
        assert!(
            matches!(Config::parse(&args(temporary.path())), Err(error) if error.contains("must be empty"))
        );
    }

    #[test]
    fn refuses_directories_outside_the_dedicated_temporary_namespace() {
        let temporary = tempfile::Builder::new()
            .prefix("other-fixture-")
            .tempdir()
            .expect("fixture directory");
        assert!(
            matches!(Config::parse(&args(temporary.path())), Err(error) if error.contains("bruhs-native-perf-*"))
        );
    }

    #[test]
    fn refuses_an_unbounded_sample_count() {
        let temporary = tempfile::Builder::new()
            .prefix("bruhs-native-perf-")
            .tempdir()
            .expect("fixture directory");
        let mut args = args(temporary.path());
        args.extend(["--samples".into(), "101".into()]);
        assert!(matches!(Config::parse(&args), Err(error) if error.contains("supported bounds")));
    }
}
