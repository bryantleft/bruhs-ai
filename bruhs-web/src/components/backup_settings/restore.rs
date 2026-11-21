//! Settings, Backup: Restore (D8, D15, D19), the wait while Bruhs stops and
//! comes back, and the browser data card.
//!
//! Restore sends the chosen file (the browser streams it from disk), opens
//! it with the typed password, shows what it holds and replaces this
//! computer's data only after **Replace my data**. The password is kept,
//! in `Zeroizing`, only when the backup has browser data, so the card can
//! put that back without asking again; it goes once the browser data is
//! restored or Settings closes (D22).
//!
//! The card fetches the waiting browser part, opens it, keeps this
//! browser's current data with the newest kept set, rewrites paths with
//! the restore's rules, then replaces this browser's data while every other
//! Bruhs tab reloads ([`super::tabs`]). Only then does it tell the server
//! the part is done, and the page reloads. That last step doesn't stop
//! when Settings closes: this browser's data is already back.

use bruhs_models::backup::{
    preview_lines, rewrite_dump, validate_password, ApplyRestore, BackupOverview, JobState,
    OpenBackup, OpenSource, RestorePreview, RestoreSource, RestoreStatus, CANCEL, CHOOSE_FILE,
    GONE, NO_WEB_PART, OPEN_BACKUP, PASSWORD, REPLACE, REPLACE_WARNING, RESTORED, RESTORE_BROWSER,
    RESTORE_GROUP, RESTORE_PENDING, RESTORE_ROLLED_BACK, RESTORING_WEB, START_AGAIN, STOPPING,
    UPLOADING_BACKUP, WEB_FINISH_RETRY, WEB_PART_DONE, WEB_PART_PASSWORD_HINT, WEB_PART_WAITING,
    WEB_STORES_SKIPPED, WRONG_PASSWORD,
};
use leptos::{html, prelude::*};
use zeroize::{Zeroize, Zeroizing};

use super::{
    error_class, focus_in_page, message, password_input, take_value, FieldHooks, Page, Slot,
    ACTIONS, GROUP, GROUP_TITLE, PAINT_MS, PROGRESS, WARN,
};
use crate::api::{backup as api, ApiError};
use crate::components::vault_settings::{unique_id, BUTTON, DANGER_BUTTON, HINT};
use crate::storage::backup_dump::{dump_web, lock_dump, restore_web, unlock_dump};

/// How often the page asks whether Bruhs is back (D24).
const HEALTH_POLL_MS: u32 = 2_000;
/// With no background service to bring it back, Bruhs is started by hand.
const START_AGAIN_AFTER_MS: u32 = 20_000;
/// How long `WEB_PART_DONE` shows before the page reloads.
const RELOAD_AFTER_MS: u32 = 1_000;
/// How long it shows with the stores a newer Bruhs wrote, so that line
/// can be read before the reload.
const RELOAD_AFTER_NOTE_MS: u32 = 5_000;
/// Tries at telling the server the browser part is done.
const DONE_TRIES: u32 = 3;

/// The Restore group's progress.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Restoring {
    Idle,
    /// `UPLOADING_BACKUP`, then the open job's line.
    Working {
        line: String,
        job: Option<String>,
    },
    Preview(Box<RestorePreview>),
    /// The apply was sent; its answer hasn't come.
    Applying,
    /// Bruhs stops and comes back; `START_AGAIN` once that takes long.
    Stopping {
        start_again: bool,
    },
    Finished {
        lines: Vec<String>,
        rolled_back: bool,
    },
    /// Bruhs came back with the restore still waiting.
    Pending,
}

/// Where focus goes when the view that held it changes (D-I2), as on the
/// desktop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Focus {
    /// A new preview's safe choice.
    Cancel,
    /// The form, back after a discard.
    Choose,
    /// The preview drawn again after a refused apply.
    Replace,
}

/// The browser data card's progress.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Browser {
    Idle,
    Working,
    Done,
}

/// The Restore group's and the card's state.
#[derive(Clone, Copy)]
pub(super) struct Restore {
    pub(super) step: RwSignal<Restoring>,
    error: RwSignal<Option<String>>,
    /// The chosen file's name, shown beside **Choose backup file**.
    chosen: RwSignal<Option<String>>,
    file: StoredValue<Option<web_sys::File>, LocalStorage>,
    file_input: NodeRef<html::Input>,
    password: NodeRef<html::Input>,
    /// The restore password, kept for the card (D22).
    held: StoredValue<Option<Zeroizing<String>>>,
    holds_password: RwSignal<bool>,
    web_waiting: RwSignal<bool>,
    browser: RwSignal<Browser>,
    browser_error: RwSignal<Option<String>>,
    /// This browser's data is already back; only the server's copy is left
    /// to drop, so a retry must not keep the restored data as the old.
    browser_restored: RwSignal<bool>,
    /// The stores a newer Bruhs wrote that the restore left out.
    skipped: RwSignal<Option<String>>,
    card_password: NodeRef<html::Input>,
    /// Where focus last went when the view that held it changed (D-I2).
    focus: RwSignal<Option<Focus>>,
    choose_button: NodeRef<html::Button>,
    replace_button: NodeRef<html::Button>,
    cancel_button: NodeRef<html::Button>,
}

impl Restore {
    pub(super) fn new() -> Self {
        Self {
            step: RwSignal::new(Restoring::Idle),
            error: RwSignal::new(None),
            chosen: RwSignal::new(None),
            file: StoredValue::new_local(None),
            file_input: NodeRef::new(),
            password: NodeRef::new(),
            held: StoredValue::new(None),
            holds_password: RwSignal::new(false),
            web_waiting: RwSignal::new(false),
            browser: RwSignal::new(Browser::Idle),
            browser_error: RwSignal::new(None),
            browser_restored: RwSignal::new(false),
            skipped: RwSignal::new(None),
            card_password: NodeRef::new(),
            focus: RwSignal::new(None),
            choose_button: NodeRef::new(),
            replace_button: NodeRef::new(),
            cancel_button: NodeRef::new(),
        }
    }

    /// The overview's restore state, unless this page is already showing
    /// one of its own.
    pub(super) fn loaded(self, overview: &BackupOverview) {
        self.web_waiting.try_set(overview.web_part_waiting);
        if self.step.try_get_untracked() != Some(Restoring::Idle) {
            return;
        }
        if overview.pending.is_some() {
            self.step.try_set(Restoring::Pending);
        } else if let Some(staged) = &overview.staged {
            self.step
                .try_set(Restoring::Preview(Box::new(staged.preview.clone())));
        }
    }

    /// An open job that landed: the preview takes focus on its Cancel,
    /// the safe choice, as the form that held it is gone (D-I2).
    pub(super) fn opened(self, preview: Box<RestorePreview>) {
        self.step.try_set(Restoring::Preview(preview));
        self.move_focus(Focus::Cancel);
    }

    /// Focus moves to `target` once it is drawn, unless the owner moved
    /// it away from the page.
    fn move_focus(self, target: Focus) {
        self.focus.try_set(Some(target));
        focus_in_page(match target {
            Focus::Cancel => self.cancel_button,
            Focus::Choose => self.choose_button,
            Focus::Replace => self.replace_button,
        });
    }

    /// An open job that failed: the form comes back with its message.
    pub(super) fn fail(self, message: String) {
        self.step.try_set(Restoring::Idle);
        self.error.try_set(Some(message));
    }

    pub(super) fn show_error(self, message: String) {
        self.error.try_set(Some(message));
    }

    fn hold(self, password: Zeroizing<String>) {
        self.held.try_set_value(Some(password));
        self.holds_password.try_set(true);
    }

    /// Drops the kept restore password; the card asks for it instead.
    pub(super) fn forget_password(self) {
        // The old value is wiped as it drops.
        self.held.try_set_value(None);
        self.holds_password.try_set(false);
    }

    fn held_copy(self) -> Option<Zeroizing<String>> {
        self.held
            .try_with_value(|held| {
                held.as_ref()
                    .map(|password| Zeroizing::new(String::clone(password)))
            })
            .flatten()
    }

    fn forget_file(self) {
        self.file.try_set_value(None);
        self.chosen.try_set(None);
        if let Some(Some(input)) = self.file_input.try_get_untracked() {
            input.set_value("");
        }
    }

    /// A restored backup's browser part waits, as the overview says.
    #[cfg(test)]
    pub(super) fn web_waiting_for_tests(self) {
        self.web_waiting.set(true);
    }

    pub(super) fn clear_fields(self) {
        super::clear_value(self.password);
        super::clear_value(self.card_password);
    }
}

/// The hidden file input changed: keep the chosen file.
fn chose(page: Page) {
    let restore = page.restore;
    let file = restore
        .file_input
        .get_untracked()
        .and_then(|input| input.files())
        .and_then(|files| files.get(0));
    restore.chosen.set(file.as_ref().map(web_sys::File::name));
    restore.file.set_value(file);
    restore.error.set(None);
}

/// Open backup: reads and empties the password, sends the file, opens it
/// and follows the job to the preview.
fn open_backup(page: Page) {
    let restore = page.restore;
    if page.busy.get_untracked() {
        return;
    }
    let Some(file) = restore.file.get_value() else {
        return;
    };
    let password = Zeroizing::new(take_value(restore.password));
    if let Err(error) = validate_password(&password) {
        restore.error.set(Some(error.to_owned()));
        return;
    }
    let Some(signal) = page.signal() else {
        return;
    };
    page.busy.set(true);
    restore.error.set(None);
    restore.step.set(Restoring::Working {
        line: UPLOADING_BACKUP.to_owned(),
        job: None,
    });
    wasm_bindgen_futures::spawn_local(async move {
        match opening(page, &file, password, &signal).await {
            Ok(JobState::Opened { preview }) => {
                restore.forget_file();
                restore.opened(preview);
            }
            Ok(JobState::Failed { message }) | Err(message) => restore.fail(message),
            Ok(_) => {
                restore.step.try_set(Restoring::Idle);
            }
        }
        page.busy.try_set(false);
    });
}

async fn opening(
    page: Page,
    file: &web_sys::File,
    password: Zeroizing<String>,
    signal: &web_sys::AbortSignal,
) -> Result<JobState, String> {
    let id = api::upload_restore(file, signal).await.map_err(message)?;
    let mut body = OpenBackup {
        source: OpenSource::Upload { id },
        password: String::clone(&password),
    };
    let job = api::open(&body, signal).await;
    body.password.zeroize();
    let state = page.follow(job.map_err(message)?, Slot::Restore).await?;
    if let JobState::Opened { preview } = &state {
        if preview.has_web_part {
            page.restore.hold(password);
        }
    }
    Ok(state)
}

/// Cancel on the preview: drops the opened backup.
fn discard(page: Page, id: String) {
    let Some(signal) = page.signal() else {
        return;
    };
    page.busy.set(true);
    wasm_bindgen_futures::spawn_local(async move {
        let result = api::discard(&id, &signal).await;
        discarded(page, result);
    });
}

fn discarded(page: Page, result: Result<(), ApiError>) {
    let restore = page.restore;
    match result {
        Ok(()) => {
            restore.forget_password();
            restore.step.try_set(Restoring::Idle);
            restore.error.try_set(None);
            // The preview and its Cancel went; the form came back.
            restore.move_focus(Focus::Choose);
        }
        // Settings closed: the page is gone (D-I1).
        Err(ApiError::Aborted) => return,
        Err(error) => {
            restore.error.try_set(Some(message(error)));
        }
    }
    page.busy.try_set(false);
}

/// Replace my data or Put back: the confirmed apply (D15), then the wait
/// while Bruhs stops and comes back. `refused` takes an answer that
/// refused it, and the group shows what it showed before.
pub(super) fn apply(page: Page, source: RestoreSource, refused: impl FnOnce(ApiError) + 'static) {
    let restore = page.restore;
    if page.busy.get_untracked() {
        return;
    }
    let Some(signal) = page.signal() else {
        return;
    };
    let before = restore.step.get_untracked();
    page.busy.set(true);
    restore.error.set(None);
    restore.step.set(Restoring::Applying);
    let body = ApplyRestore {
        source,
        replace: true,
        from_desktop: false,
    };
    wasm_bindgen_futures::spawn_local(async move {
        match api::apply(&body, &signal).await {
            Ok(accepted) => {
                restore
                    .step
                    .try_set(Restoring::Stopping { start_again: false });
                wait_for_bruhs(page, accepted.service_stopped, &signal).await;
            }
            // Settings closed: the page is gone (D-I1).
            Err(ApiError::Aborted) => {}
            Err(error) => {
                restore.step.try_set(before);
                refused(error);
                page.busy.try_set(false);
            }
        }
    });
}

/// Asks `/api/health` every 2 s; once Bruhs answers, reads the restore's
/// outcome. With no background service to bring Bruhs back, says to start
/// it again after 20 s.
async fn wait_for_bruhs(page: Page, service_stopped: bool, signal: &web_sys::AbortSignal) {
    let restore = page.restore;
    let mut waited_ms = 0_u32;
    loop {
        gloo_timers::future::TimeoutFuture::new(HEALTH_POLL_MS).await;
        if signal.aborted() {
            return;
        }
        waited_ms = waited_ms.saturating_add(HEALTH_POLL_MS);
        if api::health(signal).await {
            match api::status(signal).await {
                Ok(status) => {
                    settle(restore, status);
                    page.busy.try_set(false);
                    page.refresh();
                    return;
                }
                Err(ApiError::Aborted) => return,
                Err(_) => {}
            }
        }
        if !service_stopped && waited_ms >= START_AGAIN_AFTER_MS {
            restore
                .step
                .try_set(Restoring::Stopping { start_again: true });
        }
    }
}

/// The restore's outcome as the group's lines, and the card when a
/// browser part waits.
fn settle(restore: Restore, status: RestoreStatus) {
    let waiting = status.web_part.is_some();
    let step = match (status.pending, status.result) {
        (Some(_), _) => Restoring::Pending,
        (None, result) => {
            let warnings = result.map(|result| result.warnings).unwrap_or_default();
            let rolled_back = warnings
                .iter()
                .any(|warning| warning == RESTORE_ROLLED_BACK);
            let mut lines = Vec::new();
            if !rolled_back {
                lines.push(RESTORED.to_owned());
            }
            lines.extend(warnings);
            if !rolled_back && !waiting {
                lines.push(NO_WEB_PART.to_owned());
            }
            Restoring::Finished { lines, rolled_back }
        }
    };
    restore.step.try_set(step);
    restore.web_waiting.try_set(waiting);
    if !waiting {
        restore.forget_password();
    }
}

/// Restore browser data: the card's run, with the kept password or the
/// card's own field.
fn restore_browser(page: Page) {
    let restore = page.restore;
    if page.busy.get_untracked() {
        return;
    }
    // Once this browser's data is back only the last step is left, which
    // needs no password (N3).
    let password = if restore.browser_restored.get_untracked() {
        Zeroizing::new(String::new())
    } else {
        let typed = (!restore.holds_password.get_untracked())
            .then(|| Zeroizing::new(take_value(restore.card_password)));
        // A backup password follows the rule, so one that doesn't can't
        // open the part: say so before a key derivation (M1).
        if let Some(Err(error)) = typed.as_ref().map(|typed| validate_password(typed)) {
            restore.browser_error.set(Some(error.to_owned()));
            return;
        }
        let Some(password) = typed.or_else(|| restore.held_copy()) else {
            return;
        };
        password
    };
    let Some(signal) = page.signal() else {
        return;
    };
    page.busy.set(true);
    restore.browser.set(Browser::Working);
    restore.browser_error.set(None);
    wasm_bindgen_futures::spawn_local(async move {
        let run = put_back_browser(restore, &password, &signal).await;
        drop(password);
        match run {
            Ok(skipped) => {
                restore.forget_password();
                let note = skipped_line(&skipped);
                let wait = if note.is_some() {
                    RELOAD_AFTER_NOTE_MS
                } else {
                    RELOAD_AFTER_MS
                };
                restore.skipped.try_set(note);
                restore.browser.try_set(Browser::Done);
                gloo_timers::future::TimeoutFuture::new(wait).await;
                reload();
            }
            Err(failure) => match after_failure(signal.aborted(), failure) {
                AfterFailure::Quiet => {}
                AfterFailure::Reload => reload(),
                AfterFailure::Show { message } => show_failure(page, message),
            },
        }
    });
}

fn reload() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

/// Why a browser restore run stopped.
#[derive(Debug, Clone, PartialEq)]
struct Failure {
    message: String,
    /// This browser's data is back; only the server's copy is left.
    restored: bool,
    /// This tab's own writes stay stopped.
    fenced: bool,
}

impl Failure {
    /// A failure before this tab stopped its writes or wrote anything.
    fn before_writing(message: String) -> Self {
        Self {
            message,
            restored: false,
            fenced: false,
        }
    }
}

/// What a failed run does next.
#[derive(Debug, PartialEq)]
enum AfterFailure {
    /// Settings closed before this tab wrote: nothing to say, and the
    /// page's state is gone.
    Quiet,
    /// Settings closed while this tab's writes are stopped: it reloads
    /// onto whatever IndexedDB holds instead of hanging. The card comes
    /// back while the server's copy waits.
    Reload,
    Show {
        message: String,
    },
}

fn after_failure(left: bool, failure: Failure) -> AfterFailure {
    match (left, failure.fenced) {
        (true, true) => AfterFailure::Reload,
        (true, false) => AfterFailure::Quiet,
        (false, _) => AfterFailure::Show {
            message: browser_failure(failure.restored, failure.message),
        },
    }
}

/// A failed run on the page, which may have closed meanwhile: every
/// write is a `try_` (D-I1).
fn show_failure(page: Page, message: String) {
    let restore = page.restore;
    if message == WRONG_PASSWORD {
        restore.forget_password();
    }
    restore.browser.try_set(Browser::Idle);
    restore.browser_error.try_set(Some(message));
    page.busy.try_set(false);
}

/// What a failed run says. Once this browser's data is back, a failure can
/// only be the server's copy not going, and this tab's own writes stay
/// stopped: it says so in fixed words, with a retry of that step (N3).
fn browser_failure(restored: bool, message: String) -> String {
    if restored {
        WEB_FINISH_RETRY.to_owned()
    } else {
        message
    }
}

/// The line for the stores a newer Bruhs wrote, if the restore left any
/// out (D19).
fn skipped_line(skipped: &[String]) -> Option<String> {
    (!skipped.is_empty()).then(|| WEB_STORES_SKIPPED.replace("{list}", &skipped.join(", ")))
}

/// D19's steps. The ciphertext goes before anything is written, and the
/// server's copy goes only once this browser's data is back. This tab's
/// own writes stop before the other tabs are asked to leave, and the
/// restore reads and writes through its own connection (N2); a run that
/// stops before it writes lets them go on. Answers the stores the restore
/// left out.
async fn put_back_browser(
    restore: Restore,
    password: &str,
    signal: &web_sys::AbortSignal,
) -> Result<Vec<String>, Failure> {
    let mut skipped = Vec::new();
    if !restore
        .browser_restored
        .try_get_untracked()
        .unwrap_or(false)
    {
        skipped = write_browser(restore, password, signal).await?;
    }
    web_part_done().await.map_err(|message| Failure {
        message,
        restored: true,
        fenced: true,
    })?;
    Ok(skipped)
}

async fn write_browser(
    restore: Restore,
    password: &str,
    signal: &web_sys::AbortSignal,
) -> Result<Vec<String>, Failure> {
    gloo_timers::future::TimeoutFuture::new(PAINT_MS).await;
    let status = api::status(signal)
        .await
        .map_err(|error| Failure::before_writing(message(error)))?;
    let rules = status
        .web_part
        .ok_or_else(|| Failure::before_writing(GONE.to_owned()))?
        .rewrites;
    let blob = api::web_part(signal)
        .await
        .map_err(|error| Failure::before_writing(message(error)))?;
    let mut dump =
        unlock_dump(&blob, password).map_err(|error| Failure::before_writing(error.into()))?;
    drop(blob);
    super::tabs::leave();
    let written = super::tabs::with_other_tabs_reloaded(async {
        let kept = async {
            let current = dump_web().await?;
            let locked = lock_dump(&current, password)?;
            drop(current);
            api::web_part_previous(&locked, signal)
                .await
                .map_err(message)?;
            drop(locked);
            rewrite_dump(&mut dump, &rules);
            Ok::<(), String>(())
        }
        .await;
        // Nothing was written yet.
        kept.map_err(|message| (message, false))?;
        // A failed restore may have written part of it (localStorage goes
        // after the IndexedDB commit), so the fence stays.
        let report = restore_web(dump).await.map_err(|message| (message, true))?;
        restore.browser_restored.try_set(true);
        Ok::<Vec<String>, (String, bool)>(report.skipped_stores)
    })
    .await;
    written.map_err(|(message, wrote)| {
        if !wrote {
            super::tabs::stay();
        }
        Failure {
            message,
            restored: false,
            fenced: wrote,
        }
    })
}

/// Drops the server's copy of the browser part, trying again on a miss.
/// This browser's data is already back, so the call isn't tied to the
/// page's signal: closing Settings doesn't cut it short (D-I1).
async fn web_part_done() -> Result<(), String> {
    let signal = web_sys::AbortController::new()
        .map_err(|_| WEB_FINISH_RETRY.to_owned())?
        .signal();
    let mut tries = 0;
    loop {
        tries += 1;
        match api::web_part_done(&signal).await {
            Ok(()) => return Ok(()),
            Err(error) if tries >= DONE_TRIES => return Err(message(error)),
            Err(_) => gloo_timers::future::TimeoutFuture::new(HEALTH_POLL_MS / 2).await,
        }
    }
}

pub(super) fn browser_card(page: Page) -> AnyView {
    let restore = page.restore;
    let hint_id = StoredValue::new(unique_id("backup-browser-hint"));
    view! {
        <Show when=move || restore.web_waiting.get()>
            <div data-backup-browser="" class=GROUP>
                <p class="text-body-sm text-rambutan-200">{WEB_PART_WAITING}</p>
                <form class=ACTIONS on:submit=move |event| { event.prevent_default(); restore_browser(page); }>
                    <Show when=move || !restore.holds_password.get() && !restore.browser_restored.get() && restore.browser.get() != Browser::Done>
                        <div class="w-full sm:w-60">
                            {password_input(restore.card_password, PASSWORD, "current-password", FieldHooks { described_by: Some(hint_id.get_value()), invalid: Signal::default(), on_input: || {}, on_change: || {} })}
                            <p id=hint_id.get_value() data-backup-browser-hint="" class=HINT>{WEB_PART_PASSWORD_HINT}</p>
                        </div>
                    </Show>
                    <button type="submit" data-backup-restore-browser="" class=format!("{BUTTON} self-end") disabled=move || page.busy.get() || restore.browser.get() == Browser::Done>{RESTORE_BROWSER}</button>
                </form>
                // One live region for the run, whose text changes (M4).
                <div role="status" data-backup-browser-status="">
                    {move || match restore.browser.get() {
                        Browser::Idle => ().into_any(),
                        Browser::Working => view! { <p class=PROGRESS>{RESTORING_WEB}</p> }.into_any(),
                        Browser::Done => view! {
                            <p data-backup-browser-done="" class="mt-2 text-label-sm text-pandan-300">{WEB_PART_DONE}</p>
                            {restore.skipped.get().map(|line| view! { <p data-backup-browser-skipped="" class=WARN>{line}</p> })}
                        }.into_any(),
                    }}
                </div>
                <p role="alert" class=move || restore.browser_error.with(|error| error_class(error.as_deref().unwrap_or_default()))>{move || restore.browser_error.get()}</p>
            </div>
        </Show>
    }
    .into_any()
}

/// Whether the open form shows: not while a preview waits for an answer
/// or Bruhs restarts. After an outcome it comes back (M10).
fn picking(step: &Restoring) -> bool {
    !matches!(
        step,
        Restoring::Preview(_) | Restoring::Applying | Restoring::Stopping { .. }
    )
}

pub(super) fn restore_group(page: Page) -> AnyView {
    let restore = page.restore;
    let picking = Memo::new(move |_| restore.step.with(picking));
    let preview = Memo::new(move |_| match restore.step.get() {
        Restoring::Preview(preview) => Some(preview),
        _ => None,
    });
    view! {
        <div data-backup-restore="" class=GROUP>
            <h3 class=GROUP_TITLE>{RESTORE_GROUP}</h3>
            <Show when=move || picking.get()>
                {open_form(page)}
            </Show>
            {move || preview.get().map(|preview| preview_view(page, *preview))}
            // One live region for every line the restore says (M4).
            <div role="status" data-backup-restore-status="">
                {move || status_view(restore.step.get())}
            </div>
            <p role="alert" data-backup-restore-error="" class=move || restore.error.with(|error| error_class(error.as_deref().unwrap_or_default()))>{move || restore.error.get()}</p>
        </div>
    }
    .into_any()
}

fn open_form(page: Page) -> AnyView {
    let restore = page.restore;
    let field = password_input(
        restore.password,
        PASSWORD,
        "current-password",
        FieldHooks::plain(),
    );
    let working_job = move || match restore.step.get() {
        Restoring::Working { job, .. } => Some(job),
        _ => None,
    };
    view! {
        <form class="mt-3 space-y-3" on:submit=move |event| { event.prevent_default(); open_backup(page); }>
            <input node_ref=restore.file_input type="file" accept=".bruhsbak" tabindex="-1" aria-hidden="true" class="sr-only" on:change=move |_| chose(page) />
            <div class="flex flex-wrap items-center gap-2">
                <button type="button" node_ref=restore.choose_button data-backup-choose="" class=BUTTON disabled=move || page.busy.get() on:click=move |_| { if let Some(input) = restore.file_input.get_untracked() { input.click(); } }>{CHOOSE_FILE}</button>
                <span data-backup-chosen="" class="min-w-0 break-all text-label-sm text-rambutan-300">{move || restore.chosen.get()}</span>
            </div>
            <div class="sm:w-80">{field}</div>
            <div class="flex flex-wrap items-center gap-2">
                <button type="submit" data-backup-open="" class=BUTTON disabled=move || page.busy.get() || restore.chosen.with(Option::is_none)>{OPEN_BACKUP}</button>
                {move || working_job().flatten().map(|id| view! { <button type="button" class=BUTTON on:click=move |_| page.cancel_job(id.clone(), Slot::Restore)>{CANCEL}</button> })}
            </div>
        </form>
    }
    .into_any()
}

/// What the restore says in its live region: that a preview came (its
/// first line, for a screen reader, as focus moves to its Cancel), progress,
/// the wait (from the moment Replace is pressed, M5), the outcome or a
/// waiting restore.
fn status_view(step: Restoring) -> AnyView {
    match step {
        Restoring::Idle => ().into_any(),
        Restoring::Preview(preview) => preview_lines(&preview)
            .into_iter()
            .next()
            .map(|line| view! { <p data-backup-preview-announce="" class="sr-only">{line}</p> })
            .into_any(),
        Restoring::Working { line, .. } => {
            view! { <p data-backup-restore-progress="" class=PROGRESS>{line}</p> }.into_any()
        }
        Restoring::Applying => view! {
            <div data-backup-stopping="">
                <p class=PROGRESS>{STOPPING}</p>
            </div>
        }
        .into_any(),
        Restoring::Stopping { start_again } => view! {
            <div data-backup-stopping="">
                <p class=PROGRESS>{STOPPING}</p>
                {start_again.then(|| view! { <p class=WARN>{START_AGAIN}</p> })}
            </div>
        }
        .into_any(),
        Restoring::Finished { lines, rolled_back } => {
            let class = if rolled_back {
                "mt-2 text-label-sm text-lychee-300"
            } else {
                "mt-2 text-label-sm text-pandan-300"
            };
            view! {
                <div data-backup-restored="">
                    {lines.into_iter().map(|line| view! { <p class=class>{line}</p> }).collect_view()}
                </div>
            }
            .into_any()
        }
        Restoring::Pending => view! { <p class=WARN>{RESTORE_PENDING}</p> }.into_any(),
    }
}

/// A refused **Replace my data**: the reason, and focus back on Replace in
/// the preview drawn again (D-I2).
fn refused_replace(restore: Restore, error: ApiError) {
    restore.show_error(message(error));
    restore.move_focus(Focus::Replace);
}

fn preview_view(page: Page, preview: RestorePreview) -> AnyView {
    let restore = page.restore;
    let lines = preview_lines(&preview);
    let id = StoredValue::new(preview.id);
    view! {
        <div data-backup-preview="" class="mt-3">
            <ul class="space-y-1">
                {lines.into_iter().map(|line| view! { <li data-backup-preview-line="" class="break-words text-body-sm text-rambutan-200">{line}</li> }).collect_view()}
            </ul>
            <p data-backup-replace-warning="" class="mt-3 text-label-sm text-durian-300">{REPLACE_WARNING}</p>
            <div class=ACTIONS>
                <button type="button" node_ref=restore.replace_button data-backup-replace="" class=DANGER_BUTTON disabled=move || page.busy.get() on:click=move |_| apply(page, RestoreSource::Staged { id: id.get_value() }, move |error| refused_replace(restore, error))>{REPLACE}</button>
                <button type="button" node_ref=restore.cancel_button data-backup-cancel="" class=BUTTON disabled=move || page.busy.get() on:click=move |_| discard(page, id.get_value())>{CANCEL}</button>
            </div>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    use bruhs_models::backup::{
        server_answered, BackupApp, BackupCounts, PathRewrite, PendingRestore, RestoreResult,
        WebPartInfo, PARKED_STAYED, RECORDING_FIRST,
    };
    use bruhs_models::EpochMs;

    use crate::components::backup_settings::tests::{
        escaped, is_disabled, loaded_page, password_tags, render, tag,
    };

    fn preview() -> RestorePreview {
        RestorePreview {
            id: "0123456789abcdef0123456789abcdef".into(),
            made_at_ms: EpochMs::new(1_763_631_000_000).unwrap(),
            made_on: BackupApp::Web,
            app_version: "0.1.1".into(),
            source_home: "/Users/old".into(),
            rewrites: vec![PathRewrite {
                from: "/Users/old".into(),
                to: "/Users/new".into(),
            }],
            outside_home: vec!["/Volumes/Work <x> & y".into()],
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

    /// The text of every element carrying `marker`, in page order.
    fn texts<'a>(html: &'a str, marker: &str) -> Vec<&'a str> {
        html.match_indices(marker)
            .map(|(at, _)| {
                let start = at + html[at..].find('>').unwrap() + 1;
                let end = start + html[start..].find('<').unwrap();
                &html[start..end]
            })
            .collect()
    }

    #[test]
    fn the_preview_reads_from_the_shared_lines() {
        Owner::new().with(|| {
            let page = loaded_page();
            let preview = preview();
            page.restore
                .step
                .set(Restoring::Preview(Box::new(preview.clone())));
            let html = render(|| restore_group(page));
            let expected: Vec<String> = preview_lines(&preview)
                .iter()
                .map(|line| escaped(line))
                .collect();
            assert_eq!(texts(&html, "data-backup-preview-line"), expected, "{html}");
            let last = html.rfind("data-backup-preview-line").unwrap();
            let warning = html.find(&escaped(REPLACE_WARNING)).unwrap();
            let replace = html.find(&format!(">{REPLACE}</button>")).unwrap();
            let cancel = html.find(&format!(">{CANCEL}</button>")).unwrap();
            assert!(
                last < warning && warning < replace && replace < cancel,
                "{html}"
            );
            // The form steps aside while the preview waits for an answer.
            assert!(!html.contains("data-backup-choose"), "{html}");
        });
    }

    #[test]
    fn restore_opens_only_a_chosen_file() {
        Owner::new().with(|| {
            let page = loaded_page();
            let html = render(|| restore_group(page));
            let input = tag(&html, "accept=\".bruhsbak\"");
            assert!(input.contains("type=\"file\""), "{input}");
            assert!(html.contains(&format!(">{CHOOSE_FILE}</button>")), "{html}");
            assert!(is_disabled(tag(&html, "data-backup-open")), "{html}");

            page.restore
                .chosen
                .set(Some("bruhs-backup-2026-11-20-0930.bruhsbak".into()));
            let html = render(|| restore_group(page));
            assert!(!is_disabled(tag(&html, "data-backup-open")), "{html}");
            assert!(
                html.contains("bruhs-backup-2026-11-20-0930.bruhsbak"),
                "{html}"
            );

            page.restore.step.set(Restoring::Working {
                line: UPLOADING_BACKUP.into(),
                job: None,
            });
            let html = render(|| restore_group(page));
            assert!(html.contains(UPLOADING_BACKUP), "{html}");
        });
    }

    #[test]
    fn the_wait_says_to_start_bruhs_again() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore
                .step
                .set(Restoring::Stopping { start_again: false });
            let html = render(|| restore_group(page));
            assert!(html.contains(&escaped(STOPPING)), "{html}");
            assert!(!html.contains(START_AGAIN), "{html}");
            page.restore
                .step
                .set(Restoring::Stopping { start_again: true });
            let html = render(|| restore_group(page));
            assert!(
                html.contains(&escaped(STOPPING)) && html.contains(START_AGAIN),
                "{html}"
            );
        });
    }

    #[test]
    fn the_outcome_comes_from_the_status() {
        Owner::new().with(|| {
            let restore = Restore::new();
            let result = |warnings: Vec<String>| RestoreResult {
                applied_at_ms: EpochMs::new(1).unwrap(),
                stamp: "2026-11-20-0930-0a1f".into(),
                warnings,
            };
            restore.hold(Zeroizing::new("correct horse 1".into()));
            settle(
                restore,
                RestoreStatus {
                    pending: None,
                    result: Some(result(vec![PARKED_STAYED.into()])),
                    web_part: Some(WebPartInfo::default()),
                },
            );
            assert_eq!(
                restore.step.get_untracked(),
                Restoring::Finished {
                    lines: vec![RESTORED.into(), PARKED_STAYED.into()],
                    rolled_back: false
                }
            );
            assert!(restore.web_waiting.get_untracked());
            assert!(restore.holds_password.get_untracked(), "kept for the card");

            settle(
                restore,
                RestoreStatus {
                    pending: None,
                    result: Some(result(Vec::new())),
                    web_part: None,
                },
            );
            assert_eq!(
                restore.step.get_untracked(),
                Restoring::Finished {
                    lines: vec![RESTORED.into(), NO_WEB_PART.into()],
                    rolled_back: false
                }
            );
            assert!(!restore.web_waiting.get_untracked());
            assert!(!restore.holds_password.get_untracked(), "nothing needs it");
            assert!(restore.held_copy().is_none());

            settle(
                restore,
                RestoreStatus {
                    pending: None,
                    result: Some(result(vec![RESTORE_ROLLED_BACK.into()])),
                    web_part: None,
                },
            );
            assert_eq!(
                restore.step.get_untracked(),
                Restoring::Finished {
                    lines: vec![RESTORE_ROLLED_BACK.into()],
                    rolled_back: true
                }
            );

            settle(
                restore,
                RestoreStatus {
                    pending: Some(PendingRestore::default()),
                    result: None,
                    web_part: None,
                },
            );
            assert_eq!(restore.step.get_untracked(), Restoring::Pending);
        });
    }

    #[test]
    fn the_wait_shows_from_the_click_and_the_form_comes_back() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore.step.set(Restoring::Applying);
            let html = render(|| restore_group(page));
            assert!(html.contains(&escaped(STOPPING)), "{html}");
            assert!(!html.contains("data-backup-choose"), "{html}");
            page.restore.step.set(Restoring::Finished {
                lines: vec![RESTORED.into()],
                rolled_back: false,
            });
            let html = render(|| restore_group(page));
            assert!(html.contains(&escaped(RESTORED)), "{html}");
            assert!(
                html.contains("data-backup-choose"),
                "the form comes back: {html}"
            );
            let status = &html[html.find("data-backup-restore-status").unwrap()..];
            assert!(status.contains(&escaped(RESTORED)), "{html}");
            assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
        });
    }

    #[test]
    fn the_card_refuses_an_empty_password_before_deriving() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore.web_waiting_for_tests();
            restore_browser(page);
            assert_eq!(
                page.restore.browser_error.get_untracked().as_deref(),
                Some(bruhs_models::backup::PASSWORD_SHORT)
            );
            assert!(!page.busy.get_untracked());
            assert_eq!(page.restore.browser.get_untracked(), Browser::Idle);
        });
    }

    #[test]
    fn a_failed_finish_retries_only_the_last_step() {
        assert_eq!(
            browser_failure(true, "The server answered HTTP 502.".into()),
            WEB_FINISH_RETRY
        );
        assert_eq!(
            browser_failure(false, WRONG_PASSWORD.into()),
            WRONG_PASSWORD
        );
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore.web_waiting_for_tests();
            page.restore.browser_restored.set(true);
            page.restore
                .browser_error
                .set(Some(WEB_FINISH_RETRY.into()));
            let html = render(|| browser_card(page));
            assert!(html.contains(&escaped(WEB_FINISH_RETRY)), "{html}");
            assert!(
                !html.contains("type=\"password\""),
                "no password needed: {html}"
            );
            assert!(
                !is_disabled(tag(&html, "data-backup-restore-browser")),
                "{html}"
            );
            // The retry asks for nothing: no password refusal.
            page.restore.browser_error.set(None);
            restore_browser(page);
            assert_eq!(page.restore.browser_error.get_untracked(), None);
        });
    }

    #[test]
    fn the_card_asks_for_the_password_only_without_one() {
        Owner::new().with(|| {
            let page = loaded_page();
            assert!(!render(|| browser_card(page)).contains("data-backup-browser"));
            page.restore.web_waiting_for_tests();
            let html = render(|| browser_card(page));
            assert!(html.contains(&escaped(WEB_PART_WAITING)), "{html}");
            assert!(
                html.contains(&format!(">{RESTORE_BROWSER}</button>")),
                "{html}"
            );
            assert!(html.contains("type=\"password\""), "{html}");

            page.restore.hold(Zeroizing::new("correct horse 1".into()));
            let html = render(|| browser_card(page));
            assert!(!html.contains("type=\"password\""), "{html}");

            page.restore.browser.set(Browser::Done);
            let html = render(|| browser_card(page));
            assert!(html.contains(&escaped(WEB_PART_DONE)), "{html}");
            assert!(
                is_disabled(tag(&html, "data-backup-restore-browser")),
                "{html}"
            );
        });
    }

    #[test]
    fn focus_follows_the_view_that_held_it() {
        Owner::new().with(|| {
            let page = loaded_page();
            let restore = page.restore;
            // This tab's open job lands: the preview's safe choice.
            restore.opened(Box::new(preview()));
            assert_eq!(restore.focus.get_untracked(), Some(Focus::Cancel));
            // Cancel dropped it: the form is back, on Choose backup file.
            page.busy.set(true);
            discarded(page, Ok(()));
            assert_eq!(restore.focus.get_untracked(), Some(Focus::Choose));
            assert_eq!(restore.step.get_untracked(), Restoring::Idle);
            assert!(!page.busy.get_untracked());
            // A refused Replace: the preview is drawn again, on Replace.
            restore.step.set(Restoring::Preview(Box::new(preview())));
            refused_replace(
                restore,
                ApiError::Http {
                    status: 409,
                    detail: Some(RECORDING_FIRST.into()),
                },
            );
            assert_eq!(restore.focus.get_untracked(), Some(Focus::Replace));
            assert_eq!(
                restore.error.get_untracked().as_deref(),
                Some(RECORDING_FIRST)
            );
            // A refused Cancel leaves the preview, and focus, as they were.
            restore.focus.set(None);
            discarded(
                page,
                Err(ApiError::Http {
                    status: 502,
                    detail: None,
                }),
            );
            assert_eq!(restore.focus.get_untracked(), None);
            assert_eq!(restore.error.get_untracked(), Some(server_answered(502)));
        });
    }

    #[test]
    fn a_new_preview_is_announced() {
        Owner::new().with(|| {
            let page = loaded_page();
            let html = render(|| restore_group(page));
            assert!(!html.contains("data-backup-preview-announce"), "{html}");
            let first = preview_lines(&preview()).remove(0);
            page.restore
                .step
                .set(Restoring::Preview(Box::new(preview())));
            let html = render(|| restore_group(page));
            let status = &html[html.find("data-backup-restore-status").unwrap()..];
            let announce = tag(status, "data-backup-preview-announce");
            assert!(announce.contains("sr-only"), "{announce}");
            assert!(
                status.find(&escaped(&first)).unwrap() < status.find("</div>").unwrap(),
                "the first line, inside the live region: {html}"
            );
        });
    }

    #[test]
    fn a_run_cut_short_by_closing_settings_says_nothing() {
        let failure = |restored, fenced| Failure {
            message: WRONG_PASSWORD.into(),
            restored,
            fenced,
        };
        // Settings closed before this tab wrote: nothing to say.
        assert_eq!(
            after_failure(true, failure(false, false)),
            AfterFailure::Quiet
        );
        // Closed while this tab's writes are stopped: it reloads, not hangs.
        assert_eq!(
            after_failure(true, failure(false, true)),
            AfterFailure::Reload
        );
        assert_eq!(
            after_failure(true, failure(true, true)),
            AfterFailure::Reload
        );
        assert_eq!(
            after_failure(false, failure(false, false)),
            AfterFailure::Show {
                message: WRONG_PASSWORD.into()
            }
        );
        assert_eq!(
            after_failure(false, failure(true, true)),
            AfterFailure::Show {
                message: WEB_FINISH_RETRY.into()
            }
        );

        // Every step left after an await reads the page with `try_`: once
        // Settings closed, nothing panics (a wasm trap).
        let owner = Owner::new();
        let page = owner.with(loaded_page);
        owner.cleanup();
        assert!(page.busy.is_disposed(), "the page's signals are gone");
        show_failure(page, WRONG_PASSWORD.into());
        discarded(page, Err(ApiError::Aborted));
        discarded(page, Ok(()));
        refused_replace(page.restore, ApiError::Aborted);
        page.restore.opened(Box::new(preview()));
        settle(page.restore, RestoreStatus::default());
    }

    #[test]
    fn a_failed_run_shows_on_an_open_page() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.busy.set(true);
            page.restore.browser.set(Browser::Working);
            page.restore.hold(Zeroizing::new("correct horse 1".into()));
            show_failure(page, WRONG_PASSWORD.into());
            assert!(!page.busy.get_untracked());
            assert_eq!(page.restore.browser.get_untracked(), Browser::Idle);
            assert_eq!(
                page.restore.browser_error.get_untracked().as_deref(),
                Some(WRONG_PASSWORD)
            );
            assert!(!page.restore.holds_password.get_untracked(), "asks again");
        });
    }

    #[test]
    fn stores_a_newer_bruhs_wrote_are_named() {
        assert_eq!(skipped_line(&[]), None);
        assert_eq!(
            skipped_line(&["whiteboards".into(), "boards".into()]).as_deref(),
            Some("Some browser data comes from a newer Bruhs, so it was left out: whiteboards, boards.")
        );
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore.web_waiting_for_tests();
            page.restore.browser.set(Browser::Done);
            let html = render(|| browser_card(page));
            assert!(!html.contains("data-backup-browser-skipped"), "{html}");
            page.restore
                .skipped
                .set(skipped_line(&["whiteboards".into()]));
            let html = render(|| browser_card(page));
            let status = &html[html.find("data-backup-browser-status").unwrap()..];
            let done = status.find(&escaped(WEB_PART_DONE)).unwrap();
            let skipped = status.find("whiteboards").unwrap();
            assert!(
                done < skipped && skipped < status.find("</div>").unwrap(),
                "{html}"
            );
        });
    }

    #[test]
    fn the_card_says_which_password_it_needs() {
        Owner::new().with(|| {
            let page = loaded_page();
            page.restore.web_waiting_for_tests();
            let html = render(|| browser_card(page));
            let field = password_tags(&html)[0];
            let hint = tag(&html, "data-backup-browser-hint");
            let id = hint
                .split("id=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            assert!(
                field.contains(&format!("aria-describedby=\"{id}\"")),
                "{field} {hint}"
            );
            assert!(html.contains(&escaped(WEB_PART_PASSWORD_HINT)), "{html}");
            // With the restore's password kept, nothing to ask or explain.
            page.restore.hold(Zeroizing::new("correct horse 1".into()));
            let html = render(|| browser_card(page));
            assert!(!html.contains("data-backup-browser-hint"), "{html}");
        });
    }

    #[test]
    fn an_overview_puts_back_what_waits() {
        Owner::new().with(|| {
            let restore = Restore::new();
            let mut overview = BackupOverview {
                staged: Some(bruhs_models::backup::StagedRestore { preview: preview() }),
                web_part_waiting: true,
                ..BackupOverview::default()
            };
            restore.loaded(&overview);
            assert_eq!(
                restore.step.get_untracked(),
                Restoring::Preview(Box::new(preview()))
            );
            assert!(restore.web_waiting.get_untracked());

            // A page showing its own outcome keeps it.
            restore.step.set(Restoring::Stopping { start_again: false });
            overview.pending = Some(PendingRestore::default());
            restore.loaded(&overview);
            assert_eq!(
                restore.step.get_untracked(),
                Restoring::Stopping { start_again: false }
            );
            restore.step.set(Restoring::Idle);
            restore.loaded(&overview);
            assert_eq!(restore.step.get_untracked(), Restoring::Pending);
        });
    }
}
