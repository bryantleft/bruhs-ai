//! Settings, Backup (D24, D25): the last backup under the title, then Make
//! a backup, Restore and Kept from earlier restores. It matches the web's
//! `components/backup_settings.rs`: the same groups in the same order, the
//! same states and copy, and Put back and Delete ask first. Every rule and
//! line is `bruhs_models::backup`'s.
//!
//! The desktop holds no browser data, so Make a backup says so
//! (`DESKTOP_NO_WEB`) and offers **Open the web app**. A backup goes to the
//! file the save dialog names, and a restore opens the file the open
//! dialog names; neither travels over HTTP.
//!
//! Passwords never sit on the page. The masked fields are never filled,
//! are read and emptied in one step on submit, and are emptied on cancel
//! and when Settings closes. A password taken from a field lives in
//! `Zeroizing` until its request is built, and the request body wipes it
//! when it drops, sent or not. Nothing logs one.

mod restore;
#[cfg(test)]
mod tests;

use super::vault_settings::{button, field, labelled, panel, small};
use super::*;
use crate::app_state::spawn_tokio;
use crate::server_client::backup::Wiped;
use crate::server_client::ServerError;
use bruhs_models::backup::{
    backup_file_name, created_line, delete_pre_restore_confirm, job_line, last_backup_line,
    option_label, outside_roots_line, password_pair_error, pre_restore_line, put_back_confirm,
    validate_password, with_backup_extension, ApplyAccepted, ApplyRestore, BackupApp, BackupFile,
    BackupJob, BackupOptions, BackupOverview, CreateBackup, JobState, JobStep, OpenBackup,
    OptionKind, PreRestoreInfo, RestoreSource, RestoreStatus, BACKUP_LOADING, BACKUP_STOPPED,
    CANCEL, DELETE, DELETE_AGAIN, DESKTOP_NO_WEB, DESTINATION_EXISTS, INVALID_REQUEST, KEEP,
    KEPT_GROUP, KEYCHAIN_OPTION_HINT, KEYCHAIN_SET_UNREADABLE, MAKE_BACKUP, MAKE_GROUP,
    NOTHING_TO_PUT_BACK, OPEN_WEB_APP, PASSWORD, PASSWORD_AGAIN, PASSWORD_HINT, PUT_BACK,
    UNSUPPORTED_PLATFORM,
};
use bruhs_models::crew::ServiceNotice;
use bruhs_ui::input::InputContentType;
use bruhs_ui::{press::Interactivity, switch::switch};
use restore::{Restore, Restoring};
use std::collections::HashMap;
use std::time::Duration;
use zeroize::Zeroizing;

/// How often a running job is read (D6).
const JOB_POLL: Duration = Duration::from_millis(500);
/// The three switches, in page order.
const OPTIONS: [OptionKind; 3] = [
    OptionKind::Keychain,
    OptionKind::Audio,
    OptionKind::Browsers,
];
/// Web `sm`: below this window width the password fields stack.
const NARROW_PX: f32 = 640.0;

/// Where the page's calls go: the server, or a test's stand-in. Each call
/// runs off the UI thread and answers on it.
#[derive(Clone)]
enum BackupCalls {
    Server(ServerClient),
    #[cfg(test)]
    Fake(Rc<tests::FakeServer>),
}

/// A test stand-in's answer, or one that never comes.
#[cfg(test)]
fn answer<T: Send + 'static>(
    cx: &App,
    reply: Option<Result<T, ServerError>>,
) -> Task<Result<T, ServerError>> {
    match reply {
        Some(reply) => Task::ready(reply),
        None => cx.background_spawn(std::future::pending()),
    }
}

impl BackupCalls {
    fn overview(&self, cx: &App) -> Task<Result<BackupOverview, ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.backup_overview().await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, fake.overview()),
        }
    }

    fn create(&self, body: Wiped<CreateBackup>, cx: &App) -> Task<Result<BackupJob, ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.create_backup(&body).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, fake.create(body)),
        }
    }

    fn job(&self, id: String, cx: &App) -> Task<Result<BackupJob, ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.backup_job(&id).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, fake.job(&id)),
        }
    }

    fn cancel_job(&self, id: String, cx: &App) -> Task<Result<(), ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.cancel_backup_job(&id).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, Some(fake.cancel_job(&id))),
        }
    }

    fn open(&self, body: Wiped<OpenBackup>, cx: &App) -> Task<Result<BackupJob, ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.open_backup(&body).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, fake.open(body)),
        }
    }

    fn discard(&self, id: String, cx: &App) -> Task<Result<(), ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.discard_staged(&id).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, Some(fake.discard(&id))),
        }
    }

    fn apply(&self, body: ApplyRestore, cx: &App) -> Task<Result<ApplyAccepted, ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.apply_restore(&body).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, fake.apply(body)),
        }
    }

    fn status(&self, cx: &App) -> Task<Result<RestoreStatus, ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.restore_status().await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, Some(fake.status())),
        }
    }

    fn delete_kept(&self, stamp: String, cx: &App) -> Task<Result<(), ServerError>> {
        match self {
            Self::Server(client) => {
                let client = client.clone();
                spawn_tokio(cx, async move { client.delete_kept(&stamp).await })
            }
            #[cfg(test)]
            Self::Fake(fake) => answer(cx, Some(fake.delete_kept(&stamp))),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Load {
    /// Not asked yet: the first draw asks.
    Idle,
    Loading,
    Ready,
    Failed(String),
    /// An older server: no `/api/backup`.
    ServingOld,
}

/// What the two Make a backup fields say, without either password: the
/// rule's message under each and whether **Make backup** may run (the
/// web's `PairCheck`).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
struct PairCheck {
    first: Option<&'static str>,
    second: Option<&'static str>,
    ready: bool,
}

impl PairCheck {
    /// The first field's message shows once it was left or the second one
    /// has text; the mismatch shows under the second field.
    fn of(first: &str, second: &str, first_left: bool) -> Self {
        let rule = validate_password(first).err();
        let pair = password_pair_error(first, second);
        Self {
            first: rule.filter(|_| !first.is_empty() && (first_left || !second.is_empty())),
            second: pair.filter(|_| rule.is_none() && !second.is_empty()),
            ready: pair.is_none(),
        }
    }
}

/// Make a backup's progress.
#[derive(Clone, PartialEq, Debug)]
enum Making {
    Idle,
    /// The save dialog is open.
    Choosing,
    /// The job's line once it runs, and its id for **Cancel**.
    Working {
        line: Option<String>,
        job: Option<String>,
    },
    Made {
        line: String,
        warnings: Vec<String>,
    },
}

/// Which confirm a kept set shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ask {
    Delete,
    PutBack,
}

/// What a kept set learned from its last answer.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
struct KeptNote {
    error: Option<String>,
    /// `NOTHING_TO_PUT_BACK`: only Delete is offered.
    nothing_to_put_back: bool,
    /// `KEYCHAIN_SET_UNREADABLE`: a delete was cut short; Delete it again.
    unreadable: bool,
}

impl KeptNote {
    fn only_delete(&self) -> bool {
        self.nothing_to_put_back || self.unreadable
    }
}

/// Where a running job's line and outcome show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Make,
    Restore,
}

/// The group a job belongs to, from its step: opening a backup checks and
/// prepares it, making one collects, reads the keychain and writes. Used
/// for a job another app started, found on load.
fn slot_of(state: &JobState) -> Slot {
    match state {
        JobState::Running {
            step: JobStep::Checking | JobStep::Preparing,
            ..
        }
        | JobState::Opened { .. } => Slot::Restore,
        _ => Slot::Make,
    }
}

/// A failed job's colour: a stopped backup is the owner's own doing, so
/// it reads as a note, not an error (B Task 14 M11).
fn error_color(message: &str) -> Hsla {
    if message == BACKUP_STOPPED {
        theme::rambutan_400()
    } else {
        theme::lychee_300()
    }
}

/// The web's `DANGER_BUTTON`: an outlined button with lychee text and a
/// lychee ring, no fill (B Task 15 ruling).
fn danger(button: Stateful<Div>) -> Stateful<Div> {
    button
        .text_color(theme::lychee_200())
        .border_color(theme::lychee_700())
}

/// Where **Open the web app** goes: the web app's Settings, which holds
/// Backup (D25, amended). The web routes by hash.
fn web_settings_url(base: &str) -> String {
    format!("{}/#/settings", base.trim_end_matches('/'))
}

/// Where the save dialog opens: Downloads when it is there, else home,
/// else the temporary folder.
fn downloads_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .filter(|downloads| downloads.is_dir())
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
}

/// Where a backup chosen as `chosen` is written: the server appends
/// `.bruhsbak` to a name without it (D6). `None` for a name that isn't
/// text.
fn final_path(chosen: &Path) -> Option<PathBuf> {
    let name = chosen.file_name()?.to_str()?;
    Some(chosen.with_file_name(with_backup_extension(name)))
}

/// The destination and `overwrite` to send for the file the save dialog
/// chose, or why not. Only the file the dialog asked about may be
/// replaced: when the server would write to another name that exists, it
/// is refused here, before anything is sent.
fn destination(chosen: &Path) -> Result<(String, bool), &'static str> {
    let target = final_path(chosen).ok_or(INVALID_REQUEST)?;
    let exists = target.symlink_metadata().is_ok();
    if exists && target != chosen {
        return Err(DESTINATION_EXISTS);
    }
    let text = target.to_str().ok_or(INVALID_REQUEST)?.to_owned();
    Ok((text, exists))
}

fn option_amount(overview: &BackupOverview, kind: OptionKind) -> u64 {
    match kind {
        OptionKind::Keychain => u64::from(overview.keychain_items),
        OptionKind::Audio => overview.sizes.audio_bytes,
        OptionKind::Browsers => overview.sizes.browser_bytes,
    }
}

fn option_on(options: &mut BackupOptions, kind: OptionKind) -> &mut bool {
    match kind {
        OptionKind::Keychain => &mut options.keychain,
        OptionKind::Audio => &mut options.audio,
        OptionKind::Browsers => &mut options.browser_profiles,
    }
}

fn option_selector(kind: OptionKind) -> &'static str {
    match kind {
        OptionKind::Keychain => "desktop-backup-option-keychain",
        OptionKind::Audio => "desktop-backup-option-audio",
        OptionKind::Browsers => "desktop-backup-option-browsers",
    }
}

/// A field's text, read into a buffer that is wiped when dropped.
fn read(input: &Entity<InputState>, cx: &App) -> Zeroizing<String> {
    Zeroizing::new(input.read(cx).value().to_string())
}

/// A field's text, emptied in the same step.
fn take(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> Zeroizing<String> {
    let value = read(input, cx);
    input.update(cx, |input, cx| input.set_value("", window, cx));
    value
}

/// One kept set's buttons that focus moves between.
pub(super) struct KeptFocus {
    pub(super) put_back: FocusHandle,
    pub(super) delete: FocusHandle,
    pub(super) decline: FocusHandle,
}

pub(super) struct BackupSettings {
    calls: BackupCalls,
    /// The server's address, for **Open the web app**.
    base_url: String,
    overview: BackupOverview,
    load: Load,
    /// Which overview read an answer is for; a smoke fixture moves it on.
    load_serial: u64,
    /// A job, an apply or a delete runs: every action waits.
    busy: bool,
    options: BackupOptions,
    first: Entity<InputState>,
    second: Entity<InputState>,
    first_left: bool,
    check: PairCheck,
    making: Making,
    make_error: Option<String>,
    restore: Restore,
    asking: Option<(String, Ask)>,
    notes: BTreeMap<String, KeptNote>,
    /// Whether this launch read the last restore's outcome yet.
    status_read: bool,
    /// When this section was made: a restore applied since just before
    /// this launch shows its outcome once.
    made_at: EpochMs,
    web_app_focus: FocusHandle,
    switch_focus: [FocusHandle; 3],
    make_focus: FocusHandle,
    pub(super) kept_focus: HashMap<String, KeptFocus>,
    /// Focus anywhere on the page, so an answer knows it may move it.
    page_focus: FocusHandle,
    /// Starts this app again once a restore is scheduled (D17).
    relaunch: Rc<dyn Fn(&mut App)>,
    task: Option<Task<()>>,
    job_task: Option<Task<()>>,
    prompt_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl BackupSettings {
    pub(super) fn new(
        client: ServerClient,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let first = cx.new(|cx| InputState::new(window, cx).masked(true));
        let second = cx.new(|cx| InputState::new(window, cx).masked(true));
        let restore = Restore::new(window, cx);
        let mut subscriptions = Vec::new();
        for (input, is_first) in [(&first, true), (&second, false)] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.recheck(cx),
                    InputEvent::Blur if is_first => {
                        this.first_left = true;
                        this.recheck(cx);
                    }
                    InputEvent::PressEnter {
                        secondary: false, ..
                    } => this.make(window, cx),
                    _ => {}
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &restore.password,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        ..
                    }
                ) {
                    this.open(window, cx);
                }
            },
        ));
        let base_url = client.base_url().to_owned();
        let calls = BackupCalls::Server(client);
        // Tests never reach a server: every page starts on a stand-in that
        // never answers until a test gives it answers.
        #[cfg(test)]
        let calls = {
            drop(calls);
            BackupCalls::Fake(Rc::default())
        };
        #[cfg(not(test))]
        let relaunch: Rc<dyn Fn(&mut App)> = Rc::new(crate::restore_relaunch::relaunch_and_quit);
        // A test that reaches the relaunch sets its own; nothing else may
        // start a process.
        #[cfg(test)]
        let relaunch: Rc<dyn Fn(&mut App)> = Rc::new(|_| panic!("a test relaunched the app"));
        Self {
            calls,
            base_url,
            overview: BackupOverview::default(),
            load: Load::Idle,
            load_serial: 0,
            busy: false,
            options: BackupOptions::default(),
            first,
            second,
            first_left: false,
            check: PairCheck::default(),
            making: Making::Idle,
            make_error: None,
            restore,
            asking: None,
            notes: BTreeMap::new(),
            status_read: false,
            made_at: EpochMs::now(),
            web_app_focus: cx.focus_handle().tab_stop(true),
            switch_focus: [
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
            ],
            make_focus: cx.focus_handle().tab_stop(true),
            kept_focus: HashMap::new(),
            page_focus: cx.focus_handle(),
            relaunch,
            task: None,
            job_task: None,
            prompt_task: None,
            _subscriptions: subscriptions,
        }
    }

    /// Whether any password field holds text, so Settings closing must
    /// empty it.
    pub(super) fn holds_text(&self, cx: &App) -> bool {
        [&self.first, &self.second, &self.restore.password]
            .into_iter()
            .any(|input| !input.read(cx).is_empty())
    }

    /// Empty every password field (Settings closed).
    pub(super) fn clear_fields(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        for input in [&self.first, &self.second, &self.restore.password] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.first_left = false;
        self.check = PairCheck::default();
        cx.notify();
    }

    fn recheck(&mut self, cx: &mut Context<'_, Self>) {
        let first = read(&self.first, cx);
        let second = read(&self.second, cx);
        self.check = PairCheck::of(&first, &second, self.first_left);
        cx.notify();
    }

    /// A press that starts a wait disables its own control, whose focus
    /// would then point at nothing and leave every key dead. Focus on the
    /// page moves to the page itself, and a text field keeps it.
    fn hold_focus_on_page(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.page_focus.contains_focused(window, cx) && !window.has_focused_input(cx) {
            self.page_focus.focus(window, cx);
        }
    }

    /// Settings opened: the page starts over, as the web's remounts, and
    /// reads the overview. A running job, apply or delete keeps it as it is.
    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.making = Making::Idle;
        self.make_error = None;
        self.asking = None;
        self.notes.clear();
        self.restore.reset();
        self.clear_fields(window, cx);
        self.reload(cx);
    }

    /// Read `GET /api/backup` again. A page already showing keeps showing
    /// while it reads. Where backup doesn't run it asks nothing.
    fn reload(&mut self, cx: &mut Context<'_, Self>) {
        if !cfg!(unix) {
            self.unsupported(cx);
            return;
        }
        if self.load != Load::Ready {
            self.load = Load::Loading;
        }
        self.load_serial = self.load_serial.wrapping_add(1);
        let serial = self.load_serial;
        cx.notify();
        let operation = self.calls.overview(cx);
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The page may be gone by now.
            let _ = this.update(cx, |this, cx| this.loaded(serial, result, cx));
        }));
    }

    /// Backup runs on macOS and Linux (D27): elsewhere the page shows
    /// [`UNSUPPORTED_PLATFORM`] and nothing to make or restore.
    fn unsupported(&mut self, cx: &mut Context<'_, Self>) {
        self.overview = BackupOverview {
            supported: false,
            ..BackupOverview::default()
        };
        self.load = Load::Ready;
        cx.notify();
    }

    fn loaded(
        &mut self,
        serial: u64,
        result: Result<BackupOverview, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        if serial != self.load_serial {
            return;
        }
        let overview = match result {
            Ok(overview) => overview,
            Err(error) => {
                self.load = if error.is_not_found() {
                    Load::ServingOld
                } else {
                    Load::Failed(error.backup_message())
                };
                cx.notify();
                return;
            }
        };
        self.restore.loaded(&overview);
        let job = overview.job.clone();
        self.overview = overview;
        self.load = Load::Ready;
        let stamps: HashSet<&String> = self
            .overview
            .pre_restores
            .iter()
            .map(|set| &set.stamp)
            .collect();
        self.kept_focus.retain(|stamp, _| stamps.contains(stamp));
        self.notes.retain(|stamp, _| stamps.contains(stamp));
        // A job another app started: follow it in its own group.
        if let Some(job) = job.filter(|job| job_line(&job.state).is_some()) {
            if !self.busy {
                self.busy = true;
                let slot = slot_of(&job.state);
                self.job_answered(slot, job.id, Ok(job.state), cx);
            }
        }
        if !self.status_read {
            self.status_read = true;
            self.read_status(cx);
        }
        cx.notify();
    }

    // ── Jobs ────────────────────────────────────────────────────────────

    /// Follow `job` in `slot`: its line while it runs, then its outcome.
    fn follow(&mut self, slot: Slot, job: BackupJob, cx: &mut Context<'_, Self>) {
        self.job_answered(slot, job.id, Ok(job.state), cx);
    }

    /// A job's state came in: show its line and read it again in 500 ms, or
    /// show how it finished.
    fn job_answered(
        &mut self,
        slot: Slot,
        id: String,
        result: Result<JobState, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        let state = match result {
            Ok(state) => state,
            Err(error) => {
                self.fail(slot, error.backup_message(), cx);
                return;
            }
        };
        let Some(line) = job_line(&state) else {
            self.finish(slot, state, cx);
            return;
        };
        let working = Some(id.clone());
        match slot {
            Slot::Make => {
                self.making = Making::Working {
                    line: Some(line),
                    job: working,
                };
            }
            Slot::Restore => {
                self.restore.step = Restoring::Working {
                    line: Some(line),
                    job: working,
                };
            }
        }
        cx.notify();
        self.poll(slot, id, cx);
    }

    /// Read the job again in 500 ms. The poll stops when the page goes.
    fn poll(&mut self, slot: Slot, id: String, cx: &mut Context<'_, Self>) {
        self.job_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOB_POLL).await;
            let target = id.clone();
            let Ok(operation) = this.update(cx, |this, cx| this.calls.job(target, cx)) else {
                return;
            };
            let result = operation.await.map(|job| job.state);
            let _ = this.update(cx, |this, cx| this.job_answered(slot, id, result, cx));
        }));
    }

    /// A job finished in `state`.
    fn finish(&mut self, slot: Slot, state: JobState, cx: &mut Context<'_, Self>) {
        self.busy = false;
        match (slot, state) {
            (_, JobState::Failed { message }) => self.fail(slot, message, cx),
            (_, JobState::Created { file }) => self.made(&file),
            (_, JobState::Opened { preview }) => self.restore.opened(preview),
            (Slot::Make, _) => self.making = Making::Idle,
            (Slot::Restore, _) => self.restore.step = Restoring::Idle,
        }
        cx.notify();
    }

    fn made(&mut self, file: &BackupFile) {
        self.making = Making::Made {
            line: created_line(file, BackupApp::Desktop),
            warnings: file.warnings.clone(),
        };
    }

    /// A job's or a call's failure, under the group it belongs to (M2).
    fn fail(&mut self, slot: Slot, message: String, cx: &mut Context<'_, Self>) {
        self.busy = false;
        match slot {
            Slot::Make => {
                self.making = Making::Idle;
                self.make_error = Some(message);
            }
            Slot::Restore => self.restore.fail(message),
        }
        cx.notify();
    }

    /// Stop a running job; its poll then answers the stop.
    fn cancel_job(&mut self, slot: Slot, id: String, cx: &mut Context<'_, Self>) {
        let operation = self.calls.cancel_job(id, cx);
        cx.spawn(async move |this, cx| {
            if let Err(error) = operation.await {
                let _ = this.update(cx, |this, cx| {
                    let message = error.backup_message();
                    match slot {
                        Slot::Make => this.make_error = Some(message),
                        Slot::Restore => this.restore.error = Some(message),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    // ── Make a backup ───────────────────────────────────────────────────

    /// Make backup: reads and empties both fields, then asks where the file
    /// goes.
    fn make(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy || !self.check.ready || self.load != Load::Ready {
            return;
        }
        let password = take(&self.first, window, cx);
        let again = take(&self.second, window, cx);
        self.check = PairCheck::default();
        self.first_left = false;
        if let Some(error) = password_pair_error(&password, &again) {
            self.make_error = Some(error.to_owned());
            cx.notify();
            return;
        }
        drop(again);
        self.busy = true;
        self.make_error = None;
        self.making = Making::Choosing;
        self.hold_focus_on_page(window, cx);
        cx.notify();
        let name = backup_file_name(EpochMs::now());
        let chosen = cx.prompt_for_new_path(&downloads_dir(), Some(&name));
        self.prompt_task = Some(cx.spawn_in(window, async move |this, cx| {
            // A dialog that couldn't open counts as cancelled.
            let path = chosen.await.ok().and_then(Result::ok).flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                this.chose_destination(path, password, window, cx);
            });
        }));
    }

    /// The save dialog answered. A cancel drops the password; a file starts
    /// the job at the path the server writes to (`.bruhsbak` appended when
    /// missing), replacing a file only when it is the one the dialog asked
    /// about (I1).
    fn chose_destination(
        &mut self,
        path: Option<PathBuf>,
        mut password: Zeroizing<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(path) = path else {
            drop(password);
            self.busy = false;
            self.making = Making::Idle;
            cx.notify();
            return;
        };
        let (destination, overwrite) = match destination(&path) {
            Ok(found) => found,
            Err(refusal) => {
                drop(password);
                self.fail(Slot::Make, refusal.to_owned(), cx);
                return;
            }
        };
        self.making = Making::Working {
            line: None,
            job: None,
        };
        self.hold_focus_on_page(window, cx);
        let body = Wiped(CreateBackup {
            password: std::mem::take(&mut *password),
            options: self.options,
            app: BackupApp::Desktop,
            web_part: None,
            web_db_version: None,
            destination: Some(destination),
            overwrite,
        });
        drop(password);
        cx.notify();
        let operation = self.calls.create(body, cx);
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(job) => this.follow(Slot::Make, job, cx),
                Err(error) => this.fail(Slot::Make, error.backup_message(), cx),
            });
        }));
    }

    fn flip(&mut self, kind: OptionKind, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let on = option_on(&mut self.options, kind);
        *on = !*on;
        cx.notify();
    }

    fn open_web_app(&mut self, cx: &mut Context<'_, Self>) {
        cx.open_url(&web_settings_url(&self.base_url));
    }

    // ── Kept from earlier restores ──────────────────────────────────────

    fn kept_focus(&mut self, stamp: &str, cx: &mut Context<'_, Self>) -> &KeptFocus {
        self.kept_focus
            .entry(stamp.to_owned())
            .or_insert_with(|| KeptFocus {
                put_back: cx.focus_handle().tab_stop(true),
                delete: cx.focus_handle().tab_stop(true),
                decline: cx.focus_handle().tab_stop(true),
            })
    }

    fn note(&mut self, stamp: &str) -> &mut KeptNote {
        self.notes.entry(stamp.to_owned()).or_default()
    }

    /// The confirm takes focus on its safe choice (I3).
    fn ask(&mut self, stamp: String, ask: Ask, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.note(&stamp).error = None;
        let decline = self.kept_focus(&stamp, cx).decline.clone();
        self.asking = Some((stamp, ask));
        decline.focus(window, cx);
        cx.notify();
    }

    /// Keep or Escape: the confirm goes and focus returns to the button
    /// that opened it.
    fn keep(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((stamp, ask)) = self.asking.take() else {
            return;
        };
        let focus = self.kept_focus(&stamp, cx);
        let target = match ask {
            Ask::PutBack => focus.put_back.clone(),
            Ask::Delete => focus.delete.clone(),
        };
        target.focus(window, cx);
        cx.notify();
    }

    /// The confirm's own button: delete the set, or put it back.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        match self.asking.clone() {
            Some((stamp, Ask::Delete)) => self.delete_kept(stamp, window, cx),
            Some((stamp, Ask::PutBack)) => {
                self.apply(RestoreSource::PreRestore { stamp }, window, cx);
            }
            None => {}
        }
    }

    fn delete_kept(&mut self, stamp: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.busy = true;
        self.note(&stamp).error = None;
        self.hold_focus_on_page(window, cx);
        cx.notify();
        let operation = self.calls.delete_kept(stamp.clone(), cx);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.deleted(stamp, result, window, cx);
            });
        }));
    }

    /// The server answered a Delete. A refusal keeps the confirm open with
    /// its reason, so trying again is one press.
    fn deleted(
        &mut self,
        stamp: String,
        result: Result<(), ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = false;
        match result {
            Ok(()) => {
                self.asking = None;
                self.notes.remove(&stamp);
                self.kept_focus.remove(&stamp);
                self.reload(cx);
            }
            Err(error) => {
                self.note(&stamp).error = Some(error.backup_message());
                if self.page_focus.is_focused(window) {
                    let decline = self.kept_focus(&stamp, cx).decline.clone();
                    decline.focus(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// A refused Put back: the confirm goes, the row notes why, and focus
    /// returns to Put back, or to Delete when the refusal took Put back
    /// away (N4).
    fn put_back_refused(
        &mut self,
        stamp: &str,
        error: &ServerError,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let detail = error.backup_detail();
        let refused = |copy: &str| detail.as_deref() == Some(copy);
        let (nothing, unreadable) = (
            refused(NOTHING_TO_PUT_BACK),
            refused(KEYCHAIN_SET_UNREADABLE),
        );
        self.asking = None;
        let note = self.note(stamp);
        note.nothing_to_put_back |= nothing;
        note.unreadable |= unreadable;
        note.error = Some(error.backup_message());
        let only_delete = note.only_delete();
        let focus = self.kept_focus(stamp, cx);
        let target = if only_delete {
            focus.delete.clone()
        } else {
            focus.put_back.clone()
        };
        target.focus(window, cx);
    }

    // ── Smoke fixtures ──────────────────────────────────────────────────

    /// `settings-backup-job` and `settings-backup-preview`: the section with
    /// fixture data, a backup being written or an opened backup's preview
    /// (same-size captures). Any answer still on its way is dropped.
    #[cfg(debug_assertions)]
    pub(super) fn smoke_fixture(&mut self, preview: bool, cx: &mut Context<'_, Self>) {
        let fixture = fixtures::overview();
        self.load_serial = self.load_serial.wrapping_add(1);
        self.status_read = true;
        self.overview = fixture;
        self.load = Load::Ready;
        if preview {
            self.restore.opened(Box::new(fixtures::preview()));
        } else {
            self.busy = true;
            self.making = Making::Working {
                line: job_line(&JobState::Running {
                    done_bytes: 3_200_000,
                    total_bytes: 9_000_000,
                    step: JobStep::Writing,
                }),
                job: Some(fixtures::ID.into()),
            };
        }
        cx.notify();
    }
}

/// The data the smoke scenes and the tests show.
#[cfg(debug_assertions)]
mod fixtures {
    use bruhs_models::backup::{
        BackupApp, BackupCounts, BackupOverview, BackupSizes, PathRewrite, PreRestoreInfo,
        RestorePreview,
    };
    use bruhs_models::EpochMs;

    pub(super) const ID: &str = "0123456789abcdef0123456789abcdef";
    pub(super) const STAMP: &str = "2026-11-20-0930-0a1f";
    /// 2026-11-20 09:30 UTC, the stamp's own time.
    const MADE_AT_MS: i64 = 1_795_167_000_000;

    pub(super) fn kept_set() -> PreRestoreInfo {
        PreRestoreInfo {
            stamp: STAMP.into(),
            made_at_ms: EpochMs::saturating_from_i64(MADE_AT_MS),
            bytes: 1_200_000,
            has_web_part: false,
        }
    }

    pub(super) fn overview() -> BackupOverview {
        BackupOverview {
            supported: true,
            sizes: BackupSizes {
                data_bytes: 9_000_000,
                audio_bytes: 2_500_000,
                browser_bytes: 700_000,
            },
            keychain_items: 7,
            pre_restores: vec![kept_set()],
            ..BackupOverview::default()
        }
    }

    pub(super) fn preview() -> RestorePreview {
        RestorePreview {
            id: ID.into(),
            made_at_ms: EpochMs::saturating_from_i64(MADE_AT_MS),
            made_on: BackupApp::Web,
            app_version: "0.1.1".into(),
            source_home: "/Users/old".into(),
            rewrites: vec![PathRewrite {
                from: "/Users/old".into(),
                to: "/Users/new".into(),
            }],
            outside_home: vec!["/Volumes/Work".into()],
            counts: BackupCounts {
                bruhs: 2,
                vault_entries: 1,
                keychain_items: 3,
                files: 120,
            },
            bytes: 4_500_000,
            has_web_part: true,
            has_keychain: true,
            has_audio: false,
            has_browser_profiles: false,
            skipped_links: 1,
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────

/// Web `text-label text-rambutan-100`, for a group's title.
fn group_title(text: &'static str) -> Div {
    div()
        .min_w_0()
        .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
        .text_color(theme::rambutan_100())
        .child(text)
}

/// A status or alert line with its own id, for `role`.
fn line(selector: String, text: impl Into<SharedString>, color: Hsla, role: Role) -> Stateful<Div> {
    let id = selector.clone();
    small(text, color)
        .id(SharedString::from(id))
        .debug_selector(move || selector)
        .role(role)
}

/// One group: web `px-4 py-3`, ruled from the one above.
fn group(id: &'static str, first: bool) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .role(Role::Group)
        .min_w_0()
        .py_3()
        .when(!first, |el| {
            el.border_t_1()
                .border_color(theme::longan_800().opacity(0.7))
        })
        .flex()
        .flex_col()
}

impl BackupSettings {
    fn render_option(&self, index: usize, kind: OptionKind, cx: &Context<'_, Self>) -> AnyElement {
        let label = option_label(kind, option_amount(&self.overview, kind));
        let mut options = self.options;
        let checked = *option_on(&mut options, kind);
        let mut toggle = switch(
            option_selector(kind),
            label.clone(),
            checked,
            Interactivity::disabled_if(self.busy),
        );
        if !self.busy {
            toggle = toggle.track_focus(&self.switch_focus[index]);
        }
        let toggle = on_press_view(toggle, cx, move |this, _, cx| this.flip(kind, cx));
        div()
            .min_h(px(theme::CONTROL_PX))
            .min_w_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_200())
                            .child(label),
                    )
                    .when(kind == OptionKind::Keychain, |el| {
                        el.child(small(KEYCHAIN_OPTION_HINT, theme::rambutan_500()))
                    }),
            )
            .child(toggle)
            .into_any_element()
    }

    /// Password and Password again: side by side from 640 px, stacked
    /// below it (web `grid gap-3 sm:grid-cols-2`).
    fn render_passwords(&self, narrow: bool) -> AnyElement {
        let first_error = self.check.first.map(|text| {
            line(
                "desktop-backup-first-error".into(),
                text,
                theme::lychee_300(),
                Role::Alert,
            )
            .mt_1()
        });
        let second_error = self.check.second.map(|text| {
            line(
                "desktop-backup-second-error".into(),
                text,
                theme::lychee_300(),
                Role::Alert,
            )
            .mt_1()
        });
        let column = || {
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .when(!narrow, |el| el.flex_1())
        };
        div()
            .mt_3()
            .min_w_0()
            .flex()
            .when(narrow, |el| el.flex_col())
            .gap_3()
            .child(
                column()
                    .debug_selector(|| "desktop-backup-first-field".into())
                    .child(labelled(
                        PASSWORD,
                        field(
                            &self.first,
                            PASSWORD,
                            false,
                            Some(InputContentType::NewPassword),
                        ),
                    ))
                    .child(small(PASSWORD_HINT, theme::rambutan_500()).mt_1())
                    .children(first_error),
            )
            .child(
                column()
                    .debug_selector(|| "desktop-backup-second-field".into())
                    .child(labelled(
                        PASSWORD_AGAIN,
                        field(
                            &self.second,
                            PASSWORD_AGAIN,
                            false,
                            Some(InputContentType::NewPassword),
                        ),
                    ))
                    .children(second_error),
            )
            .into_any_element()
    }

    fn render_make(&self, narrow: bool, cx: &Context<'_, Self>) -> AnyElement {
        let disabled = self.busy || !self.check.ready;
        let mut make = button(
            "desktop-backup-make",
            "desktop-backup-make".into(),
            MAKE_BACKUP,
            WebButtonVariant::Outline,
            disabled,
        );
        if !disabled {
            make = make.track_focus(&self.make_focus);
        }
        let make = on_press_view(make, cx, |this, window, cx| this.make(window, cx));
        let cancel = match &self.making {
            Making::Working { job: Some(id), .. } => {
                let id = id.clone();
                Some(on_press_view(
                    button(
                        "desktop-backup-make-cancel",
                        "desktop-backup-make-cancel".into(),
                        CANCEL,
                        WebButtonVariant::Outline,
                        false,
                    ),
                    cx,
                    move |this, _, cx| this.cancel_job(Slot::Make, id.clone(), cx),
                ))
            }
            _ => None,
        };
        let web_app = on_press_view(
            button(
                "desktop-backup-open-web",
                "desktop-backup-open-web".into(),
                OPEN_WEB_APP,
                WebButtonVariant::Outline,
                false,
            )
            .role(Role::Link)
            .text_color(theme::blueberry_300())
            .track_focus(&self.web_app_focus),
            cx,
            |this, _, cx| this.open_web_app(cx),
        );
        let status = match &self.making {
            Making::Idle | Making::Choosing | Making::Working { line: None, .. } => None,
            Making::Working {
                line: Some(text), ..
            } => Some(
                small(text.clone(), theme::rambutan_300())
                    .mt_2()
                    .debug_selector(|| "desktop-backup-make-progress".into())
                    .into_any_element(),
            ),
            Making::Made { line, warnings } => Some(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        small(line.clone(), theme::pandan_300())
                            .mt_2()
                            .debug_selector(|| "desktop-backup-made".into()),
                    )
                    .children(
                        warnings
                            .iter()
                            .map(|warning| small(warning.clone(), theme::durian_300()).mt_1()),
                    )
                    .into_any_element(),
            ),
        };
        group("desktop-backup-make-group", true)
            .aria_label(MAKE_GROUP)
            .child(group_title(MAKE_GROUP))
            // The group's own note, above the switches, as on the web (M7):
            // this app holds no browser data, and the web app makes one
            // that does (D25).
            .child(
                div()
                    .mt_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_2()
                    .child(
                        small(DESKTOP_NO_WEB, theme::rambutan_400())
                            .debug_selector(|| "desktop-backup-no-web".into()),
                    )
                    .child(web_app),
            )
            .child(
                div().mt_3().min_w_0().flex().flex_col().gap_3().children(
                    OPTIONS
                        .iter()
                        .enumerate()
                        .map(|(index, kind)| self.render_option(index, *kind, cx)),
                ),
            )
            .child(self.render_passwords(narrow))
            .child(
                div()
                    .mt_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(make)
                    .children(cancel),
            )
            .child(
                div()
                    .id("desktop-backup-make-status")
                    .role(Role::Status)
                    .children(status),
            )
            .when_some(self.make_error.clone(), |el, message| {
                let color = error_color(&message);
                el.child(
                    line(
                        "desktop-backup-make-error".into(),
                        message,
                        color,
                        Role::Alert,
                    )
                    .mt_1(),
                )
            })
            .into_any_element()
    }

    fn render_kept_row(
        &self,
        index: usize,
        info: &PreRestoreInfo,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let stamp = info.stamp.clone();
        let note = self.notes.get(&stamp).cloned().unwrap_or_default();
        let asking = self
            .asking
            .as_ref()
            .filter(|(asked, _)| *asked == stamp)
            .map(|(_, ask)| *ask);
        let focus = self.kept_focus.get(&stamp);
        let selector = |name: &str| format!("desktop-backup-kept-{name}-{stamp}");
        let actions = asking.is_none().then(|| {
            let put_back = (!note.only_delete()).then(|| {
                let mut put_back = button(
                    SharedString::from(selector("put-back")),
                    selector("put-back"),
                    PUT_BACK,
                    WebButtonVariant::Outline,
                    self.busy,
                );
                if let (Some(focus), false) = (focus, self.busy) {
                    put_back = put_back.track_focus(&focus.put_back);
                }
                let stamp = stamp.clone();
                on_press_view(put_back, cx, move |this, window, cx| {
                    this.ask(stamp.clone(), Ask::PutBack, window, cx);
                })
            });
            let mut delete = button(
                SharedString::from(selector("delete")),
                selector("delete"),
                if note.unreadable {
                    DELETE_AGAIN
                } else {
                    DELETE
                },
                WebButtonVariant::Outline,
                self.busy,
            );
            if let (Some(focus), false) = (focus, self.busy) {
                delete = delete.track_focus(&focus.delete);
            }
            let stamp = stamp.clone();
            let delete = on_press_view(delete, cx, move |this, window, cx| {
                this.ask(stamp.clone(), Ask::Delete, window, cx);
            });
            div()
                .mt_3()
                .flex()
                .items_center()
                .gap_2()
                .children(put_back)
                .child(delete)
        });
        let confirm = asking.map(|ask| {
            let (question, confirm, decline) = match ask {
                Ask::Delete => (delete_pre_restore_confirm(info), DELETE, KEEP),
                Ask::PutBack => (put_back_confirm(info), PUT_BACK, CANCEL),
            };
            let mut decline_button = button(
                SharedString::from(selector("decline")),
                selector("decline"),
                decline,
                WebButtonVariant::Outline,
                self.busy,
            );
            if let (Some(focus), false) = (focus, self.busy) {
                decline_button = decline_button.track_focus(&focus.decline);
            }
            panel()
                .id(SharedString::from(selector("confirm")))
                .debug_selector({
                    let selector = selector("confirm");
                    move || selector
                })
                .role(Role::Group)
                .aria_label(question.clone())
                // Escape keeps the set before it can close Settings.
                .on_action(cx.listener(|this, _: &CloseOverlays, window, cx| {
                    this.keep(window, cx);
                }))
                .child(
                    div()
                        .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                        .text_color(theme::rambutan_200())
                        .child(question),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(on_press_view(
                            danger(button(
                                SharedString::from(selector("confirm-button")),
                                selector("confirm-button"),
                                confirm,
                                WebButtonVariant::Outline,
                                self.busy,
                            )),
                            cx,
                            |this, window, cx| this.confirm(window, cx),
                        ))
                        .child(on_press_view(decline_button, cx, |this, window, cx| {
                            this.keep(window, cx);
                        })),
                )
        });
        div()
            .id(SharedString::from(selector("row")))
            .debug_selector({
                let selector = selector("row");
                move || selector
            })
            .role(Role::Group)
            .aria_label(pre_restore_line(info))
            .min_w_0()
            .py_3()
            .when(index > 0, |el| {
                el.border_t_1().border_color(theme::longan_800())
            })
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .child(pre_restore_line(info)),
            )
            .children(actions)
            .children(confirm)
            .when_some(note.error, |el, message| {
                el.child(line(selector("error"), message, theme::lychee_300(), Role::Alert).mt_1())
            })
            .into_any_element()
    }

    fn render_kept(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if self.overview.pre_restores.is_empty() {
            return None;
        }
        let rows: Vec<AnyElement> = self
            .overview
            .pre_restores
            .iter()
            .enumerate()
            .map(|(index, info)| self.render_kept_row(index, info, cx))
            .collect();
        Some(
            group("desktop-backup-kept-group", false)
                .aria_label(KEPT_GROUP)
                .child(group_title(KEPT_GROUP))
                .child(div().min_w_0().flex().flex_col().children(rows))
                .into_any_element(),
        )
    }

    fn render_ready(&self, narrow: bool, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let last = self.overview.last.as_ref().map(last_backup_line);
        let outside = outside_roots_line(&self.overview.outside_roots);
        let mut parts = Vec::new();
        if last.is_some() || outside.is_some() {
            parts.push(
                div()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .children(last.map(|text| {
                        small(text, theme::rambutan_400())
                            .debug_selector(|| "desktop-backup-last".into())
                    }))
                    .children(outside.map(|text| {
                        small(text, theme::durian_300())
                            .mt_1()
                            .debug_selector(|| "desktop-backup-outside".into())
                    }))
                    .into_any_element(),
            );
        }
        parts.push(self.render_make(narrow, cx));
        parts.push(self.render_restore(cx));
        parts.extend(self.render_kept(cx));
        parts
    }
}

impl Render for BackupSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.load == Load::Idle {
            self.reload(cx);
        }
        self.focus_new_preview(window, cx);
        // Every kept set's buttons keep their focus handles while they show.
        let stamps: Vec<String> = self
            .overview
            .pre_restores
            .iter()
            .map(|set| set.stamp.clone())
            .collect();
        for stamp in &stamps {
            self.kept_focus(stamp, cx);
        }
        // A breakpoint is a design number: compare in CSS pixels.
        let narrow = bruhs_ui::zoom::css_px(window.viewport_size().width) < NARROW_PX;
        let body: Vec<AnyElement> = match &self.load {
            Load::Idle | Load::Loading => vec![line(
                "desktop-backup-status".into(),
                BACKUP_LOADING,
                theme::rambutan_400(),
                Role::Status,
            )
            .into_any_element()],
            Load::Failed(message) => vec![line(
                "desktop-backup-status".into(),
                message.clone(),
                theme::lychee_300(),
                Role::Alert,
            )
            .into_any_element()],
            Load::ServingOld => vec![line(
                "desktop-backup-status".into(),
                ServiceNotice::ServingOld.message(),
                theme::durian_300(),
                Role::Status,
            )
            .into_any_element()],
            Load::Ready if !self.overview.supported => vec![line(
                "desktop-backup-status".into(),
                UNSUPPORTED_PLATFORM,
                theme::durian_300(),
                Role::Status,
            )
            .into_any_element()],
            Load::Ready => self.render_ready(narrow, cx),
        };
        div()
            .id("desktop-backup")
            .track_focus(&self.page_focus)
            .px_4()
            .py_1()
            .min_w_0()
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
            .text_color(theme::rambutan_200())
            .flex()
            .flex_col()
            .children(body)
    }
}
