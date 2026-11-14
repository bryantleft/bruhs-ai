//! Thread-mention catalog freshness and isolated publishing cost.
use super::{PermissionMode, Workspace};
use crate::app_state::ThreadCatalog;
use gpui::{App, TestAppContext};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{hint::black_box, time::Instant};

fn original(workspace: &Workspace, cx: &mut App) {
    let threads = workspace
        .threads
        .iter()
        .map(|thread| (thread.id.clone(), thread.chat.read(cx).thread_title.clone()))
        .collect();
    cx.set_global(ThreadCatalog { threads });
}

fn populate(
    workspace: &mut Workspace,
    count: usize,
    long: bool,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<'_, Workspace>,
) {
    workspace.threads.clear();
    for index in 0..count {
        let title = format!(
            "Thread {index} café 東京 👩🏽‍💻{}",
            if long {
                " saved café 東京 👩🏽‍💻".repeat(32)
            } else {
                String::new()
            }
        );
        let mut thread = Workspace::create_thread_with_title(
            String::new(),
            "/synthetic/thread-catalog".into(),
            "gpt-5.4".into(),
            PermissionMode::Default,
            &title,
            window,
            cx,
        );
        thread.id = format!("thread-{index}");
        workspace.threads.push(thread);
    }
    workspace.active_thread_id = workspace.threads.first().map(|thread| thread.id.clone());
}

fn values(cx: &App) -> Vec<(String, String)> {
    cx.global::<ThreadCatalog>().threads.clone()
}

fn owned_capacity_bytes(cx: &App) -> usize {
    let catalog = cx.global::<ThreadCatalog>();
    std::mem::size_of::<ThreadCatalog>()
        + catalog.threads.capacity() * std::mem::size_of::<(String, String)>()
        + catalog
            .threads
            .iter()
            .map(|(id, title)| id.capacity() + title.capacity())
            .sum::<usize>()
}

#[gpui::test]
fn thread_catalog_tracks_titles_ids_order_and_membership(cx: &mut TestAppContext) {
    let (workspace, visual) = super::performance_tests::fixture(cx);
    workspace.update_in(visual, |w, window, cx| {
        populate(w, 3, false, window, cx);
        for change in 0..8 {
            match change {
                0 => {
                    cx.remove_global::<ThreadCatalog>();
                }
                1 => w.threads[0]
                    .chat
                    .update(cx, |chat, _| chat.thread_title = "Renamed 東京".into()),
                2 => w.threads.reverse(),
                3 => w.threads[0].id = "changed-id".into(),
                4 => {
                    w.threads.pop();
                }
                5 => {
                    let thread = Workspace::create_thread_with_title(
                        String::new(),
                        "/synthetic/thread-catalog".into(),
                        "gpt-5.4".into(),
                        PermissionMode::Default,
                        "Inserted",
                        window,
                        cx,
                    );
                    w.threads.insert(0, thread);
                }
                6 => cx.set_global(ThreadCatalog {
                    threads: vec![("other-workspace".into(), "Other title".into())],
                }),
                _ => w.threads.clear(),
            }
            w.sync_thread_catalog(cx);
            let actual = values(cx);
            original(w, cx);
            assert_eq!(actual, values(cx), "catalog after mutation {change}");
        }
    });
}

#[gpui::test]
fn thread_catalog_keeps_owned_buffers_when_rendered_values_are_unchanged(cx: &mut TestAppContext) {
    let (workspace, visual) = super::performance_tests::fixture(cx);
    workspace.update_in(visual, |w, window, cx| {
        populate(w, 3, true, window, cx);
        w.sync_thread_catalog(cx);
        let before = values(cx);
        let allocations = |cx: &App| {
            let threads = &cx.global::<ThreadCatalog>().threads;
            (
                threads.as_ptr(),
                threads
                    .iter()
                    .map(|(id, title)| (id.as_ptr(), title.as_ptr()))
                    .collect::<Vec<_>>(),
            )
        };
        let storage = allocations(cx);
        for _ in 0..5 {
            w.sync_thread_catalog(cx);
            assert_eq!(
                allocations(cx),
                storage,
                "each unchanged call must retain its vector and string buffers"
            );
        }
        assert_eq!(values(cx), before);
        assert_eq!(
            allocations(cx),
            storage,
            "unchanged catalog must retain its vector and string buffers"
        );
    });
}

#[gpui::test]
fn last_title_change_reuses_catalog_and_existing_string_capacity(cx: &mut TestAppContext) {
    let (workspace, visual) = super::performance_tests::fixture(cx);
    workspace.update_in(visual, |w, window, cx| {
        populate(w, 3, true, window, cx);
        w.sync_thread_catalog(cx);
        let buffers = |cx: &App| {
            let entries = &cx.global::<ThreadCatalog>().threads;
            (
                entries.as_ptr(),
                entries[0].1.as_ptr(),
                entries[2].1.as_ptr(),
            )
        };
        let before = buffers(cx);
        w.threads[2]
            .chat
            .update(cx, |chat, _| chat.thread_title = "Updated 東京".into());
        w.sync_thread_catalog(cx);
        assert_eq!(cx.global::<ThreadCatalog>().threads[2].1, "Updated 東京");
        assert_eq!(
            buffers(cx),
            before,
            "rename within existing capacity must reuse catalog storage"
        );
    });
}

#[gpui::test]
#[ignore = "manual optimized retained-original/current catalog publishing; set BRUHS_PERF_CATALOG_ORDER and run with --ignored --nocapture"]
fn thread_catalog_publishing_cost(cx: &mut TestAppContext) {
    let order = std::env::var("BRUHS_PERF_CATALOG_ORDER")
        .expect("explicit forward/reverse measurement order");
    assert!(matches!(order.as_str(), "forward" | "reverse"));
    let mut cases = Vec::new();
    for count in [1, 32, 2048] {
        for long in [false, true] {
            cases.push((count, long));
        }
    }
    if order == "reverse" {
        cases.reverse();
    }
    for (count, long) in cases {
        let (workspace, visual) = super::performance_tests::fixture(cx);
        workspace.update_in(visual, |w, window, cx| populate(w, count, long, window, cx));
        for change in ["unchanged", "last-title", "reorder", "missing-catalog"] {
            if change == "reorder" && count == 1 {
                continue;
            }
            let strategies = if order == "forward" {
                ["original", "current"]
            } else {
                ["current", "original"]
            };
            let mut samples = [Vec::new(), Vec::new()];
            let mut fingerprint = None;
            for sample in 0..60 {
                for strategy in strategies {
                    workspace.update(visual, |w, cx| {
                        original(w, cx);
                        match change {
                            "last-title" => w
                                .threads
                                .last()
                                .expect("nonempty fixture")
                                .chat
                                .update(cx, |chat, _| chat.thread_title.push('!')),
                            "reorder" => w.threads.reverse(),
                            "missing-catalog" => {
                                cx.remove_global::<ThreadCatalog>();
                            }
                            _ => {}
                        }
                        let input = black_box(&*w);
                        let began = Instant::now();
                        if strategy == "original" {
                            original(input, cx);
                        } else {
                            input.sync_thread_catalog(cx);
                        }
                        let elapsed = began.elapsed().as_secs_f64() * 1000.0;
                        if sample >= 10 {
                            samples[usize::from(strategy == "current")].push(elapsed);
                        }
                        let actual = values(cx);
                        if fingerprint.is_none() {
                            let bytes = serde_json::to_vec(&actual).expect("catalog JSON");
                            let owned_text_bytes: usize = actual
                                .iter()
                                .map(|(id, title)| id.len() + title.len())
                                .sum();
                            fingerprint =
                                Some((format!("{:x}", Sha256::digest(bytes)), owned_text_bytes));
                        }
                        original(w, cx);
                        assert_eq!(
                            actual,
                            values(cx),
                            "{count}/{long}/{change}/{strategy}/{sample}"
                        );
                        match change {
                            "last-title" => {
                                w.threads.last().expect("nonempty fixture").chat.update(
                                    cx,
                                    |chat, _| {
                                        chat.thread_title.pop();
                                    },
                                )
                            }
                            "reorder" => w.threads.reverse(),
                            _ => {}
                        }
                    });
                }
            }
            println!(
                "THREAD_CATALOG_RESULT {}",
                json!({"order":order,"threads":count,"long_titles":long,"change":change,"warmups":10,"samples":50,"catalog_fingerprint":fingerprint,"original_ms":samples[0],"current_ms":samples[1]})
            );
        }
        workspace.update(visual, |w, cx| {
            original(w, cx);
            let initial = owned_capacity_bytes(cx);
            for thread in &w.threads {
                thread.chat.update(cx, |chat, _| chat.thread_title = "Short".into());
            }
            w.sync_thread_catalog(cx);
            let retained_after_shortening = owned_capacity_bytes(cx);
            let actual = values(cx);
            original(w, cx);
            assert_eq!(actual, values(cx));
            let original_after_shortening = owned_capacity_bytes(cx);
            w.threads.clear();
            w.sync_thread_catalog(cx);
            let retained_after_deletion = owned_capacity_bytes(cx);
            original(w, cx);
            assert!(values(cx).is_empty());
            println!("THREAD_CATALOG_CAPACITY {}", json!({
                "order":order,"threads":count,"long_titles":long,"initial":initial,
                "current_after_shortening":retained_after_shortening,"original_after_shortening":original_after_shortening,
                "current_after_deletion":retained_after_deletion,"original_after_deletion":owned_capacity_bytes(cx),
            }));
        });
        visual.update(|window, _| window.remove_window());
        drop(workspace);
        visual.cx.update(|_| {});
        visual.run_until_parked();
    }
}
