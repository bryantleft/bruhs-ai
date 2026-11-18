//! Explicit local CLI fixtures; ordinary tests never launch an auth program.
use super::{
    ChatView, InitialAuthChecks, CLAUDE_AUTH_STATUS_TIMEOUT, CLAUDE_LOGIN_WAITING_MESSAGE,
};
use crate::{app_state::DesktopState, theme};
use gpui::{App, AppContext as _, Entity, Global, TestAppContext, VisualTestContext};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

struct ProbeFixture {
    program: String,
    completed: Sender<()>,
    original: bool,
}
impl Global for ProbeFixture {}

pub(super) fn install(cx: &mut App, program: String) -> Receiver<()> {
    let (completed, rx) = mpsc::channel();
    cx.set_global(ProbeFixture {
        program,
        completed,
        original: false,
    });
    rx
}

pub(super) fn clear(cx: &mut App) {
    if cx.try_global::<ProbeFixture>().is_some() {
        cx.remove_global::<ProbeFixture>();
    }
}

pub(super) fn program(cx: &App) -> Option<String> {
    cx.try_global::<ProbeFixture>()
        .map(|fixture| fixture.program.clone())
}

pub(super) fn completion(cx: &App) -> Option<Sender<()>> {
    cx.try_global::<ProbeFixture>()
        .map(|fixture| fixture.completed.clone())
}

pub(super) fn use_original_checks(cx: &App) -> bool {
    cx.global::<ProbeFixture>().original
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn fake_program(dir: &Path) -> String {
    fake_program_with(
        dir,
        r#"{"loggedIn":true,"email":"fixture@example.invalid"}"#,
        false,
    )
}

fn fake_program_with(dir: &Path, response: &str, held: bool) -> String {
    let path = dir.join("fake-claude");
    std::fs::write(dir.join("response"), response).expect("write the fixture auth response");
    if !held {
        release(dir);
    }
    std::fs::write(&path, format!(
        "#!/bin/sh\n[ \"$1 $2\" = 'auth status' ] || exit 99\nprintf '.\\n' >> {counter}\nwhile [ ! -f {release} ]; do /bin/sleep 0.01; done\n/bin/cat {response}\n",
        counter=shell_quote(&dir.join("calls")),
        release=shell_quote(&dir.join("release")),
        response=shell_quote(&dir.join("response")),
    )).expect("write the fake auth program");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the fake auth program executable");
    path.to_str()
        .expect("a UTF-8 fixture program path")
        .to_string()
}

fn release(dir: &Path) {
    std::fs::write(dir.join("release"), []).expect("release held auth probes");
}

fn calls(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("calls"))
        .expect("read the fake auth process counter")
        .lines()
        .count()
}

fn initialize(cx: &mut TestAppContext, program: String, original: bool) -> Receiver<()> {
    cx.executor().allow_parking();
    let (completed, rx) = mpsc::channel();
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("initialize isolated desktop state"));
        cx.set_global(ProbeFixture {
            program,
            completed,
            original,
        });
    });
    rx
}

fn chats(
    cx: &mut TestAppContext,
    count: usize,
) -> (
    Entity<bruhs_ui::Root>,
    Vec<Entity<ChatView>>,
    &mut VisualTestContext,
) {
    let captured = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let capture = captured.clone();
    let (root, visual) = cx.add_window_view(|window, cx| {
        let views: Vec<_> = (0..count)
            .map(|_| {
                cx.new(|cx| {
                    ChatView::new_with_project_path(
                        "/synthetic/auth-probe-fixture".to_string(),
                        window,
                        cx,
                    )
                })
            })
            .collect();
        let active = views.last().expect("at least one fixture chat").clone();
        *capture.borrow_mut() = views;
        bruhs_ui::Root::new(active, window, cx)
    });
    let views = captured.borrow_mut().drain(..).collect();
    (root, views, visual)
}

fn finish_probes(completed: &Receiver<()>, count: usize, visual: &mut VisualTestContext) {
    // A real probe already has a 15s timeout; allow that plus one equal scheduling
    // margin for this correctness watchdog. Fixture programs normally finish at once.
    for _ in 0..count {
        completed
            .recv_timeout(CLAUDE_AUTH_STATUS_TIMEOUT * 2)
            .expect("the fake auth process must finish within its watchdog");
    }
    visual.run_until_parked();
}

fn close(
    root: Entity<bruhs_ui::Root>,
    views: Vec<Entity<ChatView>>,
    visual: &mut VisualTestContext,
) {
    visual.update(|window, _| window.remove_window());
    drop(root);
    drop(views);
    visual.cx.update(|_| {});
    visual.run_until_parked();
}

#[gpui::test]
fn original_restored_chat_auth_checks_spawn_one_fake_process_per_chat(cx: &mut TestAppContext) {
    for count in [1, 8, 32] {
        let dir = tempfile::tempdir().unwrap();
        let completed = initialize(cx, fake_program(dir.path()), true);
        let (root, views, visual) = chats(cx, count);
        finish_probes(&completed, count, visual);
        assert_eq!(calls(dir.path()), count);
        for view in &views {
            view.read_with(visual, |view, _| {
                assert_eq!(view.auth_status.user(), Some("fixture@example.invalid"));
                assert!(view.auth_status.is_authenticated());
                assert!(
                    view.messages.is_empty()
                        && view.prompt_queue.is_empty()
                        && view.session_id.is_none()
                );
            });
        }
        close(root, views, visual);
    }
}

fn assert_authenticated(views: &[Entity<ChatView>], visual: &VisualTestContext) {
    for view in views {
        view.read_with(visual, |view, _| {
            assert_eq!(view.auth_status.user(), Some("fixture@example.invalid"));
            assert!(!view.initial_auth_pending);
            assert!(
                view.messages.is_empty()
                    && view.prompt_queue.is_empty()
                    && view.session_id.is_none()
            );
        });
    }
    visual
        .cx
        .read(|cx| assert!(cx.global::<InitialAuthChecks>().pending.is_none()));
}

#[gpui::test]
fn restored_chats_share_one_initial_auth_process(cx: &mut TestAppContext) {
    for count in [1, 8, 32, 128] {
        let dir = tempfile::tempdir().unwrap();
        let completed = initialize(cx, fake_program(dir.path()), false);
        let (root, views, visual) = chats(cx, count);
        finish_probes(&completed, 1, visual);
        assert_eq!(calls(dir.path()), 1);
        assert_authenticated(&views, visual);
        close(root, views, visual);
    }
}

#[gpui::test]
fn later_chats_check_again_instead_of_reusing_a_completed_auth_result(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let completed = initialize(cx, fake_program(dir.path()), false);
    for round in 1..=3 {
        let (root, views, visual) = chats(cx, 8);
        finish_probes(&completed, 1, visual);
        assert_eq!(calls(dir.path()), round);
        assert_authenticated(&views, visual);
        close(root, views, visual);
    }
}

#[gpui::test]
fn closing_the_first_chat_does_not_cancel_delivery_to_other_chats(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_program_with(
        dir.path(),
        r#"{"loggedIn":true,"email":"fixture@example.invalid"}"#,
        true,
    );
    let completed = initialize(cx, program, false);
    let (root, mut views, visual) = chats(cx, 8);
    let first = views.remove(0);
    let weak = first.downgrade();
    drop(first);
    visual.cx.update(|_| {});
    assert!(weak.upgrade().is_none());
    release(dir.path());
    finish_probes(&completed, 1, visual);
    assert_eq!(calls(dir.path()), 1);
    assert_authenticated(&views, visual);
    close(root, views, visual);
}

#[gpui::test]
fn closing_all_chats_clears_the_flight_for_the_next_chat(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let completed = initialize(
        cx,
        fake_program_with(dir.path(), r#"{"loggedIn":false}"#, true),
        false,
    );
    let (root, views, visual) = chats(cx, 8);
    let weak: Vec<_> = views.iter().map(Entity::downgrade).collect();
    close(root, views, visual);
    assert!(weak.iter().all(|view| view.upgrade().is_none()));
    release(dir.path());
    finish_probes(&completed, 1, visual);
    visual
        .cx
        .read(|cx| assert!(cx.global::<InitialAuthChecks>().pending.is_none()));
    let (root, views, visual) = chats(&mut visual.cx, 1);
    finish_probes(&completed, 1, visual);
    assert_eq!(calls(dir.path()), 2);
    close(root, views, visual);
}

#[gpui::test]
fn fresh_manual_checks_win_over_a_late_initial_response(cx: &mut TestAppContext) {
    for response in [
        r#"{"loggedIn":false}"#,
        r#"{"loggedIn":true,"email":"fresh@example.invalid"}"#,
    ] {
        let initial = tempfile::tempdir().unwrap();
        let fresh = tempfile::tempdir().unwrap();
        let completed = initialize(
            cx,
            fake_program_with(
                initial.path(),
                r#"{"loggedIn":true,"email":"fixture@example.invalid"}"#,
                true,
            ),
            false,
        );
        let (root, views, visual) = chats(cx, 2);
        visual.cx.update(|cx| {
            cx.global_mut::<ProbeFixture>().program =
                fake_program_with(fresh.path(), response, false)
        });
        views[0].update(visual, |view, cx| view.start_claude_auth_check(cx));
        finish_probes(&completed, 1, visual);
        let fresh_status = views[0].read_with(visual, |view, _| view.auth_status.clone());
        assert_eq!(
            fresh_status,
            super::parse_desktop_auth_status(response, "", None)
        );
        release(initial.path());
        finish_probes(&completed, 1, visual);
        assert_eq!(calls(initial.path()), 1);
        assert_eq!(calls(fresh.path()), 1);
        views[0].read_with(visual, |view, _| assert_eq!(view.auth_status, fresh_status));
        assert_authenticated(&views[1..], visual);
        close(root, views, visual);
    }
}

#[gpui::test]
fn preparing_login_keeps_waiting_state_when_the_initial_check_finishes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let completed = initialize(
        cx,
        fake_program_with(dir.path(), r#"{"loggedIn":true}"#, true),
        false,
    );
    let (root, views, visual) = chats(cx, 1);
    views[0].update(visual, |view, _| {
        assert!(view.prepare_claude_login());
        assert!(!view.prepare_claude_login());
        assert!(!view.initial_auth_pending);
    });
    release(dir.path());
    finish_probes(&completed, 1, visual);
    views[0].read_with(visual, |view, _| {
        assert!(view.auth_status.login_in_flight());
        assert_eq!(
            view.auth_status.message(),
            Some(CLAUDE_LOGIN_WAITING_MESSAGE)
        );
    });
    close(root, views, visual);
}

#[gpui::test]
fn failed_initial_checks_release_the_flight_so_later_chats_can_retry(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir
        .path()
        .join("missing-claude")
        .to_str()
        .unwrap()
        .to_string();
    let completed = initialize(cx, missing, false);
    let (root, views, visual) = chats(cx, 8);
    finish_probes(&completed, 1, visual);
    for view in &views {
        view.read_with(visual, |view, _| {
            assert!(!view.initial_auth_pending);
            let super::DesktopAuthStatus::SignedOut(status) = &view.auth_status else {
                panic!("A missing CLI must report an auth check error");
            };
            assert_eq!(
                status.error.as_deref(),
                Some("Claude Code CLI not found. Install it first.")
            );
        });
    }
    close(root, views, visual);
    cx.update(|cx| cx.global_mut::<ProbeFixture>().program = fake_program(dir.path()));
    let (root, views, visual) = chats(cx, 1);
    finish_probes(&completed, 1, visual);
    assert_eq!(calls(dir.path()), 1);
    assert_authenticated(&views, visual);
    close(root, views, visual);
}
