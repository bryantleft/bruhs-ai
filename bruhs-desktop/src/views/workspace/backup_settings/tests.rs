//! Settings, Backup on a stand-in server ([`FakeServer`]): the page's own
//! calls run through the same tasks, timers and answers as with the real
//! client, and the stand-in records each request as `METHOD path`.

use super::fixtures::{overview, ID, STAMP};
use super::{
    destination, downloads_dir, error_color, web_settings_url, BackupCalls, BackupSettings,
    KeptNote, Load, Making, PairCheck,
};
use crate::server_client::backup::{job_path, kept_path, staged_path, Wiped};
use crate::server_client::{ServerClient, ServerError};
use crate::{app_state::DesktopState, assets, theme};
use bruhs_models::backup::{
    created_line, option_label, ApplyAccepted, ApplyRestore, BackupApp, BackupFile, BackupJob,
    BackupOptions, BackupOverview, CreateBackup, JobState, JobStep, OpenBackup, OpenSource,
    OptionKind, RestoreSource, RestoreStatus, BACKUP_DESCRIPTION, BACKUP_STOPPED, BACKUP_TITLE,
    DESKTOP_NO_WEB, DESTINATION_EXISTS, KEYCHAIN_SET_UNREADABLE, NOTHING_TO_PUT_BACK,
    PASSWORDS_DIFFER, PASSWORD_SHORT, WEB_INCLUDED,
};
use gpui::{
    AppContext as _, Entity, FocusHandle, Focusable as _, Modifiers, TestAppContext,
    VisualTestContext,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

pub(super) const GOOD: &str = "correct horse 1";

/// A create body as the stand-in got it; the password only by length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SentCreate {
    pub(super) destination: Option<String>,
    pub(super) overwrite: bool,
    pub(super) options: BackupOptions,
    pub(super) password_chars: usize,
}

/// The server, as the page's tests see it. An answer that isn't queued
/// never comes, so a test sees the page waiting.
#[derive(Default)]
pub(super) struct FakeServer {
    pub(super) requests: RefCell<Vec<String>>,
    /// Every overview read answers this; none never answers.
    pub(super) overview: RefCell<Option<BackupOverview>>,
    /// Answers for create, open and each job read, in turn.
    pub(super) jobs: RefCell<VecDeque<JobState>>,
    pub(super) applies: RefCell<VecDeque<Result<ApplyAccepted, ServerError>>>,
    /// Answers for a cancel, discard or kept-set delete; none is `Ok`.
    pub(super) deletes: RefCell<VecDeque<Result<(), ServerError>>>,
    pub(super) status: RefCell<RestoreStatus>,
    pub(super) last_create: RefCell<Option<SentCreate>>,
    pub(super) last_open: RefCell<Option<String>>,
    pub(super) last_apply: RefCell<Option<ApplyRestore>>,
}

impl FakeServer {
    fn record(&self, request: String) {
        self.requests.borrow_mut().push(request);
    }

    fn next_job(&self) -> Option<Result<BackupJob, ServerError>> {
        self.jobs.borrow_mut().pop_front().map(|state| {
            Ok(BackupJob {
                id: ID.into(),
                state,
            })
        })
    }

    fn deleted(&self) -> Result<(), ServerError> {
        self.deletes.borrow_mut().pop_front().unwrap_or(Ok(()))
    }

    pub(super) fn overview(&self) -> Option<Result<BackupOverview, ServerError>> {
        self.record("GET /api/backup".into());
        self.overview.borrow().clone().map(Ok)
    }

    pub(super) fn create(
        &self,
        body: Wiped<CreateBackup>,
    ) -> Option<Result<BackupJob, ServerError>> {
        self.record("POST /api/backup".into());
        *self.last_create.borrow_mut() = Some(SentCreate {
            destination: body.destination.clone(),
            overwrite: body.overwrite,
            options: body.options,
            password_chars: body.password.chars().count(),
        });
        drop(body);
        self.next_job()
    }

    pub(super) fn job(&self, id: &str) -> Option<Result<BackupJob, ServerError>> {
        self.record(format!("GET {}", job_path(id).unwrap_or_default()));
        self.next_job()
    }

    pub(super) fn cancel_job(&self, id: &str) -> Result<(), ServerError> {
        self.record(format!("DELETE {}", job_path(id).unwrap_or_default()));
        self.deleted()
    }

    pub(super) fn open(&self, body: Wiped<OpenBackup>) -> Option<Result<BackupJob, ServerError>> {
        self.record("POST /api/backup/restore".into());
        if let OpenSource::Path { path } = &body.source {
            *self.last_open.borrow_mut() = Some(path.clone());
        }
        drop(body);
        self.next_job()
    }

    pub(super) fn discard(&self, id: &str) -> Result<(), ServerError> {
        self.record(format!("DELETE {}", staged_path(id).unwrap_or_default()));
        self.deleted()
    }

    pub(super) fn apply(&self, body: ApplyRestore) -> Option<Result<ApplyAccepted, ServerError>> {
        self.record("POST /api/backup/restore/apply".into());
        *self.last_apply.borrow_mut() = Some(body);
        self.applies.borrow_mut().pop_front()
    }

    pub(super) fn status(&self) -> Result<RestoreStatus, ServerError> {
        self.record("GET /api/backup/restore/status".into());
        Ok(self.status.borrow().clone())
    }

    pub(super) fn delete_kept(&self, stamp: &str) -> Result<(), ServerError> {
        self.record(format!("DELETE {}", kept_path(stamp).unwrap_or_default()));
        self.deleted()
    }
}

pub(super) fn fake(view: &BackupSettings) -> Rc<FakeServer> {
    match &view.calls {
        BackupCalls::Fake(fake) => fake.clone(),
        BackupCalls::Server(_) => panic!("a test page talks to a server"),
    }
}

/// The page alone in a window. When `loaded`, the server answers the
/// overview with the fixture; otherwise it never answers.
pub(super) fn page(
    cx: &mut TestAppContext,
    loaded: bool,
) -> (Entity<BackupSettings>, &mut VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let capture = Rc::new(RefCell::new(None));
    let output = capture.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
        let view = cx.new(|cx| BackupSettings::new(client, window, cx));
        if loaded {
            *fake(view.read(cx)).overview.borrow_mut() = Some(overview());
        }
        *output.borrow_mut() = Some(view.clone());
        bruhs_ui::Root::new(view, window, cx)
    });
    let view = capture.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2000.0)));
    cx.run_until_parked();
    (view, cx)
}

pub(super) fn server(view: &Entity<BackupSettings>, cx: &mut VisualTestContext) -> Rc<FakeServer> {
    view.read_with(cx, |view, _| fake(view))
}

pub(super) fn requests(view: &Entity<BackupSettings>, cx: &mut VisualTestContext) -> Vec<String> {
    server(view, cx).requests.borrow().clone()
}

fn leak(selector: &str) -> &'static str {
    Box::leak(selector.to_owned().into_boxed_str())
}

pub(super) fn rendered(cx: &mut VisualTestContext, selector: &str) -> bool {
    cx.debug_bounds(leak(selector)).is_some()
}

pub(super) fn bounds(cx: &mut VisualTestContext, selector: &str) -> gpui::Bounds<gpui::Pixels> {
    cx.debug_bounds(leak(selector))
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

pub(super) fn click(cx: &mut VisualTestContext, selector: &str) {
    let at = bounds(cx, selector).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

/// A key press as the app gets it: gpui clicks a focused button on key up.
pub(super) fn press(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).unwrap();
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

/// Let the 500 ms job poll (and anything else on a timer) run.
pub(super) fn wait(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

pub(super) fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
    cx.update(|window, cx| window.focused(cx))
}

pub(super) fn type_into(
    cx: &mut VisualTestContext,
    input: &Entity<bruhs_ui::input::InputState>,
    text: &str,
) {
    let handle = cx.update(|_, cx| input.focus_handle(cx));
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.simulate_input(text);
}

pub(super) fn value(
    cx: &mut VisualTestContext,
    input: &Entity<bruhs_ui::input::InputState>,
) -> String {
    cx.update(|_, cx| input.read(cx).value().to_string())
}

fn fields(
    view: &Entity<BackupSettings>,
    cx: &mut VisualTestContext,
) -> [Entity<bruhs_ui::input::InputState>; 3] {
    view.read_with(cx, |view, _| {
        [
            view.first.clone(),
            view.second.clone(),
            view.restore.password.clone(),
        ]
    })
}

/// Type a good password twice and press Make backup.
fn start_make(view: &Entity<BackupSettings>, cx: &mut VisualTestContext) {
    let [first, second, _] = fields(view, cx);
    type_into(cx, &first, GOOD);
    type_into(cx, &second, GOOD);
    click(cx, "desktop-backup-make");
    assert!(cx.did_prompt_for_new_path());
}

#[test]
fn the_section_is_after_apps() {
    use super::super::SettingsSection;
    let section = SettingsSection::Backup;
    assert_eq!(section.id(), "backup");
    assert_eq!(section.label(), BACKUP_TITLE);
    assert_eq!(section.description(), BACKUP_DESCRIPTION);
    assert_eq!(section.icon_path(), assets::ARCHIVE_ICON_PATH);
    assert_eq!(section.index(), SettingsSection::Apps.index() + 1);
    assert_eq!(SettingsSection::from_id("backup"), Some(section));
    assert_eq!(section.selector(), "desktop-settings-nav-backup");
}

#[test]
fn the_pair_check_waits_for_the_owner() {
    assert_eq!(PairCheck::of("", "", false), PairCheck::default());
    assert_eq!(PairCheck::of("short", "", false).first, None);
    assert_eq!(PairCheck::of("short", "", true).first, Some(PASSWORD_SHORT));
    assert_eq!(PairCheck::of(GOOD, "", true).second, None);
    let ready = PairCheck::of(GOOD, GOOD, true);
    assert_eq!((ready.first, ready.second, ready.ready), (None, None, true));
}

/// I1: the server writes `<name>.bruhsbak` when the name lacks it, so only
/// that final file may be replaced, and only when the dialog asked about
/// it.
#[test]
fn only_the_file_the_dialog_asked_about_is_replaced() {
    let folder = tempfile::tempdir().unwrap();
    let at = |name: &str| folder.path().join(name);
    let text = |name: &str| at(name).to_str().unwrap().to_owned();
    // Nothing there: the final name, nothing replaced.
    assert_eq!(destination(&at("new")), Ok((text("new.bruhsbak"), false)));
    assert_eq!(
        destination(&at("new.bruhsbak")),
        Ok((text("new.bruhsbak"), false))
    );
    // Both `x` and `x.bruhsbak`: the dialog asked about `x`, so the other
    // file is never replaced.
    std::fs::write(at("x"), b"notes").unwrap();
    std::fs::write(at("x.bruhsbak"), b"older backup").unwrap();
    assert_eq!(destination(&at("x")), Err(DESTINATION_EXISTS));
    // The dialog asked about `x.bruhsbak` itself: it may be replaced.
    assert_eq!(
        destination(&at("x.bruhsbak")),
        Ok((text("x.bruhsbak"), true))
    );
    // Only `y` there: a new `y.bruhsbak`, and `y` stays.
    std::fs::write(at("y"), b"notes").unwrap();
    assert_eq!(destination(&at("y")), Ok((text("y.bruhsbak"), false)));
}

#[gpui::test]
fn a_name_that_would_replace_another_file_sends_nothing(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("x"), b"notes").unwrap();
    std::fs::write(folder.path().join("x.bruhsbak"), b"older backup").unwrap();
    start_make(&view, cx);
    let chosen = folder.path().join("x");
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert!(!requests(&view, cx).iter().any(|r| r == "POST /api/backup"));
    assert!(rendered(cx, "desktop-backup-make-error"));
    view.read_with(cx, |view, _| {
        assert_eq!(view.make_error.as_deref(), Some(DESTINATION_EXISTS));
        assert!(!view.busy);
    });
    assert_eq!(
        std::fs::read(folder.path().join("x.bruhsbak")).unwrap(),
        b"older backup"
    );
}

#[test]
fn the_save_dialog_never_opens_at_the_root() {
    let folder = downloads_dir();
    assert_ne!(folder, std::path::Path::new("/"));
    assert!(folder.is_dir(), "{}", folder.display());
}

#[gpui::test]
fn make_needs_matching_passwords(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    assert_eq!(
        requests(&view, cx),
        ["GET /api/backup", "GET /api/backup/restore/status"]
    );
    // The three switches with their sizes and the rule's hint.
    for kind in [
        OptionKind::Keychain,
        OptionKind::Audio,
        OptionKind::Browsers,
    ] {
        assert!(rendered(cx, super::option_selector(kind)), "{kind:?}");
    }
    view.read_with(cx, |view, _| {
        let amount = |kind| super::option_amount(&view.overview, kind);
        assert_eq!(amount(OptionKind::Keychain), 7);
        assert_eq!(
            option_label(OptionKind::Audio, amount(OptionKind::Audio)),
            "Meeting audio (2.5 MB)"
        );
        assert_eq!(
            option_label(OptionKind::Browsers, amount(OptionKind::Browsers)),
            "Bruh browser sign-ins (700 KB)"
        );
    });
    let [first, second, _] = fields(&view, cx);
    // Nothing typed: Make backup does nothing.
    click(cx, "desktop-backup-make");
    assert!(!cx.did_prompt_for_new_path());
    // Different passwords: the mismatch shows under the second field.
    type_into(cx, &first, GOOD);
    type_into(cx, &second, "correct horse 2");
    assert!(rendered(cx, "desktop-backup-second-error"));
    assert_eq!(
        view.read_with(cx, |view, _| view.check.second),
        Some(PASSWORDS_DIFFER)
    );
    let field = bounds(cx, "desktop-backup-second-field");
    let error = bounds(cx, "desktop-backup-second-error");
    assert!(error.top() >= field.top() && error.bottom() <= field.bottom());
    click(cx, "desktop-backup-make");
    assert!(!cx.did_prompt_for_new_path(), "a mismatch can't make one");
    // Too short: the rule's message under the first field once it's left.
    cx.update(|window, cx| view.update(cx, |view, cx| view.clear_fields(window, cx)));
    type_into(cx, &first, "short");
    type_into(cx, &second, "s");
    assert!(rendered(cx, "desktop-backup-first-error"));
    assert!(!rendered(cx, "desktop-backup-second-error"));
    // Matching: Make backup asks where the file goes.
    cx.update(|window, cx| view.update(cx, |view, cx| view.clear_fields(window, cx)));
    type_into(cx, &first, GOOD);
    type_into(cx, &second, GOOD);
    assert!(!rendered(cx, "desktop-backup-second-error"));
    click(cx, "desktop-backup-make");
    assert!(cx.did_prompt_for_new_path());
}

/// I2: side by side from 640 px, stacked below it (web `sm:grid-cols-2`).
#[gpui::test]
fn the_password_fields_stack_in_a_narrow_window(cx: &mut TestAppContext) {
    let (_view, cx) = page(cx, true);
    let first = bounds(cx, "desktop-backup-first-field");
    let second = bounds(cx, "desktop-backup-second-field");
    assert_eq!(first.top(), second.top(), "side by side at 900");
    assert!(first.right() <= second.left());
    cx.simulate_resize(gpui::size(gpui::px(600.0), gpui::px(2000.0)));
    cx.run_until_parked();
    let first = bounds(cx, "desktop-backup-first-field");
    let second = bounds(cx, "desktop-backup-second-field");
    assert!(second.top() >= first.bottom(), "stacked at 600");
    assert_eq!(first.left(), second.left());
    assert!(second.right() <= gpui::px(600.0), "no sideways overflow");
}

#[gpui::test]
fn the_desktop_says_web_data_is_not_in(cx: &mut TestAppContext) {
    let (_view, cx) = page(cx, true);
    assert!(rendered(cx, "desktop-backup-no-web"));
    assert!(DESKTOP_NO_WEB != WEB_INCLUDED);
    // The note opens the group, above the switches (web M7's order).
    let note = bounds(cx, "desktop-backup-no-web");
    let keychain = bounds(cx, super::option_selector(OptionKind::Keychain));
    assert!(note.bottom() <= keychain.top());
    click(cx, "desktop-backup-open-web");
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("http://127.0.0.1:9/#/settings")
    );
    assert_eq!(
        web_settings_url("http://127.0.0.1:3001/"),
        "http://127.0.0.1:3001/#/settings"
    );
}

#[gpui::test]
fn masked_fields_clear_on_cancel(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    let [first, second, restore] = fields(&view, cx);
    for input in [&first, &second, &restore] {
        assert!(cx.update(|_, cx| input.read(cx).is_masked()));
    }
    let canary = format!("canary {}", uuid::Uuid::new_v4());
    type_into(cx, &first, &canary);
    type_into(cx, &second, &canary);
    click(cx, "desktop-backup-make");
    // Read and emptied on submit, before the dialog answers.
    assert!(value(cx, &first).is_empty() && value(cx, &second).is_empty());
    // Cancelling the save dialog sends nothing and the page is free.
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(
        requests(&view, cx),
        ["GET /api/backup", "GET /api/backup/restore/status"]
    );
    view.read_with(cx, |view, _| {
        assert!(!view.busy);
        assert_eq!(view.making, Making::Idle);
    });
    assert!(server(&view, cx).last_create.borrow().is_none());
    // Settings closing empties every field.
    type_into(cx, &first, &canary);
    type_into(cx, &restore, &canary);
    assert!(view.read_with(cx, |view, cx| view.holds_text(cx)));
    cx.update(|window, cx| view.update(cx, |view, cx| view.clear_fields(window, cx)));
    cx.run_until_parked();
    for input in [&first, &second, &restore] {
        assert!(value(cx, input).is_empty());
    }
    assert!(!view.read_with(cx, |view, cx| view.holds_text(cx)));
    assert!(!requests(&view, cx).concat().contains(&canary));
}

/// Tab walks the Make a backup group in page order (the web's DOM order,
/// ruling B Task 15) on to Choose backup file, and Enter on Make backup
/// opens the save dialog.
#[gpui::test]
fn the_keyboard_reaches_every_control_in_order(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    let [first, second, _] = fields(&view, cx);
    type_into(cx, &first, GOOD);
    type_into(cx, &second, GOOD);
    let (web, switches, make, choose, first_focus, second_focus) =
        view.read_with(cx, |view, cx| {
            (
                view.web_app_focus.clone(),
                view.switch_focus.clone(),
                view.make_focus.clone(),
                view.restore.choose_focus.clone(),
                view.first.focus_handle(cx),
                view.second.focus_handle(cx),
            )
        });
    let page_focus = view.read_with(cx, |view, _| view.page_focus.clone());
    cx.update(|window, cx| page_focus.focus(window, cx));
    let mut order = vec![web];
    order.extend(switches);
    order.extend([first_focus, second_focus, make, choose]);
    for (step, expected) in order.iter().enumerate() {
        press(cx, "tab");
        assert_eq!(focused(cx).as_ref(), Some(expected), "Tab stop {step}");
    }
    // Back on Make backup, Enter opens the save dialog.
    press(cx, "shift-tab");
    assert_eq!(focused(cx).as_ref(), order.get(6));
    press(cx, "enter");
    assert!(cx.did_prompt_for_new_path());
    let downloads = tempfile::tempdir().unwrap();
    let chosen = downloads.path().join("bruhs-backup");
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(
        requests(&view, cx).last().map(String::as_str),
        Some("POST /api/backup")
    );
    let sent = server(&view, cx).last_create.borrow().clone().unwrap();
    // The final name goes, with nothing to replace.
    assert_eq!(
        sent.destination.as_deref(),
        downloads.path().join("bruhs-backup.bruhsbak").to_str()
    );
    assert!(!sent.overwrite);
    assert_eq!(sent.options, BackupOptions::default());
    assert_eq!(sent.password_chars, GOOD.chars().count());
    assert!(view.read_with(cx, |view, _| view.busy));
}

/// A file the save dialog already asked about is replaced; the switches go
/// with the request.
#[gpui::test]
fn an_existing_file_is_replaced_only_after_the_dialog_asked(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    click(cx, super::option_selector(OptionKind::Browsers));
    start_make(&view, cx);
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("kept.bruhsbak");
    std::fs::write(&file, b"older backup").unwrap();
    let target = file.clone();
    cx.simulate_new_path_selection(move |_| Some(target));
    cx.run_until_parked();
    let sent = server(&view, cx).last_create.borrow().clone().unwrap();
    assert_eq!(sent.destination.as_deref(), file.to_str());
    assert!(sent.overwrite);
    assert!(sent.options.browser_profiles);
}

/// M1: make → create → poll every 500 ms → the saved file, through the
/// page's own tasks, with Cancel on the running job.
#[gpui::test]
fn a_made_backup_is_followed_to_the_saved_file(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    let fake = server(&view, cx);
    let file = BackupFile {
        name: "bruhs-backup-2026-11-20-0930.bruhsbak".into(),
        bytes: 2_000_000,
        path: Some("/Users/me/Downloads/bruhs-backup-2026-11-20-0930.bruhsbak".into()),
        download: None,
        warnings: vec!["1 link was left out.".into()],
    };
    fake.jobs.borrow_mut().extend([
        JobState::Running {
            done_bytes: 1_000,
            total_bytes: 9_000,
            step: JobStep::Writing,
        },
        JobState::Running {
            done_bytes: 5_000,
            total_bytes: 9_000,
            step: JobStep::Writing,
        },
        JobState::Created { file: file.clone() },
    ]);
    start_make(&view, cx);
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("b.bruhsbak");
    cx.simulate_new_path_selection(move |_| Some(target));
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-backup-make-progress"));
    assert!(rendered(cx, "desktop-backup-make-cancel"));
    // Cancel asks the server to stop the job; its poll answers the stop.
    click(cx, "desktop-backup-make-cancel");
    let job = format!("GET /api/backup/jobs/{ID}");
    assert_eq!(
        requests(&view, cx).last().cloned(),
        Some(format!("DELETE /api/backup/jobs/{ID}"))
    );
    // Nothing is read again before 500 ms.
    wait(cx, 400);
    assert!(!requests(&view, cx).contains(&job));
    wait(cx, 100);
    assert_eq!(requests(&view, cx).iter().filter(|r| **r == job).count(), 1);
    assert!(rendered(cx, "desktop-backup-make-progress"));
    wait(cx, 500);
    assert_eq!(requests(&view, cx).iter().filter(|r| **r == job).count(), 2);
    assert!(rendered(cx, "desktop-backup-made"));
    view.read_with(cx, |view, _| {
        assert!(!view.busy);
        assert_eq!(
            view.making,
            Making::Made {
                line: created_line(&file, BackupApp::Desktop),
                warnings: file.warnings.clone(),
            }
        );
    });
    assert_eq!(
        created_line(&file, BackupApp::Desktop),
        "Saved bruhs-backup-2026-11-20-0930.bruhsbak, 2.0 MB."
    );
    // Done: no more reads.
    wait(cx, 2_000);
    assert_eq!(requests(&view, cx).iter().filter(|r| **r == job).count(), 2);
}

#[gpui::test]
fn a_stopped_backup_reads_as_a_note(cx: &mut TestAppContext) {
    assert_eq!(error_color(BACKUP_STOPPED), theme::rambutan_400());
    assert_eq!(error_color(PASSWORDS_DIFFER), theme::lychee_300());
    let (view, cx) = page(cx, true);
    server(&view, cx)
        .jobs
        .borrow_mut()
        .push_back(JobState::Failed {
            message: BACKUP_STOPPED.into(),
        });
    start_make(&view, cx);
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("b.bruhsbak");
    cx.simulate_new_path_selection(move |_| Some(target));
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-backup-make-error"));
    assert_eq!(
        view.read_with(cx, |view, _| view.make_error.clone())
            .as_deref(),
        Some(BACKUP_STOPPED)
    );
}

#[gpui::test]
fn kept_sets_ask_before_acting(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "escape",
            super::super::CloseOverlays,
            None,
        )]);
    });
    let row = |name: &str| format!("desktop-backup-kept-{name}-{STAMP}");
    assert!(rendered(cx, &row("row")));
    assert!(rendered(cx, &row("put-back")));
    click(cx, &row("delete"));
    assert!(rendered(cx, &row("confirm")));
    assert!(!rendered(cx, &row("delete")));
    let focus = view.read_with(cx, |view, _| {
        let focus = &view.kept_focus[STAMP];
        (
            focus.put_back.clone(),
            focus.delete.clone(),
            focus.decline.clone(),
        )
    });
    assert_eq!(focused(cx), Some(focus.2.clone()), "Keep, the safe choice");
    // Keep sends nothing and gives focus back to Delete.
    click(cx, &row("decline"));
    assert!(!rendered(cx, &row("confirm")));
    assert_eq!(focused(cx), Some(focus.1.clone()));
    let before = requests(&view, cx).len();
    // Put back asks too; Escape keeps the set.
    click(cx, &row("put-back"));
    assert_eq!(focused(cx), Some(focus.2.clone()));
    press(cx, "escape");
    assert!(!rendered(cx, &row("confirm")));
    assert_eq!(focused(cx), Some(focus.0));
    assert_eq!(requests(&view, cx).len(), before, "nothing was sent");
    // Delete, confirmed: the server deletes it and the overview is read
    // again.
    *server(&view, cx).overview.borrow_mut() = Some(BackupOverview {
        pre_restores: Vec::new(),
        ..overview()
    });
    click(cx, &row("delete"));
    click(cx, &row("confirm-button"));
    let sent = requests(&view, cx);
    assert_eq!(
        sent[before..],
        [
            format!("DELETE /api/backup/pre-restore/{STAMP}"),
            "GET /api/backup".to_owned()
        ]
    );
    assert!(!rendered(cx, &row("row")));
    view.read_with(cx, |view, _| assert_eq!(view.asking, None));
}

/// M7: Escape in a kept set's confirm keeps the set and leaves Settings
/// open: the confirm takes the key before the workspace does.
#[gpui::test]
fn escape_in_a_confirm_leaves_settings_open(cx: &mut TestAppContext) {
    use super::super::{OpenSettings, SettingsSection, Workspace};
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().unwrap();
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "escape",
            super::super::CloseOverlays,
            None,
        )]);
    });
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(900.0)));
    cx.run_until_parked();
    let backup = workspace.read_with(cx, |workspace, _| workspace.backup_settings.clone());
    *server(&backup, cx).overview.borrow_mut() = Some(overview());
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(SettingsSection::Backup, cx);
    });
    cx.run_until_parked();
    backup.update_in(cx, |backup, window, cx| {
        backup.ask(STAMP.into(), super::Ask::Delete, window, cx);
    });
    cx.run_until_parked();
    let decline = backup.read_with(cx, |backup, _| backup.kept_focus[STAMP].decline.clone());
    assert_eq!(focused(cx), Some(decline));
    press(cx, "escape");
    backup.read_with(cx, |backup, _| assert_eq!(backup.asking, None));
    assert!(
        workspace.read_with(cx, |workspace, _| workspace.settings_visible()),
        "Settings stays open"
    );
    // A second Escape, with no confirm open, closes Settings as usual.
    press(cx, "escape");
    assert!(!workspace.read_with(cx, |workspace, _| workspace.settings_visible()));
}

fn refused(copy: &str) -> ServerError {
    ServerError::Status {
        request: "POST /api/backup/restore/apply".into(),
        code: reqwest::StatusCode::CONFLICT,
        body: serde_json::json!({ "error": copy }).to_string(),
    }
}

#[gpui::test]
fn a_set_with_nothing_or_no_keys_offers_only_delete(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, true);
    let fake = server(&view, cx);
    fake.applies
        .borrow_mut()
        .push_back(Err(refused(NOTHING_TO_PUT_BACK)));
    let row = |name: &str| format!("desktop-backup-kept-{name}-{STAMP}");
    click(cx, &row("put-back"));
    click(cx, &row("confirm-button"));
    assert_eq!(
        fake.last_apply
            .borrow()
            .as_ref()
            .map(|apply| (apply.source.clone(), apply.from_desktop)),
        Some((
            RestoreSource::PreRestore {
                stamp: STAMP.into()
            },
            true
        ))
    );
    assert!(!rendered(cx, &row("put-back")));
    assert!(rendered(cx, &row("delete")));
    assert!(rendered(cx, &row("error")));
    let delete = view.read_with(cx, |view, _| view.kept_focus[STAMP].delete.clone());
    assert_eq!(focused(cx), Some(delete), "focus goes to what is left");
    view.read_with(cx, |view, _| {
        assert!(view.notes[STAMP].nothing_to_put_back);
        assert!(!view.busy);
    });
    // A set whose keys are gone: Delete it again.
    view.update(cx, |view, cx| {
        view.notes.insert(
            STAMP.into(),
            KeptNote {
                error: Some(KEYCHAIN_SET_UNREADABLE.into()),
                nothing_to_put_back: false,
                unreadable: true,
            },
        );
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!rendered(cx, &row("put-back")));
    assert!(rendered(cx, &row("delete")));
    view.read_with(cx, |view, _| assert!(view.notes[STAMP].only_delete()));
}

#[gpui::test]
fn the_page_shows_where_it_stands(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, false);
    assert!(rendered(cx, "desktop-backup-status"));
    view.read_with(cx, |view, _| assert_eq!(view.load, Load::Loading));
    let not_found = ServerError::Status {
        request: "GET /api/backup".into(),
        code: reqwest::StatusCode::NOT_FOUND,
        body: String::new(),
    };
    view.update(cx, |view, cx| {
        let serial = view.load_serial;
        view.loaded(serial, Err(not_found), cx);
    });
    view.read_with(cx, |view, _| assert_eq!(view.load, Load::ServingOld));
    let mut unsupported = overview();
    unsupported.supported = false;
    view.update(cx, |view, cx| {
        let serial = view.load_serial;
        view.loaded(serial, Ok(unsupported), cx);
    });
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-backup-status"));
    assert!(!rendered(cx, "desktop-backup-make-group"));
    // A stale answer is dropped.
    view.update(cx, |view, cx| view.loaded(0, Ok(overview()), cx));
    view.read_with(cx, |view, _| assert!(!view.overview.supported));
    let mut full = overview();
    full.last = Some(bruhs_models::backup::LastBackup {
        made_at_ms: bruhs_models::EpochMs::saturating_from_i64(1_795_167_000_000),
        app: BackupApp::Web,
        bytes: 3_000_000,
    });
    full.outside_roots = vec!["/srv/reactive.sqlite3".into()];
    view.update(cx, |view, cx| {
        let serial = view.load_serial;
        view.loaded(serial, Ok(full), cx);
    });
    cx.run_until_parked();
    let last = bounds(cx, "desktop-backup-last");
    let outside = bounds(cx, "desktop-backup-outside");
    let make = bounds(cx, "desktop-backup-make-group");
    let restore = bounds(cx, "desktop-backup-restore-group");
    let kept = bounds(cx, "desktop-backup-kept-group");
    assert!(last.top() < outside.top() && outside.bottom() <= make.top());
    assert!(make.bottom() <= restore.top() && restore.bottom() <= kept.top());
}

#[gpui::test]
fn a_running_job_found_on_load_is_followed(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, false);
    let mut busy = overview();
    busy.job = Some(BackupJob {
        id: ID.into(),
        state: JobState::Running {
            done_bytes: 0,
            total_bytes: 1,
            step: JobStep::Checking,
        },
    });
    *server(&view, cx).overview.borrow_mut() = Some(busy);
    view.update(cx, |view, cx| view.reload(cx));
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-backup-restore-progress"));
    view.read_with(cx, |view, _| {
        assert!(view.busy);
        assert_eq!(view.making, Making::Idle);
    });
    wait(cx, 500);
    assert!(requests(&view, cx).contains(&format!("GET /api/backup/jobs/{ID}")));
}
