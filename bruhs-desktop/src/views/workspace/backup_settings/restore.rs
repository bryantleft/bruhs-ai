//! Settings, Backup: Restore (D8, D15, D17). **Choose backup file** asks
//! for one file on this computer, **Open backup** opens it with the typed
//! password, the page shows what it holds and replaces this computer's
//! data only after **Replace my data**. The desktop then says Bruhs is
//! stopping and starts itself again, so the new instance applies the
//! restore before it opens anything. The page never keeps the restore
//! password: the desktop holds no browser part to put back, so while one
//! waits (an outcome's, or the overview's on any launch) the group says to
//! restore it in the web app and offers **Open the web app** (final review
//! D-I3).

use super::super::vault_settings::{button, field, labelled, small};
use super::super::*;
use super::{danger, group, group_title, line, BackupSettings, Slot};
use crate::server_client::backup::Wiped;
use crate::server_client::ServerError;
use bruhs_models::backup::{
    preview_lines, validate_password, ApplyAccepted, ApplyRestore, BackupOverview, OpenBackup,
    OpenSource, RestorePreview, RestoreSource, RestoreStatus, CANCEL, CHOOSE_FILE, INVALID_REQUEST,
    OPEN_BACKUP, OPEN_WEB_APP, PASSWORD, REPLACE, REPLACE_WARNING, RESTORED, RESTORE_GROUP,
    RESTORE_PENDING, RESTORE_ROLLED_BACK, STOPPING, WEB_PART_ON_DESKTOP,
};
use bruhs_ui::input::InputContentType;
use std::time::Duration;

/// Long enough for `STOPPING` to draw before this app quits.
const RELAUNCH_AFTER: Duration = Duration::from_millis(300);
/// A restore applied at most this long before this section was made is
/// this launch's own: its outcome shows once.
const RECENT_RESULT_MS: i64 = 10 * 60 * 1000;
/// Clock skew allowed the other way.
const RESULT_SKEW_MS: i64 = 60 * 1000;
/// Web `sm:w-80`.
const PASSWORD_WIDTH_PX: f32 = 320.0;

/// The Restore group's progress.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Restoring {
    Idle,
    /// The open job's line once it runs, and its id for **Cancel**.
    Working {
        line: Option<String>,
        job: Option<String>,
    },
    Preview(Box<RestorePreview>),
    /// The apply was sent; its answer hasn't come. Says `STOPPING` from
    /// the press (M5).
    Applying,
    /// The apply was accepted: this app starts again.
    Stopping,
    Finished {
        lines: Vec<String>,
        rolled_back: bool,
        /// The backup's browser part waits for the web app.
        browser_waits: bool,
    },
    /// A restore waits for the next start.
    Pending,
}

/// The Restore group's state.
pub(super) struct Restore {
    pub(super) step: Restoring,
    pub(super) error: Option<String>,
    /// The file **Choose backup file** picked.
    pub(super) chosen: Option<PathBuf>,
    pub(super) password: Entity<InputState>,
    /// What the group showed before an apply, for a refusal.
    before: Option<Restoring>,
    /// The source an apply on its way restores.
    applying: Option<RestoreSource>,
    /// The open dialog is up.
    choosing: bool,
    /// A preview just showed: focus moves to its Cancel.
    focus_preview: bool,
    pub(super) choose_focus: FocusHandle,
    pub(super) cancel_focus: FocusHandle,
    replace_focus: FocusHandle,
    /// **Open the web app** beside an outcome whose browser part waits.
    pub(super) web_app_focus: FocusHandle,
    prompt_task: Option<Task<()>>,
    relaunch_task: Option<Task<()>>,
}

impl Restore {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, BackupSettings>) -> Self {
        Self {
            step: Restoring::Idle,
            error: None,
            chosen: None,
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            before: None,
            applying: None,
            choosing: false,
            focus_preview: false,
            choose_focus: cx.focus_handle().tab_stop(true),
            cancel_focus: cx.focus_handle().tab_stop(true),
            replace_focus: cx.focus_handle().tab_stop(true),
            web_app_focus: cx.focus_handle().tab_stop(true),
            prompt_task: None,
            relaunch_task: None,
        }
    }

    /// Settings opened: the group starts over.
    pub(super) fn reset(&mut self) {
        self.step = Restoring::Idle;
        self.error = None;
        self.chosen = None;
        self.before = None;
        self.applying = None;
        self.focus_preview = false;
    }

    /// The overview's restore state, unless the group is already showing
    /// one of its own.
    pub(super) fn loaded(&mut self, overview: &BackupOverview) {
        if self.step != Restoring::Idle {
            return;
        }
        if overview.pending.is_some() {
            self.step = Restoring::Pending;
        } else if let Some(staged) = &overview.staged {
            self.step = Restoring::Preview(Box::new(staged.preview.clone()));
        }
    }

    pub(super) fn opened(&mut self, preview: Box<RestorePreview>) {
        self.step = Restoring::Preview(preview);
        self.chosen = None;
        self.focus_preview = true;
    }

    /// An open that failed: the form comes back with its message.
    pub(super) fn fail(&mut self, message: String) {
        self.step = Restoring::Idle;
        self.error = Some(message);
    }

    /// Whether the open form shows: not while a preview waits for an
    /// answer or Bruhs restarts. After an outcome it comes back (M10).
    fn picking(&self) -> bool {
        !matches!(
            self.step,
            Restoring::Preview(_) | Restoring::Applying | Restoring::Stopping
        )
    }
}

/// The last restore's outcome as the group's lines (the web's `settle`).
/// This app can't put the browser part back, so one that waits points at
/// the web app (D-I3).
fn outcome(status: &RestoreStatus) -> Option<Restoring> {
    if status.pending.is_some() {
        return Some(Restoring::Pending);
    }
    let warnings = status.result.as_ref()?.warnings.clone();
    let rolled_back = warnings
        .iter()
        .any(|warning| warning == RESTORE_ROLLED_BACK);
    let mut lines = Vec::new();
    if !rolled_back {
        lines.push(RESTORED.to_owned());
    }
    lines.extend(warnings);
    let browser_waits = !rolled_back && status.web_part.is_some();
    if browser_waits {
        lines.push(WEB_PART_ON_DESKTOP.to_owned());
    }
    Some(Restoring::Finished {
        lines,
        rolled_back,
        browser_waits,
    })
}

impl BackupSettings {
    /// Choose backup file: one file, from the open dialog.
    pub(super) fn choose(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy || self.restore.choosing {
            return;
        }
        self.restore.choosing = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(CHOOSE_FILE.into()),
        });
        self.restore.prompt_task = Some(cx.spawn_in(window, async move |this, cx| {
            // A dialog that couldn't open counts as cancelled.
            let paths = paths.await.ok().and_then(Result::ok).flatten();
            let _ = this.update(cx, |this, cx| this.chosen(paths, cx));
        }));
    }

    fn chosen(&mut self, paths: Option<Vec<PathBuf>>, cx: &mut Context<'_, Self>) {
        self.restore.choosing = false;
        if let Some(path) = paths.and_then(|paths| paths.into_iter().next()) {
            self.restore.chosen = Some(path);
            self.restore.error = None;
        }
        cx.notify();
    }

    /// Open backup: reads and empties the password, then opens the chosen
    /// file and follows the job to the preview.
    pub(super) fn open(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(chosen) = self.restore.chosen.clone() else {
            return;
        };
        let mut password = super::take(&self.restore.password, window, cx);
        // A backup password follows the rule, so one that doesn't can't open
        // it: say so before a key derivation (M1).
        if let Err(error) = validate_password(&password) {
            self.restore.error = Some(error.to_owned());
            cx.notify();
            return;
        }
        let Some(path) = chosen.to_str().map(str::to_owned) else {
            self.restore.error = Some(INVALID_REQUEST.to_owned());
            cx.notify();
            return;
        };
        self.busy = true;
        self.restore.error = None;
        self.restore.step = Restoring::Working {
            line: None,
            job: None,
        };
        self.hold_focus_on_page(window, cx);
        let body = Wiped(OpenBackup {
            source: OpenSource::Path { path },
            password: std::mem::take(&mut *password),
        });
        drop(password);
        cx.notify();
        let operation = self.calls.open(body, cx);
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(job) => this.follow(Slot::Restore, job, cx),
                Err(error) => this.fail(Slot::Restore, error.backup_message(), cx),
            });
        }));
    }

    /// Cancel on the preview: drops the opened backup.
    fn discard(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Restoring::Preview(preview) = &self.restore.step else {
            return;
        };
        let id = preview.id.clone();
        self.busy = true;
        self.hold_focus_on_page(window, cx);
        cx.notify();
        let operation = self.calls.discard(id, cx);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| this.discarded(result, window, cx));
        }));
    }

    pub(super) fn discarded(
        &mut self,
        result: Result<(), ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = false;
        match result {
            Ok(()) => {
                self.restore.step = Restoring::Idle;
                self.restore.error = None;
                if self.page_focus.is_focused(window) {
                    self.restore.choose_focus.focus(window, cx);
                }
            }
            Err(error) => self.restore.error = Some(error.backup_message()),
        }
        cx.notify();
    }

    /// Replace my data or Put back: the confirmed apply (D15), from this
    /// app, so the server lets it run while this app is open (D17).
    pub(super) fn apply(
        &mut self,
        source: RestoreSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.restore.error = None;
        self.restore.before = Some(std::mem::replace(
            &mut self.restore.step,
            Restoring::Applying,
        ));
        self.restore.applying = Some(source.clone());
        self.hold_focus_on_page(window, cx);
        let body = ApplyRestore {
            source,
            replace: true,
            from_desktop: true,
        };
        cx.notify();
        let operation = self.calls.apply(body, cx);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| this.applied(result, window, cx));
        }));
    }

    /// The server answered an apply. Accepted: say Bruhs is stopping and
    /// start this app again. Refused: the group shows what it showed
    /// before with the reason, and a waiting restore shows once the
    /// overview is read again.
    pub(super) fn applied(
        &mut self,
        result: Result<ApplyAccepted, ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let source = self.restore.applying.take();
        let before = self.restore.before.take().unwrap_or(Restoring::Idle);
        match result {
            Ok(_) => {
                self.restore.step = Restoring::Stopping;
                self.restore.relaunch_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(RELAUNCH_AFTER).await;
                    let _ = this.update(cx, |this, cx| {
                        let relaunch = this.relaunch.clone();
                        relaunch(cx);
                    });
                }));
            }
            Err(error) => {
                self.busy = false;
                self.restore.step = before;
                match source {
                    Some(RestoreSource::PreRestore { stamp }) => {
                        self.put_back_refused(&stamp, &error, window, cx);
                    }
                    _ => {
                        self.restore.error = Some(error.backup_message());
                        if self.page_focus.is_focused(window) {
                            self.restore.replace_focus.focus(window, cx);
                        }
                    }
                }
                self.reload(cx);
            }
        }
        cx.notify();
    }

    /// Read the last restore's outcome once per launch.
    pub(super) fn read_status(&mut self, cx: &mut Context<'_, Self>) {
        let operation = self.calls.status(cx);
        cx.spawn(async move |this, cx| {
            if let Ok(status) = operation.await {
                let _ = this.update(cx, |this, cx| this.status_answered(&status, cx));
            }
        })
        .detach();
    }

    /// The outcome of a restore this launch applied, while the group has
    /// nothing of its own to show.
    pub(super) fn status_answered(&mut self, status: &RestoreStatus, cx: &mut Context<'_, Self>) {
        if self.restore.step != Restoring::Idle {
            return;
        }
        let recent = status.result.as_ref().is_some_and(|result| {
            let since = self.made_at.millis_since(result.applied_at_ms);
            (-RESULT_SKEW_MS..=RECENT_RESULT_MS).contains(&since)
        });
        if !recent && status.pending.is_none() {
            return;
        }
        if let Some(step) = outcome(status) {
            self.restore.step = step;
            cx.notify();
        }
    }

    /// A preview that just showed takes focus on its Cancel, the safe
    /// choice, when focus was on the page: the form it replaced is gone.
    pub(super) fn focus_new_preview(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if std::mem::take(&mut self.restore.focus_preview)
            && self.page_focus.contains_focused(window, cx)
        {
            self.restore.cancel_focus.focus(window, cx);
        }
    }

    fn render_open_form(&self, cx: &Context<'_, Self>) -> AnyElement {
        let restore = &self.restore;
        let mut choose = button(
            "desktop-backup-choose",
            "desktop-backup-choose".into(),
            CHOOSE_FILE,
            WebButtonVariant::Outline,
            self.busy,
        );
        if !self.busy {
            choose = choose.track_focus(&restore.choose_focus);
        }
        let choose = on_press_view(choose, cx, |this, window, cx| this.choose(window, cx));
        let chosen = restore.chosen.as_ref().map(|path| {
            let name = path
                .file_name()
                .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
                .into_owned();
            small(name, theme::rambutan_300()).debug_selector(|| "desktop-backup-chosen".into())
        });
        let open_disabled = self.busy || restore.chosen.is_none();
        let open = on_press_view(
            button(
                "desktop-backup-open",
                "desktop-backup-open".into(),
                OPEN_BACKUP,
                WebButtonVariant::Outline,
                open_disabled,
            ),
            cx,
            |this, window, cx| this.open(window, cx),
        );
        let cancel = match &restore.step {
            Restoring::Working { job: Some(id), .. } => {
                let id = id.clone();
                Some(on_press_view(
                    button(
                        "desktop-backup-open-cancel",
                        "desktop-backup-open-cancel".into(),
                        CANCEL,
                        WebButtonVariant::Outline,
                        false,
                    ),
                    cx,
                    move |this, _, cx| this.cancel_job(Slot::Restore, id.clone(), cx),
                ))
            }
            _ => None,
        };
        div()
            .mt_3()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(choose)
                    .children(chosen),
            )
            .child(div().max_w(px(PASSWORD_WIDTH_PX)).min_w_0().child(labelled(
                PASSWORD,
                field(
                    &restore.password,
                    PASSWORD,
                    false,
                    Some(InputContentType::Password),
                ),
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(open)
                    .children(cancel),
            )
            .into_any_element()
    }

    fn render_preview(&self, preview: &RestorePreview, cx: &Context<'_, Self>) -> AnyElement {
        let mut replace = danger(button(
            "desktop-backup-replace",
            "desktop-backup-replace".into(),
            REPLACE,
            WebButtonVariant::Outline,
            self.busy,
        ));
        let mut cancel = button(
            "desktop-backup-cancel",
            "desktop-backup-cancel".into(),
            CANCEL,
            WebButtonVariant::Outline,
            self.busy,
        );
        if !self.busy {
            replace = replace.track_focus(&self.restore.replace_focus);
            cancel = cancel.track_focus(&self.restore.cancel_focus);
        }
        let id = preview.id.clone();
        let replace = on_press_view(replace, cx, move |this, window, cx| {
            this.apply(RestoreSource::Staged { id: id.clone() }, window, cx);
        });
        let cancel = on_press_view(cancel, cx, |this, window, cx| this.discard(window, cx));
        let lines = preview_lines(preview)
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                let selector = format!("desktop-backup-preview-line-{index}");
                div()
                    .id(SharedString::from(selector.clone()))
                    .role(Role::ListItem)
                    .min_w_0()
                    .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_200())
                    .debug_selector(move || selector)
                    .child(text)
            });
        div()
            .id("desktop-backup-preview")
            .debug_selector(|| "desktop-backup-preview".into())
            .mt_3()
            .min_w_0()
            .flex()
            .flex_col()
            // The web's `<ul>`: the lines are its items.
            .child(
                div()
                    .id("desktop-backup-preview-lines")
                    .role(Role::List)
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(lines),
            )
            .child(
                small(REPLACE_WARNING, theme::durian_300())
                    .mt_3()
                    .debug_selector(|| "desktop-backup-replace-warning".into()),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(replace)
                    .child(cancel),
            )
            .into_any_element()
    }

    /// What the restore says in its live region: progress, the wait (from
    /// the press, M5), the outcome or a waiting restore.
    fn render_restore_status(&self) -> Option<AnyElement> {
        match &self.restore.step {
            Restoring::Idle | Restoring::Preview(_) | Restoring::Working { line: None, .. } => None,
            Restoring::Working {
                line: Some(text), ..
            } => Some(
                small(text.clone(), theme::rambutan_300())
                    .mt_2()
                    .debug_selector(|| "desktop-backup-restore-progress".into())
                    .into_any_element(),
            ),
            Restoring::Applying | Restoring::Stopping => Some(
                small(STOPPING, theme::rambutan_300())
                    .mt_2()
                    .debug_selector(|| "desktop-backup-stopping".into())
                    .into_any_element(),
            ),
            Restoring::Finished {
                lines, rolled_back, ..
            } => {
                let color = if *rolled_back {
                    theme::lychee_300()
                } else {
                    theme::pandan_300()
                };
                Some(
                    div()
                        .mt_2()
                        .flex()
                        .flex_col()
                        .debug_selector(|| "desktop-backup-restored".into())
                        .children(lines.iter().map(|text| small(text.clone(), color)))
                        .into_any_element(),
                )
            }
            Restoring::Pending => Some(
                small(RESTORE_PENDING, theme::durian_300())
                    .mt_2()
                    .debug_selector(|| "desktop-backup-pending".into())
                    .into_any_element(),
            ),
        }
    }

    /// **Open the web app** while a browser part waits: under an outcome
    /// that already says so, or with its own line when the overview says
    /// one waits, on any launch.
    fn render_open_web(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let in_outcome = matches!(
            &self.restore.step,
            Restoring::Finished {
                browser_waits: true,
                ..
            }
        );
        if !in_outcome && !self.overview.web_part_waiting {
            return None;
        }
        let waiting = (!in_outcome).then(|| {
            small(WEB_PART_ON_DESKTOP, theme::rambutan_300())
                .mt_2()
                .debug_selector(|| "desktop-backup-web-waiting".into())
        });
        let open = button(
            "desktop-backup-restore-open-web",
            "desktop-backup-restore-open-web".into(),
            OPEN_WEB_APP,
            WebButtonVariant::Outline,
            false,
        )
        .role(Role::Link)
        .text_color(theme::blueberry_300())
        .track_focus(&self.restore.web_app_focus);
        Some(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .children(waiting)
                .child(
                    div()
                        .mt_2()
                        .child(on_press_view(open, cx, |this, _, cx| this.open_web_app(cx))),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_restore(&self, cx: &Context<'_, Self>) -> AnyElement {
        let preview = match &self.restore.step {
            Restoring::Preview(preview) => Some(self.render_preview(preview, cx)),
            _ => None,
        };
        group("desktop-backup-restore-group", false)
            .aria_label(RESTORE_GROUP)
            .child(group_title(RESTORE_GROUP))
            .when(self.restore.picking(), |el| {
                el.child(self.render_open_form(cx))
            })
            .children(preview)
            // One live region for every line the restore says (M4).
            .child(
                div()
                    .id("desktop-backup-restore-status")
                    .role(Role::Status)
                    .children(self.render_restore_status()),
            )
            .children(self.render_open_web(cx))
            .when_some(self.restore.error.clone(), |el, message| {
                let color = super::error_color(&message);
                el.child(
                    line(
                        "desktop-backup-restore-error".into(),
                        message,
                        color,
                        Role::Alert,
                    )
                    .mt_1(),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{preview, ID};
    use super::super::tests::{
        bounds, click, focused, page, press, rendered, requests, server, type_into, value, wait,
        GOOD,
    };
    use super::{outcome, Restoring};
    use crate::server_client::ServerError;
    use bruhs_models::backup::{
        preview_lines, ApplyAccepted, BackupOverview, JobState, PendingRestore, RestoreResult,
        RestoreSource, RestoreStatus, StagedRestore, WebPartInfo, PARKED_STAYED, PASSWORD_SHORT,
        RECORDING_FIRST, RESTORED, RESTORE_ROLLED_BACK, WEB_PART_ON_DESKTOP,
    };
    use bruhs_models::EpochMs;
    use gpui::TestAppContext;
    use std::cell::Cell;
    use std::rc::Rc;

    /// The fixture overview with an opened backup waiting on its preview.
    fn staged() -> BackupOverview {
        BackupOverview {
            staged: Some(StagedRestore { preview: preview() }),
            ..super::super::fixtures::overview()
        }
    }

    #[gpui::test]
    fn the_preview_uses_the_shared_lines(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        *server(&view, cx).overview.borrow_mut() = Some(staged());
        view.update(cx, |view, cx| view.reload(cx));
        cx.run_until_parked();
        let lines = preview_lines(&preview());
        assert!(lines.len() >= 6, "{lines:?}");
        let mut last = None;
        for index in 0..lines.len() {
            let at = bounds(cx, &format!("desktop-backup-preview-line-{index}"));
            assert!(last.is_none_or(|last| at.top() > last), "line {index}");
            last = Some(at.top());
        }
        assert!(!rendered(
            cx,
            &format!("desktop-backup-preview-line-{}", lines.len())
        ));
        let warning = bounds(cx, "desktop-backup-replace-warning");
        let replace = bounds(cx, "desktop-backup-replace");
        let cancel = bounds(cx, "desktop-backup-cancel");
        assert!(last.is_some_and(|last| last < warning.top()));
        assert!(warning.bottom() <= replace.top());
        assert!(replace.right() <= cancel.left(), "Replace, then Cancel");
        // The form steps aside while the preview waits for an answer.
        assert!(!rendered(cx, "desktop-backup-choose"));
        // Cancel drops the opened backup and the form comes back.
        *server(&view, cx).overview.borrow_mut() = Some(super::super::fixtures::overview());
        click(cx, "desktop-backup-cancel");
        assert!(requests(&view, cx).contains(&format!("DELETE /api/backup/restore/{ID}")));
        assert!(rendered(cx, "desktop-backup-choose"));
        assert!(!rendered(cx, "desktop-backup-preview"));
    }

    /// The brief's test: a server answering `ApplyAccepted` makes the app
    /// say it is stopping and start itself again, once.
    #[gpui::test]
    fn replace_asks_then_relaunches(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        let fake = server(&view, cx);
        *fake.overview.borrow_mut() = Some(staged());
        let relaunched = Rc::new(Cell::new(0));
        let count = relaunched.clone();
        view.update(cx, |view, cx| {
            view.relaunch = Rc::new(move |_| count.set(count.get() + 1));
            view.reload(cx);
        });
        cx.run_until_parked();
        // The preview asks: its warning, then Replace my data and Cancel.
        assert!(rendered(cx, "desktop-backup-replace-warning"));
        // No answer yet: STOPPING shows from the press, nothing restarts.
        click(cx, "desktop-backup-replace");
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("POST /api/backup/restore/apply")
        );
        let apply = fake.last_apply.borrow().clone().unwrap();
        assert_eq!(apply.source, RestoreSource::Staged { id: ID.into() });
        assert!(apply.replace && apply.from_desktop);
        assert!(
            rendered(cx, "desktop-backup-stopping"),
            "STOPPING from the press"
        );
        wait(cx, 2_000);
        assert_eq!(relaunched.get(), 0, "nothing restarts before the answer");
        // A second page state where the server answers at once.
        view.update(cx, |view, cx| {
            view.busy = false;
            view.restore.step = Restoring::Preview(Box::new(preview()));
            cx.notify();
        });
        fake.applies.borrow_mut().push_back(Ok(ApplyAccepted {
            service_stopped: true,
        }));
        cx.run_until_parked();
        click(cx, "desktop-backup-replace");
        assert!(rendered(cx, "desktop-backup-stopping"));
        view.read_with(cx, |view, _| {
            assert_eq!(view.restore.step, Restoring::Stopping);
        });
        wait(cx, 1_000);
        assert_eq!(relaunched.get(), 1);
        wait(cx, 5_000);
        assert_eq!(relaunched.get(), 1, "once");
        assert!(
            view.read_with(cx, |view, _| view.busy),
            "every action waits"
        );
    }

    #[gpui::test]
    fn a_refused_replace_goes_back_to_the_preview(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        let fake = server(&view, cx);
        *fake.overview.borrow_mut() = Some(staged());
        fake.applies
            .borrow_mut()
            .push_back(Err(ServerError::Status {
                request: "POST /api/backup/restore/apply".into(),
                code: reqwest::StatusCode::CONFLICT,
                body: serde_json::json!({ "error": RECORDING_FIRST }).to_string(),
            }));
        view.update(cx, |view, cx| {
            view.relaunch = Rc::new(|_| panic!("a refused restore never restarts"));
            view.reload(cx);
        });
        cx.run_until_parked();
        click(cx, "desktop-backup-replace");
        wait(cx, 1_000);
        assert!(rendered(cx, "desktop-backup-preview"));
        assert!(rendered(cx, "desktop-backup-restore-error"));
        view.read_with(cx, |view, _| {
            assert!(!view.busy);
            assert_eq!(view.restore.error.as_deref(), Some(RECORDING_FIRST));
        });
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("GET /api/backup"),
            "a restore that waits shows once the overview is read again"
        );
    }

    #[gpui::test]
    fn a_chosen_file_opens_with_its_password(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        let fake = server(&view, cx);
        assert!(rendered(cx, "desktop-backup-open"));
        // Nothing chosen: Open backup does nothing.
        click(cx, "desktop-backup-open");
        assert!(!requests(&view, cx)
            .iter()
            .any(|r| r == "POST /api/backup/restore"));
        click(cx, "desktop-backup-choose");
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files && !options.directories && !options.multiple);
            Some(vec![
                "/Users/me/bruhs-backup-2026-11-20-0930.bruhsbak".into()
            ])
        });
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-backup-chosen"));
        let password = view.read_with(cx, |view, _| view.restore.password.clone());
        assert!(cx.update(|_, cx| password.read(cx).is_masked()));
        // A password the rule refuses is emptied and refused before any work.
        type_into(cx, &password, "short");
        click(cx, "desktop-backup-open");
        assert!(value(cx, &password).is_empty());
        assert_eq!(
            view.read_with(cx, |view, _| view.restore.error.clone())
                .as_deref(),
            Some(PASSWORD_SHORT)
        );
        // The server checks it, then the opened backup's preview comes.
        fake.jobs.borrow_mut().extend([
            JobState::Running {
                done_bytes: 1,
                total_bytes: 9,
                step: bruhs_models::backup::JobStep::Checking,
            },
            JobState::Opened {
                preview: Box::new(preview()),
            },
        ]);
        type_into(cx, &password, GOOD);
        click(cx, "desktop-backup-open");
        assert!(
            value(cx, &password).is_empty(),
            "read and emptied on submit"
        );
        assert_eq!(
            fake.last_open.borrow().as_deref(),
            Some("/Users/me/bruhs-backup-2026-11-20-0930.bruhsbak")
        );
        assert!(!requests(&view, cx).concat().contains(GOOD));
        assert!(rendered(cx, "desktop-backup-restore-progress"));
        wait(cx, 500);
        assert!(rendered(cx, "desktop-backup-preview"));
        let cancel = view.read_with(cx, |view, _| view.restore.cancel_focus.clone());
        assert_eq!(focused(cx), Some(cancel), "the preview's safe choice");
    }

    #[test]
    fn the_outcome_comes_from_the_status() {
        let result = |warnings: Vec<String>| RestoreResult {
            applied_at_ms: EpochMs::saturating_from_i64(1),
            stamp: "2026-11-20-0930-0a1f".into(),
            warnings,
        };
        let status = |pending, result| RestoreStatus {
            pending,
            result,
            web_part: None,
        };
        assert_eq!(
            outcome(&status(None, Some(result(vec![PARKED_STAYED.into()])))),
            Some(Restoring::Finished {
                lines: vec![RESTORED.into(), PARKED_STAYED.into()],
                rolled_back: false,
                browser_waits: false,
            })
        );
        assert_eq!(
            outcome(&status(
                None,
                Some(result(vec![RESTORE_ROLLED_BACK.into()]))
            )),
            Some(Restoring::Finished {
                lines: vec![RESTORE_ROLLED_BACK.into()],
                rolled_back: true,
                browser_waits: false,
            })
        );
        assert_eq!(
            outcome(&status(Some(PendingRestore::default()), None)),
            Some(Restoring::Pending)
        );
        assert_eq!(outcome(&status(None, None)), None);
        // A browser part waits: the web app restores it (D-I3).
        let waiting = |warnings| RestoreStatus {
            web_part: Some(WebPartInfo::default()),
            ..status(None, Some(result(warnings)))
        };
        assert_eq!(
            outcome(&waiting(Vec::new())),
            Some(Restoring::Finished {
                lines: vec![RESTORED.into(), WEB_PART_ON_DESKTOP.into()],
                rolled_back: false,
                browser_waits: true,
            })
        );
        assert_eq!(
            outcome(&waiting(vec![RESTORE_ROLLED_BACK.into()])),
            Some(Restoring::Finished {
                lines: vec![RESTORE_ROLLED_BACK.into()],
                rolled_back: true,
                browser_waits: false,
            })
        );
    }

    /// A restore applied here from a web-made backup: the outcome says the
    /// browser data waits and **Open the web app** goes there, by keyboard
    /// too (D-I3).
    #[gpui::test]
    fn a_waiting_browser_part_points_at_the_web_app(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        let made_at = view.read_with(cx, |view, _| view.made_at);
        let status = RestoreStatus {
            pending: None,
            result: Some(RestoreResult {
                applied_at_ms: EpochMs::saturating_from_i64(made_at.as_millis() - 5_000),
                stamp: "2026-11-20-0930-0a1f".into(),
                warnings: Vec::new(),
            }),
            web_part: Some(WebPartInfo::default()),
        };
        view.update(cx, |view, cx| view.status_answered(&status, cx));
        cx.run_until_parked();
        let lines = bounds(cx, "desktop-backup-restored");
        let open = bounds(cx, "desktop-backup-restore-open-web");
        assert!(lines.bottom() <= open.top(), "under the outcome's lines");
        view.read_with(cx, |view, _| {
            let Restoring::Finished { lines, .. } = &view.restore.step else {
                panic!("an outcome");
            };
            assert_eq!(lines.last().map(String::as_str), Some(WEB_PART_ON_DESKTOP));
        });
        click(cx, "desktop-backup-restore-open-web");
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("http://127.0.0.1:9/#/settings")
        );
        let handle = view.read_with(cx, |view, _| view.restore.web_app_focus.clone());
        cx.update(|window, cx| handle.focus(window, cx));
        press(cx, "enter");
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("http://127.0.0.1:9/#/settings"),
            "Enter opens it too"
        );
    }

    /// A browser part that waits shows on any launch, not just after this
    /// launch's own restore, and an outcome that says so isn't repeated.
    #[gpui::test]
    fn the_overview_says_a_browser_part_waits(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        assert!(!rendered(cx, "desktop-backup-web-waiting"));
        *server(&view, cx).overview.borrow_mut() = Some(BackupOverview {
            web_part_waiting: true,
            ..super::super::fixtures::overview()
        });
        view.update(cx, |view, cx| view.reload(cx));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.restore.step, Restoring::Idle, "no outcome of its own");
        });
        let line = bounds(cx, "desktop-backup-web-waiting");
        let open = bounds(cx, "desktop-backup-restore-open-web");
        assert!(line.bottom() <= open.top());
        click(cx, "desktop-backup-restore-open-web");
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("http://127.0.0.1:9/#/settings")
        );
        // This launch's outcome already says it: one line, one link.
        let made_at = view.read_with(cx, |view, _| view.made_at);
        let status = RestoreStatus {
            pending: None,
            result: Some(RestoreResult {
                applied_at_ms: EpochMs::saturating_from_i64(made_at.as_millis() - 5_000),
                stamp: "2026-11-20-0930-0a1f".into(),
                warnings: Vec::new(),
            }),
            web_part: Some(WebPartInfo::default()),
        };
        view.update(cx, |view, cx| view.status_answered(&status, cx));
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-backup-restored"));
        assert!(!rendered(cx, "desktop-backup-web-waiting"));
        assert!(rendered(cx, "desktop-backup-restore-open-web"));
    }

    #[gpui::test]
    fn an_outcome_without_browser_data_offers_nothing_more(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        let made_at = view.read_with(cx, |view, _| view.made_at);
        let status = RestoreStatus {
            pending: None,
            result: Some(RestoreResult {
                applied_at_ms: EpochMs::saturating_from_i64(made_at.as_millis() - 5_000),
                stamp: "2026-11-20-0930-0a1f".into(),
                warnings: Vec::new(),
            }),
            web_part: None,
        };
        view.update(cx, |view, cx| view.status_answered(&status, cx));
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-backup-restored"));
        assert!(!rendered(cx, "desktop-backup-restore-open-web"));
    }

    #[gpui::test]
    fn only_this_launchs_restore_shows_its_outcome(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, true);
        let made_at = view.read_with(cx, |view, _| view.made_at);
        let at = |ms: i64| RestoreStatus {
            pending: None,
            result: Some(RestoreResult {
                applied_at_ms: EpochMs::saturating_from_i64(made_at.as_millis() - ms),
                stamp: "2026-11-20-0930-0a1f".into(),
                warnings: Vec::new(),
            }),
            web_part: None,
        };
        view.update(cx, |view, cx| {
            view.status_answered(&at(24 * 60 * 60 * 1000), cx)
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.restore.step.clone()),
            Restoring::Idle
        );
        view.update(cx, |view, cx| view.status_answered(&at(5_000), cx));
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-backup-restored"));
        assert!(rendered(cx, "desktop-backup-choose"), "the form comes back");
    }
}
