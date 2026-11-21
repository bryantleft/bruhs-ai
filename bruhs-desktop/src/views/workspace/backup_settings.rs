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
//! `Zeroizing` until its request is built, and the request's own copy is
//! wiped once it was sent. Nothing logs one.

mod restore;

use super::vault_settings::{button, field, labelled, panel, small};
use super::*;
use crate::app_state::spawn_tokio;
use crate::server_client::ServerError;
use bruhs_models::backup::{
    backup_file_name, created_line, delete_pre_restore_confirm, job_line, last_backup_line,
    option_label, outside_roots_line, password_pair_error, pre_restore_line, put_back_confirm,
    validate_password, BackupApp, BackupFile, BackupJob, BackupOptions, BackupOverview,
    CreateBackup, JobState, JobStep, OptionKind, PreRestoreInfo, RestoreSource, BACKUP_LOADING,
    BACKUP_STOPPED, CANCEL, DELETE, DELETE_AGAIN, DESKTOP_NO_WEB, INVALID_REQUEST, KEEP,
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
/// Web `rounded-grape` and the confirm's `bg-longan-950/60`.
const OPTIONS: [OptionKind; 3] = [
    OptionKind::Keychain,
    OptionKind::Audio,
    OptionKind::Browsers,
];

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

/// Where **Open the web app** goes: the web app's Settings, which holds
/// Backup (D25). The web routes by hash.
fn web_settings_url(base: &str) -> String {
    format!("{}/#/settings", base.trim_end_matches('/'))
}

/// Where the save dialog opens: Downloads, else home.
fn downloads_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("/"))
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

/// A create body the tests can read; the password is never kept.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct SentCreate {
    destination: Option<String>,
    overwrite: bool,
    options: BackupOptions,
}

pub(super) struct BackupSettings {
    client: ServerClient,
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
    /// Every request, as `METHOD path`; tests never reach a server.
    #[cfg(test)]
    requests: Vec<String>,
    #[cfg(test)]
    last_create: Option<SentCreate>,
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
        #[cfg(not(test))]
        let relaunch: Rc<dyn Fn(&mut App)> = Rc::new(crate::restore_relaunch::relaunch_and_quit);
        // A test that reaches the relaunch sets its own; nothing else may
        // start a process.
        #[cfg(test)]
        let relaunch: Rc<dyn Fn(&mut App)> = Rc::new(|_| panic!("a test relaunched the app"));
        Self {
            client,
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
            #[cfg(test)]
            requests: Vec::new(),
            #[cfg(test)]
            last_create: None,
        }
    }

    fn record(&mut self, request: String) {
        #[cfg(test)]
        self.requests.push(request);
        #[cfg(not(test))]
        drop(request);
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
    /// while it reads.
    fn reload(&mut self, cx: &mut Context<'_, Self>) {
        if self.load != Load::Ready {
            self.load = Load::Loading;
        }
        self.load_serial = self.load_serial.wrapping_add(1);
        let serial = self.load_serial;
        self.record(format!("GET {}", crate::server_client::backup::BACKUP_PATH));
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.backup_overview().await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The page may be gone by now.
            let _ = this.update(cx, |this, cx| this.loaded(serial, result, cx));
        }));
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

    fn poll(&mut self, slot: Slot, id: String, cx: &mut Context<'_, Self>) {
        let Some(path) = crate::server_client::backup::job_path(&id) else {
            self.fail(slot, bruhs_models::backup::GONE.to_owned(), cx);
            return;
        };
        self.record(format!("GET {path}"));
        if cfg!(test) {
            return;
        }
        self.job_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOB_POLL).await;
            let target = id.clone();
            let Ok(operation) = this.update(cx, |this, cx| {
                let client = this.client.clone();
                spawn_tokio(cx, async move { client.backup_job(&target).await })
            }) else {
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
        let Some(path) = crate::server_client::backup::job_path(&id) else {
            return;
        };
        self.record(format!("DELETE {path}"));
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.cancel_backup_job(&id).await });
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
    /// the job, replacing a file that is there only because the dialog
    /// already asked about it.
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
        let Some(destination) = path.to_str().map(str::to_owned) else {
            self.fail(Slot::Make, INVALID_REQUEST.to_owned(), cx);
            return;
        };
        let overwrite = path.symlink_metadata().is_ok();
        self.making = Making::Working {
            line: None,
            job: None,
        };
        self.hold_focus_on_page(window, cx);
        self.record(format!(
            "POST {}",
            crate::server_client::backup::BACKUP_PATH
        ));
        #[cfg(test)]
        {
            self.last_create = Some(SentCreate {
                destination: Some(destination.clone()),
                overwrite,
                options: self.options,
            });
        }
        let mut body = CreateBackup {
            password: std::mem::take(&mut *password),
            options: self.options,
            app: BackupApp::Desktop,
            web_part: None,
            web_db_version: None,
            destination: Some(destination),
            overwrite,
        };
        drop(password);
        cx.notify();
        if cfg!(test) {
            zeroize::Zeroize::zeroize(&mut body.password);
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move {
            let result = client.create_backup(&body).await;
            zeroize::Zeroize::zeroize(&mut body.password);
            result
        });
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
        cx.open_url(&web_settings_url(self.client.base_url()));
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
        let Some(path) = crate::server_client::backup::kept_path(&stamp) else {
            return;
        };
        self.busy = true;
        self.note(&stamp).error = None;
        self.hold_focus_on_page(window, cx);
        self.record(format!("DELETE {path}"));
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let target = stamp.clone();
        let operation = spawn_tokio(cx, async move { client.delete_kept(&target).await });
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

    #[cfg(test)]
    fn requests_for_test(&self) -> Vec<String> {
        self.requests.clone()
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

    pub(super) fn kept_set() -> PreRestoreInfo {
        PreRestoreInfo {
            stamp: STAMP.into(),
            made_at_ms: EpochMs::saturating_from_i64(1_763_631_000_000),
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
            made_at_ms: EpochMs::saturating_from_i64(1_763_631_000_000),
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

    fn render_make(&self, cx: &Context<'_, Self>) -> AnyElement {
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
            .child(
                div()
                    .mt_3()
                    .min_w_0()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
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
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
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
                    ),
            )
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
                            button(
                                SharedString::from(selector("confirm-button")),
                                selector("confirm-button"),
                                confirm,
                                WebButtonVariant::Destructive,
                                self.busy,
                            ),
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

    fn render_ready(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
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
        parts.push(self.render_make(cx));
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
            Load::Ready => self.render_ready(cx),
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

#[cfg(test)]
mod tests {
    use super::fixtures::{overview, ID, STAMP};
    use super::{
        error_color, web_settings_url, BackupSettings, KeptNote, Load, Making, PairCheck, Slot,
    };
    use crate::server_client::{ServerClient, ServerError};
    use crate::{app_state::DesktopState, assets, theme};
    use bruhs_models::backup::{
        created_line, option_label, BackupApp, BackupFile, BackupOptions, JobState, JobStep,
        OptionKind, BACKUP_DESCRIPTION, BACKUP_STOPPED, BACKUP_TITLE, DESKTOP_NO_WEB,
        KEYCHAIN_SET_UNREADABLE, NOTHING_TO_PUT_BACK, PASSWORDS_DIFFER, PASSWORD_SHORT,
        WEB_INCLUDED,
    };
    use gpui::{
        AppContext as _, Entity, FocusHandle, Focusable as _, Modifiers, TestAppContext,
        VisualTestContext,
    };
    use std::{cell::RefCell, rc::Rc};

    pub(super) const GOOD: &str = "correct horse 1";

    /// The page alone in a window, its overview as `overview` (or none yet).
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
            *output.borrow_mut() = Some(view.clone());
            bruhs_ui::Root::new(view, window, cx)
        });
        let view = capture.borrow().clone().unwrap();
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2000.0)));
        cx.run_until_parked();
        if loaded {
            view.update(cx, |view, cx| {
                let serial = view.load_serial;
                view.loaded(serial, Ok(overview()), cx);
            });
            cx.run_until_parked();
        }
        (view, cx)
    }

    pub(super) fn requests(
        view: &Entity<BackupSettings>,
        cx: &mut VisualTestContext,
    ) -> Vec<String> {
        view.read_with(cx, |view, _| view.requests_for_test())
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
            assert!(view.last_create.is_none());
        });
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

    /// Tab walks the Make a backup group in page order (the web's DOM
    /// order) on to Choose backup file, and Enter on Make backup opens the
    /// save dialog.
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
        // Back on Make backup, Enter opens the save dialog once.
        press(cx, "shift-tab");
        assert_eq!(focused(cx).as_ref(), order.get(6));
        press(cx, "enter");
        assert!(cx.did_prompt_for_new_path());
        let downloads = tempfile::tempdir().unwrap();
        let chosen = downloads.path().join("bruhs-backup.bruhsbak");
        let target = chosen.clone();
        cx.simulate_new_path_selection(move |_| Some(target));
        cx.run_until_parked();
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("POST /api/backup")
        );
        view.read_with(cx, |view, _| {
            let sent = view.last_create.clone().unwrap();
            assert_eq!(sent.destination.as_deref(), chosen.to_str());
            assert!(!sent.overwrite, "a new file replaces nothing");
            assert_eq!(sent.options, BackupOptions::default());
            assert!(view.busy);
        });
    }

    /// A file the save dialog already asked about is replaced; the switches
    /// go with the request.
    #[gpui::test]
    fn an_existing_file_is_replaced_only_after_the_dialog_asked(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        click(cx, super::option_selector(OptionKind::Browsers));
        let [first, second, _] = fields(&view, cx);
        type_into(cx, &first, GOOD);
        type_into(cx, &second, GOOD);
        click(cx, "desktop-backup-make");
        let file = tempfile::NamedTempFile::new().unwrap();
        let target = file.path().to_path_buf();
        cx.simulate_new_path_selection(move |_| Some(target));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let sent = view.last_create.clone().unwrap();
            assert!(sent.overwrite);
            assert!(sent.options.browser_profiles);
        });
    }

    #[gpui::test]
    fn a_job_shows_its_line_then_the_saved_file(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        view.update(cx, |view, cx| {
            view.busy = true;
            view.job_answered(
                Slot::Make,
                ID.into(),
                Ok(JobState::Running {
                    done_bytes: 1_000,
                    total_bytes: 9_000,
                    step: JobStep::Writing,
                }),
                cx,
            );
        });
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-backup-make-progress"));
        assert!(rendered(cx, "desktop-backup-make-cancel"));
        assert_eq!(
            requests(&view, cx).last().cloned(),
            Some(format!("GET /api/backup/jobs/{ID}"))
        );
        click(cx, "desktop-backup-make-cancel");
        assert_eq!(
            requests(&view, cx).last().cloned(),
            Some(format!("DELETE /api/backup/jobs/{ID}"))
        );
        let file = BackupFile {
            name: "bruhs-backup-2026-11-20-0930.bruhsbak".into(),
            bytes: 2_000_000,
            path: Some("/Users/me/Downloads/bruhs-backup-2026-11-20-0930.bruhsbak".into()),
            download: None,
            warnings: vec!["1 link was left out.".into()],
        };
        view.update(cx, |view, cx| {
            view.job_answered(
                Slot::Make,
                ID.into(),
                Ok(JobState::Created { file: file.clone() }),
                cx,
            );
        });
        cx.run_until_parked();
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
    }

    #[gpui::test]
    fn a_stopped_backup_reads_as_a_note(cx: &mut TestAppContext) {
        assert_eq!(error_color(BACKUP_STOPPED), theme::rambutan_400());
        assert_eq!(error_color(PASSWORDS_DIFFER), theme::lychee_300());
        let (view, cx) = page(cx, true);
        view.update(cx, |view, cx| {
            view.busy = true;
            view.job_answered(
                Slot::Make,
                ID.into(),
                Ok(JobState::Failed {
                    message: BACKUP_STOPPED.into(),
                }),
                cx,
            );
        });
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
        // Delete, confirmed.
        click(cx, &row("delete"));
        click(cx, &row("confirm-button"));
        assert_eq!(
            requests(&view, cx).last().cloned(),
            Some(format!("DELETE /api/backup/pre-restore/{STAMP}"))
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.deleted(STAMP.into(), Ok(()), window, cx)
            });
        });
        cx.run_until_parked();
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("GET /api/backup")
        );
        view.read_with(cx, |view, _| assert_eq!(view.asking, None));
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
        let row = |name: &str| format!("desktop-backup-kept-{name}-{STAMP}");
        click(cx, &row("put-back"));
        click(cx, &row("confirm-button"));
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("POST /api/backup/restore/apply")
        );
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.restore
                    .last_apply
                    .as_ref()
                    .map(|apply| (&apply.source, apply.from_desktop)),
                Some((
                    &bruhs_models::backup::RestoreSource::PreRestore {
                        stamp: STAMP.into()
                    },
                    true
                ))
            );
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.applied(Err(refused(NOTHING_TO_PUT_BACK)), window, cx);
            });
        });
        cx.run_until_parked();
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
            made_at_ms: bruhs_models::EpochMs::saturating_from_i64(1_763_631_000_000),
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
        busy.job = Some(bruhs_models::backup::BackupJob {
            id: ID.into(),
            state: JobState::Running {
                done_bytes: 0,
                total_bytes: 1,
                step: JobStep::Checking,
            },
        });
        view.update(cx, |view, cx| {
            let serial = view.load_serial;
            view.loaded(serial, Ok(busy), cx);
        });
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-backup-restore-progress"));
        view.read_with(cx, |view, _| {
            assert!(view.busy);
            assert_eq!(view.making, Making::Idle);
        });
        assert!(requests(&view, cx).contains(&format!("GET /api/backup/jobs/{ID}")));
    }
}
