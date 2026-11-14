//! Real-window rendering probe with synthetic data and no application services.
use crate::{assets, views::whiteboard::WhiteboardView};
use bruhs_models::{num::ToFloatLossy as _, whiteboard::*};
use gpui::*;
use gpui_component::Root;
use serde_json::json;
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Instant};

pub fn is_executable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_owned()))
        .is_some_and(|s| s == "bruhs-whiteboard-performance")
}
struct Run {
    view: Entity<WhiteboardView>,
    step: usize,
    frames: usize,
    count: usize,
    output: PathBuf,
    began: Instant,
    rows: Vec<serde_json::Value>,
    last: Instant,
}
fn next(run: Rc<RefCell<Run>>, window: &mut Window) {
    window.on_next_frame(move |window,cx| {
        let mut state=run.borrow_mut();
        let now=Instant::now();
        let elapsed=now.duration_since(state.last).as_secs_f64()*1000.0;
        let step=state.step;
        let frames=state.frames;
        let elapsed_ms=now.duration_since(state.began).as_secs_f64()*1000.0;
        state.rows.push(json!({"step":step,"phase":step/frames,"callback_interval_ms":elapsed,"elapsed_ms":elapsed_ms}));
        state.last=now;
        if step>=frames*3 {
            let shapes=state.view.read(cx).input.borrow().editor.document().shapes.len();
            let output=json!({"schema":1,"renderer":"gpui-paths-native","count":state.count,"shapes":shapes,"viewport":[f32::from(window.viewport_size().width),f32::from(window.viewport_size().height)],"dpr":window.scale_factor(),"timing_scope":"Display-link callback intervals include scheduling; these are not Metal GPU execution or presentation timestamps.","samples":state.rows});
            let result=serde_json::to_vec_pretty(&output).map_err(|e|e.to_string()).and_then(|bytes|std::fs::write(&state.output,bytes).map_err(|e|e.to_string()));
            if let Err(error)=result{eprintln!("WHITEBOARD_PERFORMANCE_ERROR {error}");}else{println!("WHITEBOARD_PERFORMANCE_DONE {}",state.output.display());}
            drop(state);cx.quit();return;
        }
        let t=(step%frames).to_f64_lossy()/(frames-1).to_f64_lossy();
        let phase=step/frames;
        state.view.update(cx,|view,cx| {
            let mut input=view.input.borrow_mut();
            let editor=&mut input.editor;
            match phase {
                0=>{editor.camera.x=400.0*t;editor.camera.y=200.0*t;}
                _=>{
                    if step.is_multiple_of(frames) {
                        editor.camera=Camera::default();
                        editor.selection=if phase==1 {std::collections::BTreeSet::from(["node-00000".into()])}else{editor.document().shapes.keys().cloned().collect()};
                        editor.start_move();
                    }
                    if let Err(error)=editor.preview_move(bruhs_models::whiteboard::Point::new(180.0*t,90.0*t)){eprintln!("WHITEBOARD_PERFORMANCE_ERROR {error}");cx.quit();return;}
                    if step%frames==frames-1 {if let Err(error)=editor.commit_move(0){eprintln!("WHITEBOARD_PERFORMANCE_ERROR {error}");cx.quit();return;}}
                }
            }
            cx.notify();
        });
        state.step+=1;
        drop(state);next(run,window);
    });
}
pub fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--editor") {
        let project = args
            .get(1)
            .ok_or("Pass --editor and an isolated project path")?;
        if std::env::var_os("BRUHS_DESKTOP_STATE_FILE").is_none() {
            return Err(
                "The isolated editor requires an explicit BRUHS_DESKTOP_STATE_FILE profile.".into(),
            );
        }
        return editor(project.clone());
    }
    if args.len() != 2 {
        return Err("Pass shape count and output JSON path".into());
    }
    let count = args[0].parse::<usize>().map_err(|e| e.to_string())?;
    if ![1000, 10000].contains(&count) {
        return Err("The rendering probe accepts 1000 or 10000 shapes".into());
    }
    let output = PathBuf::from(&args[1]);
    if output.exists() {
        return Err("Choose a new output path; existing evidence is never overwritten".into());
    }
    let editor = BoardEditor::new(benchmark_scene(count).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    gpui_platform::application()
        .with_assets(assets::DesktopAssets)
        .run(move |cx| {
            gpui_component::init(cx);
            if let Err(error) = cx.text_system().add_fonts(assets::embedded_fonts()) {
                eprintln!("WHITEBOARD_PERFORMANCE_ERROR {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
            let captured = Rc::new(RefCell::new(None));
            let capture = captured.clone();
            let window = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1280.0), px(800.0)),
                        cx,
                    ))),
                    titlebar: None,
                    ..Default::default()
                },
                move |window, cx| {
                    window.set_window_title("Bruhs whiteboard rendering probe (synthetic scene)");
                    let view = cx.new(|_| WhiteboardView::new(editor));
                    *capture.borrow_mut() = Some(view.clone());
                    cx.new(|cx| Root::new(view, window, cx))
                },
            );
            let (Ok(window), Some(view)) = (window, captured.borrow_mut().take()) else {
                eprintln!("WHITEBOARD_PERFORMANCE_ERROR Native window could not open");
                cx.quit();
                return;
            };
            let now = Instant::now();
            let run = Rc::new(RefCell::new(Run {
                view,
                step: 0,
                frames: 180,
                count,
                output,
                began: now,
                last: now,
                rows: Vec::new(),
            }));
            let _ = window.update(cx, |_, window, _| next(run, window));
        });
    Ok(())
}

fn editor(project: String) -> Result<(), String> {
    gpui_platform::application()
        .with_assets(assets::DesktopAssets)
        .run(move |cx| {
            gpui_component::init(cx);
            if let Err(error) = cx.text_system().add_fonts(assets::embedded_fonts()) {
                eprintln!("WHITEBOARD_EDITOR_ERROR {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1280.0), px(900.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                move |window, cx| {
                    window
                        .set_window_title("Bruhs whiteboard editor acceptance (isolated profile)");
                    let library = cx.new(|cx| {
                        crate::views::whiteboard::catalog::WhiteboardLibrary::new(
                            project, None, window, cx,
                        )
                    });
                    library.update(cx, |library, _| library.set_shown(true));
                    cx.new(|cx| Root::new(library, window, cx))
                },
            );
            if let Err(error) = result {
                eprintln!("WHITEBOARD_EDITOR_ERROR {error}");
                cx.quit();
            }
        });
    Ok(())
}
