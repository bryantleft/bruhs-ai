//! Settings, Backup (D24): the last backup under the title, the browser
//! data card while a restored backup's browser part waits (D19), then Make
//! a backup, Restore and Kept from earlier restores. Every rule and line is
//! `bruhs_models::backup`'s, so the desktop page matches.
//!
//! Passwords never sit in a signal. Their inputs are never filled, are read
//! and emptied in one step on submit, and are emptied on cancel and
//! unmount; the page keeps only whether the two typed passwords pass the
//! rule ([`PairCheck`]). The one password kept past its form is the
//! restore's, held in `Zeroizing` until the browser part is restored or
//! Settings closes (D22). Nothing here reaches IndexedDB or browser
//! storage, and no password is logged.
//!
//! Each view ends in `.into_any()`: without it the page's nested view
//! types make the debug wasm link (`rust-lld`) crash.

mod restore;
pub(crate) mod tabs;

use std::collections::BTreeMap;

use bruhs_models::backup::{
    created_line, delete_pre_restore_confirm, job_line, last_backup_line, option_label,
    outside_roots_line, password_pair_error, pre_restore_line, put_back_confirm, validate_password,
    BackupApp, BackupFile, BackupJob, BackupOptions, BackupOverview, CreateBackup, JobState,
    JobStep, OptionKind, PreRestoreInfo, RestoreSource, BACKUP_LOADING, BACKUP_STOPPED, CANCEL,
    DELETE, DELETE_AGAIN, DOWNLOAD, KEEP, KEPT_GROUP, KEYCHAIN_OPTION_HINT,
    KEYCHAIN_SET_UNREADABLE, LOCKING_WEB, MAKE_BACKUP, MAKE_GROUP, NOTHING_TO_PUT_BACK, PASSWORD,
    PASSWORD_AGAIN, PASSWORD_HINT, PUT_BACK, UNSUPPORTED_PLATFORM, WEB_INCLUDED,
};
use bruhs_models::crew::ServiceNotice;
use leptos::{html, prelude::*};
use wasm_bindgen::JsCast;
use zeroize::{Zeroize, Zeroizing};

use super::vault_settings::{
    focus_after_render, unique_id, BUTTON, DANGER_BUTTON, FIELD, HINT, LABEL,
};
use crate::api::{backup as api, ApiError};
use crate::storage::backup_dump::{dump_web, lock_dump};
use crate::ui::Switch;

const GROUP: &str = "px-4 py-3";
const GROUP_TITLE: &str = "text-label text-rambutan-100";
const NOTE: &str = "mt-1 text-label-sm text-rambutan-400";
const PROGRESS: &str = "mt-2 text-label-sm text-rambutan-300";
const ERROR: &str = "mt-1 text-label-sm text-lychee-300 empty:hidden";
/// A stopped job's line, in the neutral tone (M11).
const STOPPED: &str = "mt-1 text-label-sm text-rambutan-400 empty:hidden";
const WARN: &str = "mt-1 text-label-sm text-durian-300";
const DONE: &str = "mt-2 text-label-sm text-pandan-300";
const ACTIONS: &str = "mt-3 flex flex-wrap items-center gap-2";
/// How often a running job is read (D6).
const JOB_POLL_MS: u32 = 500;
/// One frame, so the page paints its progress line before a key
/// derivation holds the thread (D7).
const PAINT_MS: u32 = 16;

#[derive(Clone, PartialEq, Debug)]
enum Load {
    Loading,
    Ready,
    Failed(String),
    /// An older server: no `/api/backup` (the desktop's notice).
    ServingOld,
}

/// What the two Make a backup fields say, without either password: the
/// rule's message under each and whether **Make backup** may run.
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
#[derive(Clone, PartialEq)]
enum Making {
    Idle,
    /// `LOCKING_WEB`, then the job's line once it runs.
    Working {
        line: String,
        job: Option<String>,
    },
    Made {
        line: String,
        download: Option<String>,
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
#[derive(Clone, PartialEq, Default)]
struct KeptNote {
    error: Option<String>,
    /// `NOTHING_TO_PUT_BACK`: only Delete is offered.
    nothing_to_put_back: bool,
    /// `KEYCHAIN_SET_UNREADABLE`: a delete was cut short; Delete it again.
    unreadable: bool,
}

/// Where a running job's line and outcome show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Make,
    Restore,
}

/// The group a job belongs to, from its step: opening a backup checks and
/// prepares it, making one collects, reads the keychain and writes. Used
/// for a job another tab started, found on load.
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

/// A failed job's line: a stopped backup is the owner's own doing, so it
/// reads as a note, not an error (B Task 14 M11).
fn error_class(message: &str) -> &'static str {
    if message == BACKUP_STOPPED {
        STOPPED
    } else {
        ERROR
    }
}

/// The page's state, shared by every group.
#[derive(Clone, Copy)]
struct Page {
    overview: RwSignal<BackupOverview>,
    load: RwSignal<Load>,
    /// A job, apply or browser restore runs: every action waits.
    busy: RwSignal<bool>,
    options: RwSignal<BackupOptions>,
    check: RwSignal<PairCheck>,
    first_left: RwSignal<bool>,
    making: RwSignal<Making>,
    make_error: RwSignal<Option<String>>,
    first: NodeRef<html::Input>,
    second: NodeRef<html::Input>,
    restore: restore::Restore,
    asking: RwSignal<Option<(String, Ask)>>,
    notes: RwSignal<BTreeMap<String, KeptNote>>,
    /// Aborted when Settings closes, which drops every call and poll.
    abort: StoredValue<Option<web_sys::AbortController>, LocalStorage>,
    /// The section, which holds focus when the control that had it goes
    /// (the desktop's page focus).
    section: NodeRef<html::Div>,
}

impl Page {
    fn new() -> Self {
        Self {
            overview: RwSignal::new(BackupOverview::default()),
            load: RwSignal::new(Load::Loading),
            busy: RwSignal::new(false),
            options: RwSignal::new(BackupOptions::default()),
            check: RwSignal::new(PairCheck::default()),
            first_left: RwSignal::new(false),
            making: RwSignal::new(Making::Idle),
            make_error: RwSignal::new(None),
            first: NodeRef::new(),
            second: NodeRef::new(),
            restore: restore::Restore::new(),
            asking: RwSignal::new(None),
            notes: RwSignal::new(BTreeMap::new()),
            abort: StoredValue::new_local(None),
            section: NodeRef::new(),
        }
    }

    /// The page's abort signal, while the page is open in a browser.
    fn signal(self) -> Option<web_sys::AbortSignal> {
        self.abort
            .try_get_value()
            .flatten()
            .map(|controller| controller.signal())
    }

    /// Reads `GET /api/backup` again.
    fn refresh(self) {
        let Some(signal) = self.signal() else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            match api::overview(&signal).await {
                Ok(overview) => self.loaded(overview),
                Err(ApiError::Aborted) => {}
                Err(error) if error.is_status(404) => {
                    self.load.try_set(Load::ServingOld);
                }
                Err(error) => {
                    self.load.try_set(Load::Failed(message(error)));
                }
            }
        });
    }

    fn loaded(self, overview: BackupOverview) {
        self.restore.loaded(&overview);
        let job = overview.job.clone();
        self.overview.try_set(overview);
        self.load.try_set(Load::Ready);
        // A job another tab started: follow it in its own group until it
        // finishes. Only the tab that made a backup downloads it (M3).
        if let Some(job) = job.filter(|job| job_line(&job.state).is_some()) {
            if self.busy.try_get_untracked() == Some(false) {
                self.busy.set(true);
                let slot = slot_of(&job.state);
                wasm_bindgen_futures::spawn_local(async move {
                    match self.follow(job, slot).await {
                        Ok(JobState::Created { file }) => self.made(&file, false),
                        Ok(JobState::Opened { preview }) => self.restore.opened(preview),
                        Ok(JobState::Failed { message }) | Err(message) => {
                            self.fail(slot, message);
                        }
                        Ok(_) => {}
                    }
                    self.busy.try_set(false);
                });
            }
        }
    }

    /// Polls `job` every 500 ms, showing its line in `slot`, and answers
    /// the state it finished in.
    async fn follow(self, job: BackupJob, slot: Slot) -> Result<JobState, String> {
        let signal = self.signal().ok_or_else(|| ApiError::Aborted.to_string())?;
        let BackupJob { id, mut state } = job;
        while let Some(line) = job_line(&state) {
            self.show_line(slot, line, &id);
            gloo_timers::future::TimeoutFuture::new(JOB_POLL_MS).await;
            state = api::job(&id, &signal).await.map_err(message)?.state;
        }
        Ok(state)
    }

    /// A job's failure, under the group it belongs to (M2).
    fn fail(self, slot: Slot, message: String) {
        match slot {
            Slot::Make => {
                self.making.try_set(Making::Idle);
                self.make_error.try_set(Some(message));
            }
            Slot::Restore => self.restore.fail(message),
        }
    }

    fn show_line(self, slot: Slot, line: String, id: &str) {
        let job = Some(id.to_owned());
        match slot {
            Slot::Make => {
                self.making.try_set(Making::Working { line, job });
            }
            Slot::Restore => {
                self.restore
                    .step
                    .try_set(restore::Restoring::Working { line, job });
            }
        }
    }

    /// A finished backup: its line, warnings and Download, which the page
    /// clicks once when `click` (this tab made it).
    fn made(self, file: &BackupFile, click: bool) {
        let download = download_href(file);
        let click = click && download.is_some();
        self.making.try_set(Making::Made {
            line: created_line(file, BackupApp::Web),
            download,
            warnings: file.warnings.clone(),
        });
        if click {
            request_animation_frame(|| {
                let link = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| {
                        document
                            .query_selector("[data-backup-download]")
                            .ok()
                            .flatten()
                    })
                    .and_then(|link| link.dyn_into::<web_sys::HtmlElement>().ok());
                if let Some(link) = link {
                    link.click();
                }
            });
        }
    }

    /// Make backup: reads and empties both fields, then locks this
    /// browser's data, uploads it, starts the job and follows it.
    fn make(self) {
        if self.busy.get_untracked() || !self.check.get_untracked().ready {
            return;
        }
        let password = Zeroizing::new(take_value(self.first));
        let again = Zeroizing::new(take_value(self.second));
        self.check.set(PairCheck::default());
        self.first_left.set(false);
        if let Some(error) = password_pair_error(&password, &again) {
            self.make_error.set(Some(error.to_owned()));
            return;
        }
        drop(again);
        let Some(signal) = self.signal() else {
            return;
        };
        let options = self.options.get_untracked();
        self.busy.set(true);
        self.make_error.set(None);
        self.making.set(Making::Working {
            line: LOCKING_WEB.to_owned(),
            job: None,
        });
        wasm_bindgen_futures::spawn_local(async move {
            match self.make_backup(password, options, &signal).await {
                Ok(JobState::Created { file }) => self.made(&file, true),
                Ok(JobState::Failed { message }) | Err(message) => {
                    self.fail(Slot::Make, message);
                }
                Ok(_) => {
                    self.making.try_set(Making::Idle);
                }
            }
            self.busy.try_set(false);
        });
    }

    async fn make_backup(
        self,
        mut password: Zeroizing<String>,
        options: BackupOptions,
        signal: &web_sys::AbortSignal,
    ) -> Result<JobState, String> {
        gloo_timers::future::TimeoutFuture::new(PAINT_MS).await;
        let dump = dump_web().await?;
        let web_db_version = dump.db_version;
        let locked = lock_dump(&dump, &password)?;
        drop(dump);
        let web_part = api::upload_web_part(&locked, signal)
            .await
            .map_err(message)?;
        drop(locked);
        let mut body = CreateBackup {
            password: std::mem::take(&mut *password),
            options,
            app: BackupApp::Web,
            web_part: Some(web_part),
            web_db_version: Some(web_db_version),
            destination: None,
            overwrite: false,
        };
        let job = api::create(&body, signal).await;
        body.password.zeroize();
        self.follow(job.map_err(message)?, Slot::Make).await
    }

    /// Stops a running job; its poll then answers the stop. A refusal
    /// shows under the job's own group (M2).
    fn cancel_job(self, id: String, slot: Slot) {
        let Some(signal) = self.signal() else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = api::cancel_job(&id, &signal).await {
                match slot {
                    Slot::Make => {
                        self.make_error.try_set(Some(message(error)));
                    }
                    Slot::Restore => self.restore.show_error(message(error)),
                }
            }
        });
    }

    fn note(self, stamp: &str, change: impl FnOnce(&mut KeptNote)) {
        self.notes
            .try_update(|notes| change(notes.entry(stamp.to_owned()).or_default()));
    }

    fn delete_kept(self, stamp: String) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(signal) = self.signal() else {
            return;
        };
        self.busy.set(true);
        self.note(&stamp, |note| note.error = None);
        wasm_bindgen_futures::spawn_local(async move {
            match api::delete_kept(&stamp, &signal).await {
                Ok(()) => {
                    self.asking.try_set(None);
                    self.notes.try_update(|notes| {
                        notes.remove(&stamp);
                    });
                    self.busy.try_set(false);
                    // The row goes with its confirm: the section keeps focus.
                    focus_in_page(self.section);
                    self.refresh();
                }
                Err(error) => {
                    self.note(&stamp, |note| note.error = Some(message(error)));
                    self.busy.try_set(false);
                }
            }
        });
    }

    /// Put back, confirmed. A refusal closes the confirm, notes why on the
    /// row and calls `refocus` (N4).
    fn put_back(self, stamp: String, refocus: impl Fn() + 'static) {
        let key = stamp.clone();
        restore::apply(
            self,
            RestoreSource::PreRestore { stamp },
            move |error: ApiError| self.put_back_refused(&key, error, refocus),
        );
    }

    /// A Put back the server refused. One cut short because Settings
    /// closed says nothing: the page is gone (final review D-I1).
    fn put_back_refused(self, stamp: &str, error: ApiError, refocus: impl Fn()) {
        if error == ApiError::Aborted {
            return;
        }
        let refused = |copy: &str| matches!(&error, ApiError::Http { detail: Some(detail), .. } if detail == copy);
        let nothing = refused(NOTHING_TO_PUT_BACK);
        let unreadable = refused(KEYCHAIN_SET_UNREADABLE);
        self.asking.try_set(None);
        self.note(stamp, |note| {
            note.nothing_to_put_back |= nothing;
            note.unreadable |= unreadable;
            note.error = Some(message(error));
        });
        refocus();
    }

    fn clear_fields(self) {
        clear_value(self.first);
        clear_value(self.second);
        self.restore.clear_fields();
    }
}

/// What a failed call says, worded as on the desktop.
fn message(error: ApiError) -> String {
    api::backup_message(&error)
}

/// Focuses `target` once it is drawn, when focus is on this section or
/// nowhere: the control that held it went with the view it was in, or was
/// disabled while it worked (final review D-I2). Focus the owner moved
/// elsewhere stays there.
fn focus_in_page<E>(target: NodeRef<E>)
where
    E: leptos::tachys::html::element::ElementType,
    E::Output: JsCast + Clone + 'static,
{
    // A native test build has no page to focus.
    if cfg!(not(target_arch = "wasm32")) {
        return;
    }
    request_animation_frame(move || {
        let Some(Some(element)) = target.try_get_untracked() else {
            return;
        };
        let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() else {
            return;
        };
        let free = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_none_or(|active| {
                active.tag_name() == "BODY"
                    || active
                        .closest("[data-backup-settings]")
                        .ok()
                        .flatten()
                        .is_some()
            });
        if free {
            let _ = element.focus();
        }
    });
}

/// The full download URL of a finished web backup, from its
/// `/api/backup/files/{id}` path; `None` for anything else.
fn download_href(file: &BackupFile) -> Option<String> {
    let id = file
        .download
        .as_deref()?
        .strip_prefix("/api/backup/files/")?;
    api::download_url(id)
}

/// An input's value, emptied in the same step.
fn take_value(node: NodeRef<html::Input>) -> String {
    node.try_get_untracked()
        .flatten()
        .map(|input| {
            let value = input.value();
            input.set_value("");
            value
        })
        .unwrap_or_default()
}

fn clear_value(node: NodeRef<html::Input>) {
    if let Some(Some(input)) = node.try_get_untracked() {
        input.set_value("");
    }
}

/// A password field's wiring: the elements that describe it, whether it
/// holds a refused value, and what typing and leaving it do.
struct FieldHooks<I, C> {
    /// Ids of the hint and message under the field, if any (M9).
    described_by: Option<String>,
    invalid: Signal<bool>,
    on_input: I,
    on_change: C,
}

impl FieldHooks<fn(), fn()> {
    /// A field nothing else describes and nothing checks as it is typed.
    fn plain() -> Self {
        Self {
            described_by: None,
            invalid: Signal::default(),
            on_input: || {},
            on_change: || {},
        }
    }
}

/// A password input: never filled, emptied when it leaves the page, and
/// its keys kept from the app's shortcuts.
fn password_input<I: Fn() + 'static, C: Fn() + 'static>(
    node: NodeRef<html::Input>,
    label: &'static str,
    autocomplete: &'static str,
    hooks: FieldHooks<I, C>,
) -> AnyView {
    let FieldHooks {
        described_by,
        invalid,
        on_input,
        on_change,
    } = hooks;
    on_cleanup(move || clear_value(node));
    view! {
        <label class=LABEL>{label}
            <input node_ref=node type="password" autocomplete=autocomplete autocapitalize="none" spellcheck="false" aria-describedby=described_by aria-invalid=move || invalid.get().then_some("true") class=FIELD on:input=move |_| on_input() on:change=move |_| on_change() on:keydown=move |event: web_sys::KeyboardEvent| event.stop_propagation() />
        </label>
    }
    .into_any()
}

#[component]
pub fn BackupSettings() -> impl IntoView {
    let page = Page::new();
    if let Ok(controller) = web_sys::AbortController::new() {
        page.abort.set_value(Some(controller));
    }
    on_cleanup(move || {
        page.clear_fields();
        page.restore.forget_password();
        if let Some(Some(controller)) = page.abort.try_get_value() {
            controller.abort();
        }
    });
    page.restore.take_carried();
    Effect::new(move |_| page.refresh());
    page_view(page)
}

/// What the section draws. A refresh sets the same state again; the memo
/// keeps that from drawing the groups again, so focus and typed passwords
/// stay. The parts that follow the overview update themselves.
fn shown(page: Page) -> Memo<(Load, bool)> {
    Memo::new(move |_| {
        (
            page.load.get(),
            page.overview.with(|overview| overview.supported),
        )
    })
}

fn page_view(page: Page) -> AnyView {
    let shown = shown(page);
    view! {
        <div node_ref=page.section data-backup-settings="" tabindex="-1" class="divide-y divide-longan-800/70 text-body-sm text-rambutan-200 focus:outline-none">
            {move || match shown.get() {
                (Load::Loading, _) => view! { <p role="status" class="px-4 py-3 text-label-sm text-rambutan-400">{BACKUP_LOADING}</p> }.into_any(),
                (Load::Failed(message), _) => view! { <p role="alert" class="px-4 py-3 text-label-sm text-lychee-300">{message}</p> }.into_any(),
                (Load::ServingOld, _) => view! { <p role="status" class="px-4 py-3 text-label-sm text-durian-300">{ServiceNotice::ServingOld.message()}</p> }.into_any(),
                (Load::Ready, false) => view! { <p role="status" class="px-4 py-3 text-label-sm text-durian-300">{UNSUPPORTED_PLATFORM}</p> }.into_any(),
                (Load::Ready, true) => ready_view(page),
            }}
        </div>
    }
    .into_any()
}

fn ready_view(page: Page) -> AnyView {
    let lines = Memo::new(move |_| {
        page.overview.with(|overview| {
            (
                overview.last.as_ref().map(last_backup_line),
                outside_roots_line(&overview.outside_roots),
            )
        })
    });
    view! {
        {move || {
            let (last, outside) = lines.get();
            (last.is_some() || outside.is_some()).then(|| view! {
                <div class="px-4 py-2.5">
                    {last.map(|line| view! { <p data-backup-last="" class="text-label-sm text-rambutan-400">{line}</p> })}
                    {outside.map(|line| view! { <p data-backup-outside="" class=WARN>{line}</p> })}
                </div>
            })
        }}
        {restore::browser_card(page)}
        {make_group(page)}
        {restore::restore_group(page)}
        {kept_group(page)}
    }
    .into_any()
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

fn option_is_on(mut options: BackupOptions, kind: OptionKind) -> bool {
    *option_on(&mut options, kind)
}

/// One switch. Its row is drawn again only when its label's amount
/// changes, not on every refresh.
fn option_row(page: Page, kind: OptionKind, hint: Option<&'static str>) -> AnyView {
    let label = Memo::new(move |_| {
        page.overview
            .with(|overview| option_label(kind, option_amount(overview, kind)))
    });
    let checked = Signal::derive(move || option_is_on(page.options.get(), kind));
    let on_change = Callback::new(move |on: bool| {
        page.options
            .update(|options| *option_on(options, kind) = on);
    });
    view! {
        {move || {
            let label = label.get();
            view! {
                <div data-backup-option="" class="flex items-center justify-between gap-3">
                    <div class="min-w-0">
                        <p class="text-label text-rambutan-200">{label.clone()}</p>
                        {hint.map(|hint| view! { <p class=HINT>{hint}</p> })}
                    </div>
                    <Switch checked=checked on_change=on_change disabled=page.busy label=label />
                </div>
            }
        }}
    }
    .into_any()
}

fn make_group(page: Page) -> AnyView {
    let prefix = unique_id("backup-make");
    let hint_id = format!("{prefix}-hint");
    let first_error_id = format!("{prefix}-first-error");
    let second_error_id = format!("{prefix}-second-error");
    let recheck = move || {
        let first = Zeroizing::new(
            page.first
                .try_get_untracked()
                .flatten()
                .map(|input| input.value())
                .unwrap_or_default(),
        );
        let second = Zeroizing::new(
            page.second
                .try_get_untracked()
                .flatten()
                .map(|input| input.value())
                .unwrap_or_default(),
        );
        page.check.set(PairCheck::of(
            &first,
            &second,
            page.first_left.get_untracked(),
        ));
    };
    let first_field = password_input(
        page.first,
        PASSWORD,
        "new-password",
        FieldHooks {
            described_by: Some(format!("{hint_id} {first_error_id}")),
            invalid: Signal::derive(move || page.check.get().first.is_some()),
            on_input: recheck,
            on_change: move || {
                page.first_left.set(true);
                recheck();
            },
        },
    );
    let second_field = password_input(
        page.second,
        PASSWORD_AGAIN,
        "new-password",
        FieldHooks {
            described_by: Some(second_error_id.clone()),
            invalid: Signal::derive(move || page.check.get().second.is_some()),
            on_input: recheck,
            on_change: recheck,
        },
    );
    let disabled = move || page.busy.get() || !page.check.get().ready;
    view! {
        <div data-backup-make="" class=GROUP>
            <h3 class=GROUP_TITLE>{MAKE_GROUP}</h3>
            // The group's own note, above the switches, so it isn't read as
            // the last switch's hint (M7).
            <p data-backup-web-included="" class=NOTE>{WEB_INCLUDED}</p>
            <div class="mt-3 space-y-3">
                {option_row(page, OptionKind::Keychain, Some(KEYCHAIN_OPTION_HINT))}
                {option_row(page, OptionKind::Audio, None)}
                {option_row(page, OptionKind::Browsers, None)}
            </div>
            <form class="mt-3 grid gap-3 sm:grid-cols-2" on:submit=move |event| { event.prevent_default(); page.make(); }>
                <div class="min-w-0">
                    {first_field}
                    <p id=hint_id class=HINT>{PASSWORD_HINT}</p>
                    <p id=first_error_id role="alert" class=ERROR>{move || page.check.get().first}</p>
                </div>
                <div class="min-w-0">
                    {second_field}
                    <p id=second_error_id role="alert" data-backup-second-error="" class=ERROR>{move || page.check.get().second}</p>
                </div>
                <div class="flex flex-wrap items-center gap-2 sm:col-span-2">
                    <button type="submit" data-backup-make-button="" class=BUTTON disabled=disabled>{MAKE_BACKUP}</button>
                    {move || match page.making.get() {
                        Making::Working { job: Some(id), .. } => view! { <button type="button" class=BUTTON on:click=move |_| page.cancel_job(id.clone(), Slot::Make)>{CANCEL}</button> }.into_any(),
                        _ => ().into_any(),
                    }}
                </div>
            </form>
            // One live region for the whole job, whose text changes (M4).
            <div role="status" data-backup-make-status="">
                {move || match page.making.get() {
                    Making::Idle => ().into_any(),
                    Making::Working { line, .. } => view! { <p data-backup-make-progress="" class=PROGRESS>{line}</p> }.into_any(),
                    Making::Made { line, warnings, .. } => view! {
                        <p data-backup-made="" class=DONE>{line}</p>
                        {warnings.into_iter().map(|warning| view! { <p class=WARN>{warning}</p> }).collect_view()}
                    }.into_any(),
                }}
            </div>
            {move || match page.making.get() {
                Making::Made { download: Some(href), .. } => view! { <a data-backup-download="" href=href download="" class=format!("{BUTTON} mt-2 inline-flex items-center")>{DOWNLOAD}</a> }.into_any(),
                _ => ().into_any(),
            }}
            <p role="alert" data-backup-make-error="" class=move || page.make_error.with(|error| error_class(error.as_deref().unwrap_or_default()))>{move || page.make_error.get()}</p>
        </div>
    }
    .into_any()
}

fn kept_group(page: Page) -> AnyView {
    let sets = move || page.overview.with(|overview| overview.pre_restores.clone());
    view! {
        <Show when=move || page.overview.with(|overview| !overview.pre_restores.is_empty())>
            <div data-backup-kept="" class=GROUP>
                <h3 class=GROUP_TITLE>{KEPT_GROUP}</h3>
                <div class="divide-y divide-longan-800">
                    <For each=sets key=|info| info.stamp.clone() children=move |info| kept_row(page, info) />
                </div>
            </div>
        </Show>
    }
    .into_any()
}

/// Whether a refusal left a kept set only Delete, while its row is there.
fn only_delete(note: Signal<KeptNote>) -> Option<bool> {
    note.try_with_untracked(|note| note.nothing_to_put_back || note.unreadable)
}

fn kept_row(page: Page, info: PreRestoreInfo) -> AnyView {
    let stamp = StoredValue::new(info.stamp.clone());
    let line = pre_restore_line(&info);
    let note = Signal::derive(move || {
        page.notes
            .with(|notes| notes.get(&stamp.get_value()).cloned().unwrap_or_default())
    });
    let asking = Signal::derive(move || {
        page.asking.with(|asking| {
            asking
                .as_ref()
                .filter(|(asked, _)| *asked == stamp.get_value())
                .map(|(_, ask)| *ask)
        })
    });
    let question_id = unique_id("backup-kept-question");
    let put_back_ref = NodeRef::<html::Button>::new();
    let delete_ref = NodeRef::<html::Button>::new();
    let decline_ref = NodeRef::<html::Button>::new();
    // The confirm takes focus on its safe choice; leaving it gives focus
    // back to the button that opened it (I3).
    let ask = move |ask: Ask| {
        page.note(&stamp.get_value(), |note| note.error = None);
        page.asking.set(Some((stamp.get_value(), ask)));
        focus_after_render(decline_ref);
    };
    let keep = move || {
        let asked = asking.get_untracked();
        page.asking.set(None);
        match asked {
            Some(Ask::PutBack) => focus_after_render(put_back_ref),
            Some(Ask::Delete) => focus_after_render(delete_ref),
            None => {}
        }
    };
    // After a refused Put back: Put back again, or Delete when the refusal
    // took Put back away.
    let refocus = move || {
        // The row may be gone by now (D-I1).
        let Some(only_delete) = only_delete(note) else {
            return;
        };
        focus_after_render(if only_delete {
            delete_ref
        } else {
            put_back_ref
        });
    };
    let delete_question = delete_pre_restore_confirm(&info);
    let put_back_question = put_back_confirm(&info);
    view! {
        <article data-backup-kept-row=info.stamp class="min-w-0 py-3">
            <p class="text-body-sm text-rambutan-200">{line}</p>
            <Show when=move || asking.get().is_none()>
                <div class=ACTIONS>
                    <Show when=move || note.with(|note| !note.nothing_to_put_back && !note.unreadable)>
                        <button type="button" node_ref=put_back_ref data-backup-put-back="" class=BUTTON disabled=move || page.busy.get() on:click=move |_| ask(Ask::PutBack)>{PUT_BACK}</button>
                    </Show>
                    <button type="button" node_ref=delete_ref data-backup-delete="" class=BUTTON disabled=move || page.busy.get() on:click=move |_| ask(Ask::Delete)>{move || if note.with(|note| note.unreadable) { DELETE_AGAIN } else { DELETE }}</button>
                </div>
            </Show>
            {move || asking.get().map(|ask| {
                let (question, confirm, confirm_class, decline) = match ask {
                    Ask::Delete => (delete_question.clone(), DELETE, DANGER_BUTTON, KEEP),
                    Ask::PutBack => (put_back_question.clone(), PUT_BACK, DANGER_BUTTON, CANCEL),
                };
                view! {
                    <div role="group" data-backup-kept-confirm="" aria-labelledby=question_id.clone() class="mt-3 space-y-2 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50" on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { event.prevent_default(); event.stop_propagation(); keep(); }>
                        <p id=question_id.clone() class="text-body-sm text-rambutan-200">{question}</p>
                        <div class="flex flex-wrap items-center gap-2">
                            <button type="button" data-backup-confirm="" class=confirm_class disabled=move || page.busy.get() on:click=move |_| match ask {
                                Ask::Delete => page.delete_kept(stamp.get_value()),
                                Ask::PutBack => page.put_back(stamp.get_value(), refocus),
                            }>{confirm}</button>
                            <button type="button" node_ref=decline_ref data-backup-decline="" class=BUTTON disabled=move || page.busy.get() on:click=move |_| keep()>{decline}</button>
                        </div>
                    </div>
                }
            })}
            <p role="alert" class=ERROR>{move || note.with(|note| note.error.clone())}</p>
        </article>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    use bruhs_models::backup::{BackupSizes, BACKUP_TITLE, PASSWORDS_DIFFER, PASSWORD_SHORT};
    use bruhs_models::EpochMs;

    use crate::components::settings_page::SECTIONS;

    pub(super) const STAMP: &str = "2026-11-20-0930-0a1f";

    pub(super) fn kept_set() -> PreRestoreInfo {
        PreRestoreInfo {
            stamp: STAMP.into(),
            made_at_ms: EpochMs::new(1_763_631_000_000).unwrap(),
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

    /// A page holding `overview`, as it is once `GET /api/backup` answered.
    pub(super) fn loaded_page() -> Page {
        let page = Page::new();
        page.overview.set(overview());
        page.load.set(Load::Ready);
        page
    }

    pub(super) fn render(view: impl FnOnce() -> AnyView) -> String {
        untrack(|| tachys::view::RenderHtml::to_html(view()))
    }

    /// Text as the renderer escapes it.
    pub(super) fn escaped(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// The opening tag of the element carrying `marker`.
    pub(super) fn tag<'a>(html: &'a str, marker: &str) -> &'a str {
        let at = html
            .find(marker)
            .unwrap_or_else(|| panic!("{marker} in {html}"));
        let start = html[..at].rfind('<').unwrap();
        let end = at + html[at..].find('>').unwrap();
        &html[start..=end]
    }

    /// Whether a tag has the `disabled` attribute (not a `disabled:` class).
    pub(super) fn is_disabled(tag: &str) -> bool {
        tag.match_indices(" disabled")
            .any(|(at, word)| !tag[at + word.len()..].starts_with(':'))
    }

    /// Every `<input>` tag of type password.
    pub(super) fn password_tags(html: &str) -> Vec<&str> {
        html.match_indices("<input")
            .map(|(at, _)| &html[at..=at + html[at..].find('>').unwrap()])
            .filter(|tag| tag.contains("type=\"password\""))
            .collect()
    }

    #[test]
    fn the_section_sits_after_people() {
        // The desktop keeps it beside Maintenance, in its Application
        // group; the web has no Maintenance, so it follows People there.
        let people = SECTIONS
            .iter()
            .position(|(id, _, _)| *id == "people")
            .unwrap();
        assert_eq!(
            SECTIONS.get(people + 1),
            Some(&("backup", "archive", BACKUP_TITLE))
        );
    }

    #[test]
    fn make_backup_needs_matching_passwords() {
        Owner::new().with(|| {
            let page = loaded_page();
            let html = render(|| make_group(page));
            for (kind, amount) in [
                (OptionKind::Keychain, 7),
                (OptionKind::Audio, 2_500_000),
                (OptionKind::Browsers, 700_000),
            ] {
                let label = escaped(&option_label(kind, amount));
                assert!(html.contains(&label), "{label} in {html}");
            }
            assert!(html.contains("Meeting audio (2.5 MB)"), "{html}");
            assert!(html.contains(&escaped(WEB_INCLUDED)), "{html}");
            assert!(html.contains(&escaped(PASSWORD_HINT)), "{html}");
            assert!(is_disabled(tag(&html, "data-backup-make-button")));
            assert!(!html.contains(&escaped(PASSWORDS_DIFFER)), "{html}");

            page.check
                .set(PairCheck::of("correct horse 1", "correct horse 2", false));
            let html = render(|| make_group(page));
            assert!(is_disabled(tag(&html, "data-backup-make-button")));
            let second = tag(&html, "data-backup-second-error");
            let after = &html[html.find(second).unwrap() + second.len()..];
            assert!(
                after.starts_with(&escaped(PASSWORDS_DIFFER)),
                "the mismatch sits under the second field: {html}"
            );

            page.check.set(PairCheck::of("short", "", true));
            let html = render(|| make_group(page));
            assert!(is_disabled(tag(&html, "data-backup-make-button")));
            assert!(html.contains(PASSWORD_SHORT), "{html}");

            page.check
                .set(PairCheck::of("correct horse 1", "correct horse 1", false));
            let html = render(|| make_group(page));
            assert!(
                !is_disabled(tag(&html, "data-backup-make-button")),
                "{html}"
            );
            assert!(!html.contains(&escaped(PASSWORDS_DIFFER)), "{html}");

            page.busy.set(true);
            let html = render(|| make_group(page));
            assert!(is_disabled(tag(&html, "data-backup-make-button")));
        });
    }

    #[test]
    fn the_pair_check_waits_for_the_owner() {
        // Nothing typed, or a first field still being typed: no message yet.
        assert_eq!(PairCheck::of("", "", false), PairCheck::default());
        assert_eq!(PairCheck::of("short", "", false).first, None);
        assert_eq!(PairCheck::of("short", "", true).first, Some(PASSWORD_SHORT));
        assert_eq!(
            PairCheck::of("short", "s", false).first,
            Some(PASSWORD_SHORT)
        );
        // The mismatch only once the second field has text and the first is fine.
        assert_eq!(PairCheck::of("correct horse 1", "", true).second, None);
        assert_eq!(PairCheck::of("short", "other", true).second, None);
        let ready = PairCheck::of("correct horse 1", "correct horse 1", true);
        assert_eq!((ready.first, ready.second, ready.ready), (None, None, true));
    }

    #[test]
    fn password_fields_are_private() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore.web_waiting_for_tests();
            let make = render(|| make_group(page));
            let restore = render(|| restore::restore_group(page));
            let card = render(|| restore::browser_card(page));
            let make_fields = password_tags(&make);
            assert_eq!(make_fields.len(), 2, "{make}");
            for field in &make_fields {
                assert!(field.contains("autocomplete=\"new-password\""), "{field}");
            }
            let restore_fields: Vec<&str> = password_tags(&restore)
                .into_iter()
                .chain(password_tags(&card))
                .collect();
            assert_eq!(restore_fields.len(), 2, "{restore}{card}");
            for field in &restore_fields {
                assert!(
                    field.contains("autocomplete=\"current-password\""),
                    "{field}"
                );
            }
            for field in make_fields.iter().chain(&restore_fields) {
                assert!(!field.contains("value"), "{field}");
            }
        });
    }

    #[test]
    fn kept_sets_ask_before_acting() {
        Owner::new().with(|| {
            let page = loaded_page();
            let info = kept_set();
            let html = render(|| kept_group(page));
            assert!(html.contains(&escaped(&pre_restore_line(&info))), "{html}");
            assert!(html.contains("data-backup-put-back"), "{html}");
            assert!(html.contains("data-backup-delete"), "{html}");
            assert!(!html.contains("data-backup-kept-confirm"), "{html}");

            page.asking.set(Some((STAMP.into(), Ask::Delete)));
            let html = render(|| kept_group(page));
            let question = escaped(&delete_pre_restore_confirm(&info));
            let confirm = &html[html.find("data-backup-kept-confirm").unwrap()..];
            assert!(confirm.contains(&question), "{html}");
            let delete = confirm.find(&format!(">{DELETE}</button>")).unwrap();
            let keep = confirm.find(&format!(">{KEEP}</button>")).unwrap();
            assert!(delete < keep, "{html}");
            assert!(!html.contains("data-backup-delete"), "{html}");

            page.asking.set(Some((STAMP.into(), Ask::PutBack)));
            let html = render(|| kept_group(page));
            assert!(html.contains(&escaped(&put_back_confirm(&info))), "{html}");
            assert!(html.contains(&format!(">{CANCEL}</button>")), "{html}");
        });
    }

    #[test]
    fn a_set_with_nothing_or_no_keys_offers_only_delete() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.note(STAMP, |note| {
                note.nothing_to_put_back = true;
                note.error = Some(NOTHING_TO_PUT_BACK.into());
            });
            let html = render(|| kept_group(page));
            assert!(!html.contains("data-backup-put-back"), "{html}");
            assert!(html.contains(&format!(">{DELETE}</button>")), "{html}");
            assert!(html.contains(&escaped(NOTHING_TO_PUT_BACK)), "{html}");

            page.note(STAMP, |note| {
                note.nothing_to_put_back = false;
                note.unreadable = true;
                note.error = Some(KEYCHAIN_SET_UNREADABLE.into());
            });
            let html = render(|| kept_group(page));
            assert!(!html.contains("data-backup-put-back"), "{html}");
            assert!(
                html.contains(&format!(">{DELETE_AGAIN}</button>")),
                "{html}"
            );
        });
    }

    #[test]
    fn the_page_shows_where_it_stands() {
        Owner::new().with(|| {
            let page = Page::new();
            let html = render(|| page_view(page));
            assert!(html.contains(BACKUP_LOADING), "{html}");

            page.load
                .set(Load::Failed("Backup runs on macOS and Linux.".into()));
            let html = render(|| page_view(page));
            assert!(html.contains("Backup runs on macOS and Linux."), "{html}");

            page.load.set(Load::Ready);
            let html = render(|| page_view(page));
            assert!(html.contains(UNSUPPORTED_PLATFORM), "{html}");
            assert!(!html.contains("data-backup-make"), "{html}");

            let mut full = overview();
            full.last = Some(bruhs_models::backup::LastBackup {
                made_at_ms: EpochMs::new(1_763_631_000_000).unwrap(),
                app: BackupApp::Web,
                bytes: 3_000_000,
            });
            full.outside_roots = vec!["/srv/reactive.sqlite3".into()];
            let last = last_backup_line(full.last.as_ref().unwrap());
            let outside = outside_roots_line(&full.outside_roots).unwrap();
            page.overview.set(full);
            let html = render(|| page_view(page));
            let last_at = html.find(&escaped(&last)).expect("last line");
            assert!(html[last_at..].contains(&escaped(&outside)), "{html}");
            let make_at = html.find("data-backup-make").unwrap();
            let restore_at = html.find("data-backup-restore").unwrap();
            let kept_at = html.find("data-backup-kept").unwrap();
            assert!(last_at < make_at && make_at < restore_at && restore_at < kept_at);
        });
    }

    #[test]
    fn jobs_land_in_their_own_group() {
        let running = |step| JobState::Running {
            done_bytes: 0,
            total_bytes: 1,
            step,
        };
        for step in [JobStep::Checking, JobStep::Preparing] {
            assert_eq!(slot_of(&running(step)), Slot::Restore, "{step:?}");
        }
        for step in [
            JobStep::Collecting,
            JobStep::Keychain,
            JobStep::Writing,
            JobStep::Other,
        ] {
            assert_eq!(slot_of(&running(step)), Slot::Make, "{step:?}");
        }
        Owner::new().with(|| {
            let page = loaded_page();
            page.fail(Slot::Restore, "open failed".into());
            assert_eq!(page.make_error.get_untracked(), None);
            let html = render(|| restore::restore_group(page));
            assert!(html.contains("open failed"), "{html}");
            page.fail(Slot::Make, "make failed".into());
            assert_eq!(
                page.make_error.get_untracked().as_deref(),
                Some("make failed")
            );
        });
    }

    #[test]
    fn a_stopped_backup_reads_as_a_note() {
        assert_eq!(error_class(BACKUP_STOPPED), STOPPED);
        assert_eq!(error_class(PASSWORDS_DIFFER), ERROR);
        Owner::new().with(|| {
            let page = loaded_page();
            page.make_error.set(Some(BACKUP_STOPPED.into()));
            let html = render(|| make_group(page));
            let error = tag(&html, "data-backup-make-error");
            assert!(error.contains("text-rambutan-400"), "{error}");
            assert!(!error.contains("text-lychee"), "{error}");
        });
    }

    #[test]
    fn make_progress_has_one_live_region() {
        Owner::new().with(|| {
            let page = loaded_page();
            let html = render(|| make_group(page));
            // The live region is there before any job, so its first line is read.
            let status = tag(&html, "data-backup-make-status");
            assert!(status.contains("role=\"status\""), "{status}");
            assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
            page.making.set(Making::Working {
                line: LOCKING_WEB.into(),
                job: None,
            });
            let html = render(|| make_group(page));
            let inside = &html[html.find("data-backup-make-status").unwrap()..];
            assert!(
                inside.find(&escaped(LOCKING_WEB)).unwrap() < inside.find("</div>").unwrap(),
                "{html}"
            );
            assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
            // The browser-data line opens the group, before the switches (M7).
            let web = html.find("data-backup-web-included").unwrap();
            assert!(web < html.find("data-backup-option").unwrap(), "{html}");
        });
    }

    #[test]
    fn only_refused_fields_are_marked() {
        Owner::new().with(|| {
            let page = loaded_page();
            let html = render(|| make_group(page));
            assert!(!html.contains("aria-invalid"), "{html}");
            page.check
                .set(PairCheck::of("correct horse 1", "correct horse 2", false));
            let html = render(|| make_group(page));
            let fields = password_tags(&html);
            assert!(!fields[0].contains("aria-invalid"), "{}", fields[0]);
            assert!(fields[1].contains("aria-invalid=\"true\""), "{}", fields[1]);
            // A field nothing describes says so by having no attribute (M9).
            let restore = render(|| restore::restore_group(page));
            for field in password_tags(&restore) {
                assert!(!field.contains("aria-describedby"), "{field}");
            }
        });
    }

    #[test]
    fn a_put_back_cut_short_by_closing_settings_says_nothing() {
        let owner = Owner::new();
        let (page, note) = owner.with(|| {
            let page = loaded_page();
            let note = Signal::derive(move || {
                page.notes
                    .with(|notes| notes.get(STAMP).cloned().unwrap_or_default())
            });
            (page, note)
        });
        assert_eq!(owner.with(|| only_delete(note)), Some(false));
        owner.cleanup();
        assert!(page.notes.is_disposed(), "the page's signals are gone");
        // The row is gone: no target, no panic (a wasm trap).
        assert_eq!(only_delete(note), None);
        let refocused = std::cell::Cell::new(false);
        page.put_back_refused(STAMP, ApiError::Aborted, || refocused.set(true));
        assert!(!refocused.get(), "an abort says and focuses nothing");
        page.put_back_refused(
            STAMP,
            ApiError::Http {
                status: 409,
                detail: Some(NOTHING_TO_PUT_BACK.into()),
            },
            || {},
        );
    }

    #[test]
    fn a_refused_put_back_notes_why_on_its_row() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.asking.set(Some((STAMP.into(), Ask::PutBack)));
            let refocused = std::cell::Cell::new(false);
            page.put_back_refused(
                STAMP,
                ApiError::Http {
                    status: 409,
                    detail: Some(NOTHING_TO_PUT_BACK.into()),
                },
                || refocused.set(true),
            );
            assert!(refocused.get());
            assert_eq!(page.asking.get_untracked(), None);
            let note = page.notes.with_untracked(|notes| notes[STAMP].clone());
            assert!(note.nothing_to_put_back && !note.unreadable);
            assert_eq!(note.error.as_deref(), Some(NOTHING_TO_PUT_BACK));
        });
    }

    #[test]
    fn a_refresh_keeps_the_groups_it_drew() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        Owner::new().with(|| {
            let page = loaded_page();
            let shown = shown(page);
            let builds = Arc::new(AtomicUsize::new(0));
            let count = Arc::clone(&builds);
            let drawn = Memo::new(move |_| {
                count.fetch_add(1, Ordering::Relaxed);
                shown.get()
            });
            assert_eq!(drawn.get_untracked(), (Load::Ready, true));
            // What a refresh after a Delete does: the same state again, with
            // one kept set fewer.
            page.overview
                .update(|overview| overview.pre_restores.clear());
            page.load.set(Load::Ready);
            assert_eq!(drawn.get_untracked(), (Load::Ready, true));
            assert_eq!(builds.load(Ordering::Relaxed), 1, "not drawn again");
            // The lines under the title still follow the overview.
            page.overview.update(|overview| {
                overview.outside_roots = vec!["/srv/reactive.sqlite3".into()];
            });
            let html = render(|| page_view(page));
            assert!(html.contains("/srv/reactive.sqlite3"), "{html}");
            assert!(!html.contains("data-backup-kept"), "{html}");
        });
    }

    #[test]
    fn an_older_server_reads_as_on_the_desktop() {
        Owner::new().with(|| {
            let page = Page::new();
            page.load.set(Load::ServingOld);
            let html = render(|| page_view(page));
            assert!(
                html.contains(&escaped(ServiceNotice::ServingOld.message())),
                "{html}"
            );
            assert!(!html.contains("role=\"alert\""), "{html}");
        });
    }

    #[test]
    fn a_made_backup_offers_its_download() {
        Owner::new().with(|| {
            let page = loaded_page();
            let file = BackupFile {
                name: "bruhs-backup-2026-11-20-0930.bruhsbak".into(),
                bytes: 2_000_000,
                path: None,
                download: Some("/api/backup/files/0123456789abcdef0123456789abcdef".into()),
                warnings: vec!["1 link was left out.".into()],
            };
            let href = download_href(&file).unwrap();
            assert!(href.ends_with("/api/backup/files/0123456789abcdef0123456789abcdef"));
            page.making.set(Making::Made {
                line: created_line(&file, BackupApp::Web),
                download: Some(href.clone()),
                warnings: file.warnings.clone(),
            });
            let html = render(|| make_group(page));
            assert!(
                html.contains("Backup ready: bruhs-backup-2026-11-20-0930.bruhsbak, 2.0 MB."),
                "{html}"
            );
            assert!(html.contains("1 link was left out."), "{html}");
            let link = tag(&html, "data-backup-download");
            assert!(link.starts_with("<a"), "{link}");
            assert!(link.contains(&format!("href=\"{href}\"")), "{link}");
            assert!(link.contains("download"), "{link}");

            for bad in [
                None,
                Some("https://elsewhere.example/x"),
                Some("/api/backup/files/../vault"),
            ] {
                let mut other = file.clone();
                other.download = bad.map(str::to_owned);
                assert_eq!(download_href(&other), None, "{bad:?}");
            }

            page.making.set(Making::Working {
                line: LOCKING_WEB.into(),
                job: None,
            });
            let html = render(|| make_group(page));
            assert!(html.contains(&escaped(LOCKING_WEB)), "{html}");
        });
    }
}
