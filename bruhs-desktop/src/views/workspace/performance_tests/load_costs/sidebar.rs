//! Native sidebar input and complete headless frame settling; no provider or save activity.
use super::super::super::{element_key, MessageRole};
use super::*;
use gpui::{px, size};

#[derive(Clone, Copy)]
struct Case {
    label: &'static str,
    threads: usize,
    code: bool,
    mixed: bool,
    settled_open: bool,
    width: f32,
    height: f32,
}

const CASES: [Case; 6] = [
    Case {
        label: "chat-8-active",
        threads: 8,
        code: false,
        mixed: false,
        settled_open: true,
        width: 1280.0,
        height: 900.0,
    },
    Case {
        label: "chat-128-active",
        threads: 128,
        code: false,
        mixed: false,
        settled_open: true,
        width: 1280.0,
        height: 900.0,
    },
    Case {
        label: "chat-512-active",
        threads: 512,
        code: false,
        mixed: false,
        settled_open: true,
        width: 1280.0,
        height: 900.0,
    },
    Case {
        label: "chat-512-mixed-open",
        threads: 512,
        code: false,
        mixed: true,
        settled_open: true,
        width: 1280.0,
        height: 640.0,
    },
    Case {
        label: "code-512-mixed-closed",
        threads: 512,
        code: true,
        mixed: true,
        settled_open: false,
        width: 1280.0,
        height: 900.0,
    },
    Case {
        label: "code-2048-active",
        threads: 2048,
        code: true,
        mixed: false,
        settled_open: true,
        width: 1440.0,
        height: 900.0,
    },
];

fn scene_state(case: Case) -> PersistedWorkspaceState {
    let mut state = saved_state("sidebar-actions", 0, 0, 0);
    state.threads = (0..case.threads)
        .map(|index| {
            let mut thread = saved_thread(index, 2);
            thread.title = format!("Saved thread {index}: café 👩🏽‍💻");
            thread.project_name = if case.code {
                format!("Project {}", index % 8)
            } else {
                String::new()
            };
            thread.project_path = None;
            // A fixed future activity keeps synthetic active rows reproducible on later runs.
            thread.updated_at_ms = bruhs_models::EpochMs::new(4_000_000_000_000).unwrap();
            if case.mixed {
                thread.pinned = index.is_multiple_of(4);
                thread.settled = index % 4 == 1;
                thread.snoozed_until_ms = (index % 4 == 2).then_some(4_100_000_000_000);
            }
            thread.messages[0].content = "Saved question: café, 日本語 and 👩🏽‍💻.".into();
            thread.messages[1].role = MessageRole::Assistant;
            thread.messages[1].content =
                "### Saved response\n\nA short saved result with **Markdown**, café and 日本語."
                    .into();
            thread
        })
        .collect();
    state.active_thread_id = Some("thread-0".into());
    state.active_view = Some(AppView::Chat);
    state.sidebar_visible = Some(true);
    state.sidebar_settled_open = Some(case.settled_open);
    state.sidebar_snoozed_open = Some(true);
    state.sidebar_working_open = Some(true);
    state
}

struct Scene {
    workspace: Entity<Workspace>,
    root: Entity<gpui_component::Root>,
    input_bytes: usize,
    input_sha256: String,
    selectors: Vec<&'static str>,
}

fn mount(cx: &mut TestAppContext, case: Case) -> (Scene, &mut VisualTestContext) {
    let state = scene_state(case);
    let bytes = serde_json::to_vec(&state).unwrap();
    let input_bytes = bytes.len();
    let input_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (root, visual) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::from_loaded_state(
                (Some(state), None, super::super::super::DiskVersion::Missing),
                window,
                cx,
                current,
                threads::current_threads,
            )
        });
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow_mut().take().unwrap();
    visual.simulate_resize(size(px(case.width), px(case.height)));
    workspace.update_in(visual, |workspace, window, cx| {
        workspace
            .sidebar_search
            .update(cx, |input, cx| input.focus(window, cx));
    });
    visual.run_until_parked();
    let mut selectors: Vec<&'static str> = vec![
        "desktop-workspace-sidebar",
        "desktop-sidebar-search",
        "desktop-sidebar-threads",
        "desktop-sidebar-footer",
        "desktop-composer",
    ];
    for index in 0..case.threads {
        let key = element_key(&format!("thread-{index}"));
        for prefix in [
            "desktop-thread-row",
            "desktop-thread-card-title",
            "desktop-thread-card-project",
        ] {
            // GPUI's test API requires static selectors; allocate once per scene outside timing.
            selectors.push(Box::leak(format!("{prefix}-{key}").into_boxed_str()));
        }
    }
    (
        Scene {
            workspace,
            root,
            input_bytes,
            input_sha256,
            selectors,
        },
        visual,
    )
}

#[derive(Clone, Copy)]
enum Action {
    Redraw,
    SearchAll,
    ClearAll,
    SearchNone,
    ClearNone,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Redraw => "redraw",
            Self::SearchAll => "search-all",
            Self::ClearAll => "clear-all",
            Self::SearchNone => "search-none",
            Self::ClearNone => "clear-none",
        }
    }
    fn query(self) -> &'static str {
        match self {
            Self::SearchAll => "Saved",
            Self::SearchNone => "absentquartz",
            _ => "",
        }
    }
}

const ACTIONS: [Action; 5] = [
    Action::Redraw,
    Action::SearchAll,
    Action::ClearAll,
    Action::SearchNone,
    Action::ClearNone,
];

#[inline(never)]
fn interact(scene: &Scene, visual: &mut VisualTestContext, action: Action) -> f64 {
    let started = Instant::now();
    match action {
        Action::Redraw => scene.workspace.update(visual, |_, cx| cx.notify()),
        Action::SearchAll | Action::SearchNone => visual.simulate_input(action.query()),
        Action::ClearAll | Action::ClearNone => visual.simulate_keystrokes("secondary-a backspace"),
    }
    visual.run_until_parked();
    started.elapsed().as_secs_f64() * 1000.0
}

fn snapshot(scene: &Scene, visual: &VisualTestContext, case: Case, query: &str) -> Value {
    scene.workspace.read_with(visual, |workspace, cx| {
        assert_eq!(workspace.threads.len(), case.threads);
        assert_eq!(workspace.active_thread_id.as_deref(), Some("thread-0"));
        assert_eq!(workspace.sidebar_search.read(cx).value().as_str(), query);
        assert_eq!(workspace.persist_requests, 0);
        assert!(workspace.threads.iter().all(|thread| {
            let chat = thread.chat.read(cx);
            !chat.is_streaming && chat.prompt_queue.is_empty()
        }));
        serde_json::to_value(workspace.snapshot(cx)).unwrap()
    })
}

fn geometry(scene: &Scene, visual: &mut VisualTestContext, query: &str) -> Value {
    let bounds = scene
        .selectors
        .iter()
        .copied()
        .map(|selector| {
            let bounds = visual.debug_bounds(selector).map(|b| {
                [
                    f32::from(b.origin.x),
                    f32::from(b.origin.y),
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                ]
            });
            (selector.to_string(), json!(bounds))
        })
        .collect::<serde_json::Map<_, _>>();
    let focused = scene.workspace.update_in(visual, |workspace, window, cx| {
        use gpui::Focusable;
        workspace
            .sidebar_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "sidebar input keeps focus after {query:?}");
    assert!(visual.debug_bounds("desktop-composer").is_some());
    json!({"bounds":bounds,"search_focused":focused,"query":query})
}

fn close(scene: Scene, visual: &mut VisualTestContext) {
    visual.update(|window, _| window.remove_window());
    drop(scene.root);
    drop(scene.workspace);
    visual.cx.update(|_| {});
    visual.run_until_parked();
}

#[test]
fn native_sidebar_scene_inputs_are_reproducible() {
    for case in CASES {
        assert_eq!(
            serde_json::to_vec(&scene_state(case)).unwrap(),
            serde_json::to_vec(&scene_state(case)).unwrap()
        );
    }
}

#[gpui::test]
fn native_sidebar_geometry_and_input_contract(cx: &mut TestAppContext) {
    initialize(cx);
    let mut rows = Vec::new();
    for case in [
        CASES[0],
        Case {
            threads: 24,
            ..CASES[3]
        },
        Case {
            threads: 24,
            ..CASES[4]
        },
    ] {
        let (scene, visual) = mount(cx, case);
        let expected = snapshot(&scene, visual, case, "");
        let snapshot_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expected).unwrap())
        );
        let mut steps = Vec::new();
        for action in ACTIONS {
            interact(&scene, visual, action);
            assert_eq!(snapshot(&scene, visual, case, action.query()), expected);
            steps.push(
                json!({"action":action.name(),"geometry":geometry(&scene,visual,action.query())}),
            );
        }
        rows.push(json!({"label":case.label,"threads":case.threads,"input_bytes":scene.input_bytes,"input_sha256":scene.input_sha256,"snapshot_sha256":snapshot_sha256,"steps":steps}));
        close(scene, visual);
    }
    if let Some(path) = std::env::var_os("BRUHS_PERF_SIDEBAR_GEOMETRY_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
}

#[gpui::test]
#[ignore = "manual optimized native search input/redraw through complete headless settling"]
fn native_sidebar_action_cost(cx: &mut TestAppContext) {
    initialize(cx);
    let variant = std::env::var("BRUHS_PERF_SIDEBAR_VARIANT").unwrap();
    assert!(variant == "before" || variant == "after");
    let selected = std::env::var("BRUHS_PERF_SIDEBAR_CASE").ok();
    let mut measured = 0;
    for case in CASES {
        if selected
            .as_deref()
            .is_some_and(|selected| selected != case.label)
        {
            continue;
        }
        let (scene, visual) = mount(cx, case);
        let expected = snapshot(&scene, visual, case, "");
        let snapshot_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&expected).unwrap())
        );
        let mut samples: [Vec<f64>; 5] = Default::default();
        let mut final_geometry = Vec::new();
        for iteration in 0..35 {
            for (index, action) in ACTIONS.into_iter().enumerate() {
                let elapsed = interact(&scene, visual, action);
                assert_eq!(snapshot(&scene, visual, case, action.query()), expected);
                if iteration >= 5 {
                    samples[index].push(elapsed);
                }
                if iteration == 34 {
                    final_geometry.push(geometry(&scene, visual, action.query()));
                }
            }
        }
        for (index, action) in ACTIONS.into_iter().enumerate() {
            println!(
                "SIDEBAR_ACTION_COST {}",
                json!({"label":case.label,"threads":case.threads,"width":case.width,"height":case.height,"action":action.name(),"variant":variant,"input_bytes":scene.input_bytes,"input_sha256":scene.input_sha256,"snapshot_sha256":snapshot_sha256,"warmups":5,"samples_ms":samples[index],"final_geometry":final_geometry[index]})
            );
        }
        close(scene, visual);
        measured += 1;
    }
    assert!(measured > 0, "selected sidebar scene must exist");
}

fn row_selector(id: &str) -> &'static str {
    Box::leak(format!("desktop-thread-row-{}", element_key(id)).into_boxed_str())
}

#[gpui::test]
fn native_sidebar_viewport_scrolls_and_tabs_through_offscreen_cards(cx: &mut TestAppContext) {
    initialize(cx);
    let case = Case {
        threads: 32,
        height: 640.0,
        ..CASES[1]
    };
    let (scene, visual) = mount(cx, case);
    let expected = snapshot(&scene, visual, case, "");
    let ids = scene
        .workspace
        .read_with(visual, |workspace, cx| workspace.sidebar_thread_ids(cx));
    let first = row_selector(&ids[0]);
    let last = row_selector(ids.last().unwrap());
    let first_bounds = visual.debug_bounds(first).unwrap();
    let list_bounds = visual.debug_bounds("desktop-sidebar-threads").unwrap();
    assert_eq!(first_bounds.left(), list_bounds.left() + px(8.0));
    assert_eq!(first_bounds.right(), list_bounds.right() - px(8.0));
    assert!(
        visual.debug_bounds(last).is_none(),
        "offscreen cards must not be constructed"
    );
    let (scroll_max, viewport) = scene.workspace.read_with(visual, |workspace, _| {
        let viewport = workspace.sidebar_viewport.borrow();
        assert_eq!(viewport.list.item_count(), case.threads);
        let bounds = viewport.list.viewport_bounds();
        let scroll_max = viewport.list.max_offset_for_scrollbar();
        assert!(
            scroll_max.y > bounds.size.height,
            "the scrollbar includes offscreen rows from its first frame"
        );
        (scroll_max, bounds)
    });
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(
            px(0.0),
            -scroll_max.y - viewport.size.height,
        )),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    visual.run_until_parked();
    let bounds = visual
        .debug_bounds(last)
        .expect("the final card is reachable by scrolling");
    assert!(visual.debug_bounds(first).is_none());
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    scene.workspace.read_with(visual, |workspace, cx| {
        let mut selected = expected.clone();
        selected["active_thread_id"] = json!(ids.last());
        assert_eq!(
            serde_json::to_value(workspace.snapshot(cx)).unwrap(),
            selected
        );
    });

    // Shift-Tab crosses all three controls in each row and reaches every
    // preceding conversation, including cards that were never laid out.
    for id in ids.iter().rev().skip(1) {
        visual.simulate_keystrokes("shift-tab shift-tab shift-tab enter");
        scene.workspace.read_with(visual, |workspace, _| {
            assert_eq!(workspace.active_thread_id.as_ref(), Some(id))
        });
        assert!(visual.debug_bounds(row_selector(id)).is_some());
    }
    // Leaving and re-entering the list must use the surrounding control,
    // rather than wrap to another visible or offscreen row.
    visual.simulate_keystrokes("shift-tab tab enter");
    scene.workspace.read_with(visual, |workspace, _| {
        assert_eq!(workspace.active_thread_id.as_ref(), ids.first())
    });
    for id in ids.iter().skip(1) {
        visual.simulate_keystrokes("tab tab tab enter");
        scene.workspace.read_with(visual, |workspace, _| {
            assert_eq!(workspace.active_thread_id.as_ref(), Some(id))
        });
        assert!(visual.debug_bounds(row_selector(id)).is_some());
    }
    visual.simulate_keystrokes("tab tab tab shift-tab shift-tab shift-tab enter");
    scene.workspace.read_with(visual, |workspace, _| {
        assert_eq!(workspace.active_thread_id.as_ref(), ids.last())
    });
    assert_eq!(
        scene
            .workspace
            .read_with(visual, |workspace, _| workspace.threads.len()),
        case.threads
    );
    scene.workspace.update_in(visual, |workspace, window, cx| {
        workspace
            .sidebar_search
            .update(cx, |input, cx| input.focus(window, cx));
    });
    let final_index = ids.last().unwrap().strip_prefix("thread-").unwrap();
    visual.simulate_input(&format!("Saved thread {final_index}:"));
    assert!(visual.debug_bounds(last).is_some());
    visual.simulate_keystrokes("secondary-a backspace");
    assert!(
        visual.debug_bounds(first).is_some(),
        "clearing a single match returns to the first result"
    );
    close(scene, visual);
}

#[gpui::test]
fn native_sidebar_viewport_updates_card_heights_and_folded_search(cx: &mut TestAppContext) {
    initialize(cx);
    let case = Case {
        threads: 24,
        ..CASES[3]
    };
    let (scene, visual) = mount(cx, case);
    let first_id = scene.workspace.read_with(visual, |workspace, cx| {
        workspace.sidebar_thread_ids(cx)[0].clone()
    });
    let selector = row_selector(&first_id);
    let before = visual.debug_bounds(selector).unwrap().size.height;
    scene.workspace.update(visual, |workspace, cx| {
        let thread = workspace
            .threads
            .iter_mut()
            .find(|thread| thread.id == first_id)
            .unwrap();
        thread.chat.update(cx, |chat, cx| {
            chat.messages.clear();
            cx.notify();
        });
        cx.notify();
    });
    visual.run_until_parked();
    let shorter = visual.debug_bounds(selector).unwrap().size.height;
    assert!(
        shorter < before,
        "removing the metadata line remeasures the same card"
    );
    scene.workspace.update(visual, |workspace, cx| {
        workspace
            .threads
            .iter_mut()
            .find(|thread| thread.id == first_id)
            .unwrap()
            .branch = Some("feature/virtual-sidebar".into());
        workspace.sidebar_settled_open = false;
        workspace.sidebar_snoozed_open = false;
        cx.notify();
    });
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds(selector).unwrap().size.height, before);
    let saved = scene.workspace.read_with(visual, |workspace, cx| {
        assert_eq!(workspace.sidebar_viewport.borrow().list.item_count(), 14);
        serde_json::to_value(workspace.snapshot(cx)).unwrap()
    });
    visual.simulate_input("Saved");
    scene.workspace.read_with(visual, |workspace, cx| {
        assert_eq!(workspace.sidebar_thread_ids(cx).len(), case.threads);
        assert_eq!(
            workspace.sidebar_viewport.borrow().list.item_count(),
            case.threads
        );
        assert_eq!(serde_json::to_value(workspace.snapshot(cx)).unwrap(), saved);
    });
    visual.simulate_keystrokes("secondary-a backspace");
    scene.workspace.read_with(visual, |workspace, cx| {
        assert_eq!(workspace.sidebar_viewport.borrow().list.item_count(), 14);
        assert_eq!(serde_json::to_value(workspace.snapshot(cx)).unwrap(), saved);
    });
    close(scene, visual);
}
