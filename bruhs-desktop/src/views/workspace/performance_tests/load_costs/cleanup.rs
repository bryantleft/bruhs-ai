use super::super::super::desktop_state_backup_path;
use super::*;
use std::fs;

pub(super) fn close_fixture(
    root: Entity<bruhs_ui::Root>,
    workspace: Entity<Workspace>,
    visual: &mut VisualTestContext,
) {
    visual.update(|window, _| window.remove_window());
    drop(root);
    drop(workspace);
    // Executor draining alone leaves GPUI's dropped-entity queue for the next App update.
    // Flush it after the last handles drop, outside every startup timing interval.
    visual.cx.update(|_| {});
    visual.run_until_parked();
}

#[gpui::test]
fn workspace_fixture_cleanup_releases_entities_before_return(cx: &mut TestAppContext) {
    initialize(cx);
    let temporary = tempfile::tempdir().unwrap();
    for (label, threads) in [("settings-control", 1), ("thread-many-small", 512)] {
        for recovery in [false, true] {
            let path = temporary.path().join(format!("{label}-{recovery}.json"));
            let mut state = saved_state(label, 0, 0, 0);
            state.threads = (0..threads).map(|index| saved_thread(index, 0)).collect();
            let bytes = serde_json::to_vec(&state).unwrap();
            if recovery {
                fs::write(&path, b"{ invalid primary").unwrap();
                fs::write(desktop_state_backup_path(&path), &bytes).unwrap();
            } else {
                fs::write(&path, &bytes).unwrap();
            }
            for iteration in 0..2 {
                let root_released = Rc::new(std::cell::Cell::new(false));
                let workspace_released = Rc::new(std::cell::Cell::new(false));
                let root_flag = root_released.clone();
                let workspace_flag = workspace_released.clone();
                let captured = Rc::new(RefCell::new(None));
                let capture = captured.clone();
                let (root, visual) = cx.add_window_view(|window, cx| {
                    let loaded = load_desktop_state_with_backup(&path);
                    let workspace = cx.new(|cx| {
                        cx.on_release(move |_, _| workspace_flag.set(true)).detach();
                        Workspace::from_loaded_state(
                            loaded,
                            window,
                            cx,
                            current,
                            threads::current_threads,
                        )
                    });
                    cx.on_release(move |_, _| root_flag.set(true)).detach();
                    *capture.borrow_mut() = Some(workspace.clone());
                    bruhs_ui::Root::new(workspace, window, cx)
                });
                let workspace = captured.borrow_mut().take().unwrap();
                assert!(visual.debug_bounds("desktop-composer").is_some());
                workspace.read_with(visual, |workspace, cx| {
                    assert_eq!(workspace.persist_requests, 0);
                    assert!(workspace
                        .threads
                        .iter()
                        .all(|t| !t.chat.read(cx).is_streaming));
                    std::hint::black_box(serde_json::to_value(workspace.snapshot(cx)).unwrap());
                });
                close_fixture(root, workspace, visual);
                assert!(
                    root_released.get(),
                    "Root must be released before the next sample"
                );
                assert!(
                    workspace_released.get(),
                    "Workspace must be released before the next sample"
                );
                println!(
                    "WORKSPACE_CLEANUP_CONTRACT {}",
                    json!({
                        "label":label,"recovery":recovery,"iteration":iteration,
                        "root_released":root_released.get(),"workspace_released":workspace_released.get(),
                    })
                );
            }
        }
    }
}
