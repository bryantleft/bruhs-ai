//! The App source on the desktop (D20; web `reactive_automations/app_poll.rs`
//! and the routine editors): what to watch, the account to poll with, the
//! repository and how often. A poll runs as the owner, so its accounts are
//! the owner's: Global GitHub accounts for the GitHub watches and the
//! connected mailboxes for new email. Every rule and line of copy is
//! `bruhs_models::apps`', so the web editor matches.
//!
//! Also what both editors do with a newer Bruhs' source: its type changes
//! only after the owner confirms, since that replaces what that Bruhs
//! saved, and it is never saved as it is (ruling F3).
use super::*;
use bruhs_models::apps::{
    check_poll_account, new_poll_form, open_apps_from_own_address, poll_account_choices,
    poll_account_label, poll_account_options, poll_form_for, poll_form_kind, poll_form_with_watch,
    watch_label, AppPollForm, AppQuery, AppsListing, PollWatch, APPS_LOADING, CHANGE_NEWER_SOURCE,
    CHANGE_TYPE, MINUTES_LABEL, NO_GITHUB_ACCOUNTS, NO_MAILBOXES, POLL_ACCOUNT_LABEL,
    POLL_ACCOUNT_NEEDED, REPO_HINT, REPO_LABEL, SOURCE_KIND_APP, SOURCE_KIND_OTHER,
    STARTS_WHEN_APP, WATCH_LABEL,
};
use bruhs_models::vault::KEEP;

const WATCHES: [PollWatch; 3] = [
    PollWatch::GithubIssues,
    PollWatch::GithubPulls,
    PollWatch::NewMail,
];
/// Web `bg-durian-500/10` behind the newer-source question.
const CONFIRM_FILL_OPACITY: f32 = 0.1;

fn watch_selector(watch: PollWatch) -> &'static str {
    match watch {
        PollWatch::GithubIssues => "desktop-app-poll-watch-github_issues",
        PollWatch::GithubPulls => "desktop-app-poll-watch-github_pulls",
        PollWatch::NewMail => "desktop-app-poll-watch-new_mail",
    }
}

/// The "Starts when" choices: the App source only on a service that keeps
/// one, or for a routine that already has one (shown off with the reason),
/// and a newer Bruhs' source only for a routine that has one.
pub(super) fn starts_when_options(apps: bool, current: &str) -> Vec<(&'static str, &'static str)> {
    let mut options = vec![
        ("folder", "A file arrives"),
        ("http", "A website changes"),
        ("command", "A command reports a change"),
        ("schedule", "On a repeating schedule"),
        ("calendar", "At set times of day"),
        ("webhook", "A webhook arrives"),
        ("app_event", "A meeting event occurs"),
        ("custom", "A custom event arrives"),
    ];
    if apps || current == "app_poll" {
        options.push(("app_poll", STARTS_WHEN_APP));
    }
    if current == "other" {
        options.push(("other", SOURCE_KIND_OTHER));
    }
    options
}

/// The advanced editor's source types, plus the read-only type of the
/// source it has open, so the choices always show the one it has. The
/// advanced editor never builds an App source (ruling F18).
pub(super) fn source_type_options(current: &str) -> Vec<(&'static str, &'static str)> {
    let mut options = vec![
        ("folder", "Folder"),
        ("http", "Website"),
        ("command", "Command"),
        ("schedule", "Schedule"),
        ("calendar", "Calendar"),
        ("webhook", "Webhook"),
        ("app_event", "App event"),
        ("custom", "Custom event"),
    ];
    match current {
        "app_poll" => options.push(("app_poll", SOURCE_KIND_APP)),
        "other" => options.push(("other", SOURCE_KIND_OTHER)),
        _ => {}
    }
    options
}

/// Whether picking `kind` must ask first: the open source is a newer
/// Bruhs' one and `kind` replaces it. Then the pick waits on Change type.
pub(super) fn ask_before_kind(source: &mut SourceEditor, kind: &'static str) -> bool {
    if source.kind == "other" && kind != "other" {
        source.change_type = Some(kind);
        return true;
    }
    source.change_type = None;
    false
}

/// The routine editor's type change: the type, and the value it starts
/// with.
pub(super) fn automation_kind(
    source: &mut SourceEditor,
    kind: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    source.kind = kind;
    source.value = edit_field(
        if kind == "app_event" {
            "meeting.completed"
        } else {
            ""
        },
        kind == "command",
        window,
        cx,
    );
    if kind == "schedule" {
        source
            .interval
            .update(cx, |input, cx| input.set_value("3600", window, cx));
    }
}

/// The App source the routine editor reviews: refused on an older service
/// (`blocked`), with a malformed form, or with an account the loaded
/// listing no longer offers.
pub(super) fn app_poll_source(
    form: &AppPollForm,
    listing: Option<&AppsListing>,
    blocked: Option<&'static str>,
) -> Result<SourceKind, String> {
    if let Some(reason) = blocked {
        return Err(reason.to_owned());
    }
    let source = poll_form_kind(form)?;
    check_poll_account(form, listing)?;
    Ok(source)
}

/// The App source's fields in a routine's editor.
pub(crate) struct AppPollEditor {
    watch: PollWatch,
    account: String,
    repo: Entity<InputState>,
    minutes: Entity<InputState>,
}

impl AppPollEditor {
    /// The fields `kind` opens with: its own when it is an App source,
    /// else new GitHub issues at the default interval.
    pub(super) fn new(kind: &SourceKind, window: &mut Window, cx: &mut App) -> Self {
        let form = poll_form_for(kind).unwrap_or_else(new_poll_form);
        let repo = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder(REPO_HINT);
            input.set_value(&form.repo, window, cx);
            input
        });
        Self {
            watch: form.watch,
            account: form.account,
            repo,
            minutes: edit_input(&form.minutes, window, cx),
        }
    }

    /// The fields as typed.
    pub(super) fn form(&self, cx: &App) -> AppPollForm {
        AppPollForm {
            watch: self.watch,
            account: self.account.clone(),
            repo: value(&self.repo, cx),
            minutes: value(&self.minutes, cx),
        }
    }

    #[cfg(test)]
    pub(super) fn inputs_for_test(&self) -> (Entity<InputState>, Entity<InputState>) {
        (self.repo.clone(), self.minutes.clone())
    }
}

/// The owner's app accounts and mailboxes, for App sources: the routine
/// editor's account choices and the mailbox a new mail routine names. Read
/// when a routine's editor opens, or once when the snapshot first holds a
/// new mail routine, and never stored.
#[derive(Default)]
pub(super) struct PollApps {
    listing: Option<AppsListing>,
    /// Why the accounts can't be listed, such as an older server.
    notice: Option<String>,
    asked: bool,
    /// Bumped by each load, so an older answer never replaces a newer one.
    generation: u64,
    task: Option<Task<()>>,
    /// Every request, as `METHOD path`; tests never reach a server.
    #[cfg(test)]
    requests: Vec<String>,
}

impl PollApps {
    pub(super) fn listing(&self) -> Option<&AppsListing> {
        self.listing.as_ref()
    }
}

impl ReactiveAutomations {
    /// Read the owner's app accounts and mailboxes again.
    pub(super) fn load_poll_apps(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let poll = &mut self.poll_apps;
        poll.asked = true;
        poll.generation = poll.generation.wrapping_add(1);
        let generation = poll.generation;
        #[cfg(test)]
        poll.requests.push("GET /api/apps".into());
        if cfg!(test) {
            return;
        }
        let operation = spawn_tokio(cx, async move { client.apps_listing().await });
        poll.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The panel may be gone by now.
            let _ = this.update(cx, |this, cx| {
                this.poll_apps_answered(generation, result, cx)
            });
        }));
    }

    /// A new mail routine names its mailbox, so the first snapshot that
    /// holds one reads the listing.
    pub(super) fn note_poll_sources(&mut self, cx: &mut Context<'_, Self>) {
        let wants = !self.poll_apps.asked
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.sources.iter().any(|source| {
                    matches!(
                        source.kind,
                        SourceKind::AppPoll {
                            query: AppQuery::NewMail,
                            ..
                        }
                    )
                })
            });
        if wants {
            self.load_poll_apps(cx);
        }
    }

    fn poll_apps_answered(
        &mut self,
        generation: u64,
        result: Result<AppsListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.poll_apps.generation != generation {
            return;
        }
        let (listing, notice) = match result {
            Ok(listing) => (Some(listing), None),
            Err(error) if error.is_not_found() => {
                (None, Some(ServiceNotice::ServingOld.message().to_owned()))
            }
            Err(ServerError::Status { code, .. }) if code == reqwest::StatusCode::FORBIDDEN => (
                None,
                Some(open_apps_from_own_address(self.client.base_url())),
            ),
            Err(error) => (None, Some(error.vault_message())),
        };
        self.poll_apps.task = None;
        self.poll_apps.listing = listing;
        self.poll_apps.notice = notice;
        cx.notify();
    }

    /// Why an App source can't be saved here: an older service, or one
    /// whose version isn't known yet.
    pub(super) fn app_source_block(&self) -> Option<&'static str> {
        (!self.apps_ready()).then(|| {
            self.apps_reason()
                .unwrap_or(ServiceNotice::ServingOld.message())
        })
    }

    /// The mailbox a new mail routine names, when the listing has it.
    pub(super) fn poll_label(&self, kind: &SourceKind) -> Option<String> {
        self.poll_apps
            .listing()
            .and_then(|listing| poll_account_label(kind, listing))
    }

    fn choose_poll_watch(&mut self, watch: PollWatch, cx: &mut Context<'_, Self>) {
        let listing = self.poll_apps.listing.as_ref();
        if let Some(Editor::Automation(editor)) = self.editor.as_mut() {
            let poll = editor.poll_mut();
            let next = poll_form_with_watch(&poll.form(cx), watch, listing);
            poll.watch = next.watch;
            poll.account = next.account;
        }
        cx.notify();
    }

    fn choose_poll_account(&mut self, account: String, cx: &mut Context<'_, Self>) {
        if let Some(Editor::Automation(editor)) = self.editor.as_mut() {
            editor.poll_mut().account = account;
        }
        cx.notify();
    }

    /// Change type: the newer Bruhs' source takes the type the owner
    /// picked, which replaces it once saved.
    fn confirm_type_change(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        match self.editor.as_mut() {
            Some(Editor::Source(editor)) => {
                if let Some(kind) = editor.change_type.take() {
                    editor.kind = kind;
                }
            }
            Some(Editor::Automation(editor)) => {
                let source = editor.source_mut();
                if let Some(kind) = source.change_type.take() {
                    automation_kind(source, kind, window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// The question before a newer Bruhs' source changes type, while it
    /// waits on Change type or Keep.
    pub(super) fn render_newer_source_confirm(
        &self,
        source: &SourceEditor,
        cx: &Context<'_, Self>,
    ) -> Option<Div> {
        source.change_type?;
        Some(
            div()
                .debug_selector(|| "desktop-newer-source-confirm".to_string())
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .p_3()
                .rounded(px(8.0))
                .bg(theme::durian_500().opacity(CONFIRM_FILL_OPACITY))
                .child(
                    div()
                        .id("desktop-newer-source-question")
                        .role(Role::Alert)
                        .min_w_0()
                        .text_size(px(13.0))
                        .text_color(theme::durian_200())
                        .child(CHANGE_NEWER_SOURCE),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            control("desktop-newer-source-change", CHANGE_TYPE, self.busy)
                                .debug_selector(|| "desktop-newer-source-change".to_string())
                                .on_activate(cx, |this, window, cx| {
                                    this.confirm_type_change(window, cx)
                                }),
                        )
                        .child(
                            ghost_control("desktop-newer-source-keep", KEEP, self.busy)
                                .debug_selector(|| "desktop-newer-source-keep".to_string())
                                .on_activate(cx, |this, _, cx| {
                                    if let Some(source) = this.source_editor_mut() {
                                        source.change_type = None;
                                    }
                                    cx.notify();
                                }),
                        ),
                ),
        )
    }

    /// The App source's fields: Watch, Account, Repository (GitHub only)
    /// and Check every, off with the reason on an older service.
    pub(super) fn render_app_poll(&self, poll: &AppPollEditor, cx: &Context<'_, Self>) -> Div {
        let disabled = self.app_source_block();
        let off = disabled.is_some() || self.busy;
        let form = poll.form(cx);
        let listing = self.poll_apps.listing();
        let watches = div()
            .debug_selector(|| "desktop-app-poll-watch".to_string())
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(muted(WATCH_LABEL))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(WATCHES.into_iter().map(|watch| {
                        let selector = watch_selector(watch);
                        let chosen = form.watch == watch;
                        control(selector, watch_label(watch), off)
                            .debug_selector(move || selector.to_string())
                            .aria_selected(chosen)
                            .when(chosen, |button| {
                                button.bg(theme::blueberry_500().opacity(0.15))
                            })
                            .when(!off, |button| {
                                button.on_activate(cx, move |this, _, cx| {
                                    this.choose_poll_watch(watch, cx)
                                })
                            })
                    })),
            );
        // Nothing to choose, or no list: say why.
        let status = self.poll_apps.notice.clone().or_else(|| match listing {
            None => Some(APPS_LOADING.to_owned()),
            Some(listing) if poll_account_choices(form.watch, listing).is_empty() => Some(
                if form.watch == PollWatch::NewMail {
                    NO_MAILBOXES
                } else {
                    NO_GITHUB_ACCOUNTS
                }
                .to_owned(),
            ),
            Some(_) if form.account.is_empty() => Some(POLL_ACCOUNT_NEEDED.to_owned()),
            Some(_) => None,
        });
        // "Choose an account." is the status line here, not a choice.
        let choices = listing
            .map(|listing| poll_account_options(&form, listing))
            .unwrap_or_default()
            .into_iter()
            .filter(|(id, _)| !id.is_empty());
        let accounts = div()
            .debug_selector(|| "desktop-app-poll-account".to_string())
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(muted(POLL_ACCOUNT_LABEL))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(choices.map(|(id, label)| {
                        let chosen = form.account == id;
                        let selector = format!("desktop-app-poll-account-{id}");
                        control(SharedString::from(selector.clone()), &label, off)
                            .debug_selector(move || selector)
                            .aria_selected(chosen)
                            .when(chosen, |button| {
                                button.bg(theme::blueberry_500().opacity(0.15))
                            })
                            .when(!off, |button| {
                                button.on_activate(cx, move |this, _, cx| {
                                    this.choose_poll_account(id.clone(), cx)
                                })
                            })
                    })),
            )
            .when_some(status, |el, status| {
                el.child(
                    div()
                        .id("desktop-app-poll-status")
                        .debug_selector(|| "desktop-app-poll-status".to_string())
                        .role(Role::Status)
                        .child(muted(status)),
                )
            });
        div()
            .debug_selector(|| "desktop-app-poll".to_string())
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(disabled, |el, reason| {
                el.child(
                    div()
                        .id("desktop-app-poll-off")
                        .debug_selector(|| "desktop-app-poll-off".to_string())
                        .role(Role::Status)
                        .min_w_0()
                        .text_size(px(12.0))
                        .text_color(theme::durian_300())
                        .child(reason),
                )
            })
            .child(watches)
            .child(accounts)
            .when(form.watch != PollWatch::NewMail, |el| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-app-poll-repo".to_string())
                        .child(field(REPO_LABEL, &poll.repo, false, off)),
                )
            })
            .child(
                div()
                    .debug_selector(|| "desktop-app-poll-minutes".to_string())
                    .child(field(MINUTES_LABEL, &poll.minutes, false, off)),
            )
    }

    /// Answer the listing request as the server would.
    #[cfg(test)]
    pub(super) fn answer_poll_apps_for_test(
        &mut self,
        result: Result<AppsListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        let generation = self.poll_apps.generation;
        self.poll_apps_answered(generation, result, cx);
    }

    #[cfg(test)]
    pub(super) fn poll_apps_requests_for_test(&self) -> Vec<String> {
        self.poll_apps.requests.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{ask_before_kind, starts_when_options, Editor, ReactiveAutomations};
    use crate::theme;
    use bruhs_models::apps::{
        AppAccount, AppQuery, AppsListing, PollWatch, APP_POLL_ACCOUNT_GONE, FROM_NEWER_SOURCE,
        REPO_SHAPE, STARTS_WHEN_APP,
    };
    use bruhs_models::crew::{APPS_PROTOCOL, VAULT_PROTOCOL};
    use bruhs_models::reactive::{
        AutomationSetup, BusyPolicy, EventSource, ReactiveAgent, ReactiveSnapshot, SourceKind,
        Subscription,
    };
    use bruhs_models::secrets::SavedCredentialState;
    use bruhs_models::vault::CredentialScope;
    use gpui::{AppContext as _, Entity, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn listing() -> AppsListing {
        AppsListing {
            accounts: vec![AppAccount {
                id: "v1-g".into(),
                app: "github".into(),
                name: "GitHub octo".into(),
                account: "octo".into(),
                scope: CredentialScope::Global,
                state: SavedCredentialState::Saved,
                attention: None,
            }],
            protocol: APPS_PROTOCOL,
            ..AppsListing::default()
        }
    }

    fn agent() -> ReactiveAgent {
        ReactiveAgent {
            id: "a".into(),
            name: "Ada".into(),
            instructions: "Triage.".into(),
            memory: String::new(),
            model: "claude-sonnet-4-6".into(),
            project_path: "/tmp".into(),
            enabled: true,
        }
    }

    /// A complete routine around `source`, as a saved one would be.
    fn setup_with(source: EventSource) -> AutomationSetup {
        AutomationSetup {
            agent: agent(),
            subscription: Subscription {
                id: "sub".into(),
                source_id: source.id.clone(),
                agent_id: "a".into(),
                instructions: "Look.".into(),
                enabled: true,
                auto_run: true,
                debounce_ms: 0,
                throttle_ms: 0,
                on_busy: BusyPolicy::Queue,
            },
            source,
        }
    }

    fn open(
        cx: &mut gpui::TestAppContext,
        protocol: u32,
    ) -> (Entity<ReactiveAutomations>, &mut VisualTestContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update(cx, |panel, cx| {
            panel.loaded = true;
            panel.set_snapshot(
                ReactiveSnapshot {
                    agents: vec![agent()],
                    protocol,
                    ..ReactiveSnapshot::default()
                },
                cx,
            );
        });
        (panel, cx)
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }

    fn type_poll(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
        repo: &str,
        minutes: &str,
    ) {
        panel.update_in(cx, |panel, window, cx| {
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("automation editor");
            };
            let (repo_input, minutes_input) = editor.poll_mut().inputs_for_test();
            repo_input.update(cx, |input, cx| input.set_value(repo, window, cx));
            minutes_input.update(cx, |input, cx| input.set_value(minutes, window, cx));
        });
    }

    /// The editor's form as it stands, and the source it reviewed.
    fn reviewed(
        panel: &Entity<ReactiveAutomations>,
        cx: &mut VisualTestContext,
    ) -> (
        bruhs_models::apps::AppPollForm,
        Option<AutomationSetup>,
        Option<String>,
    ) {
        panel.update(cx, |panel, cx| {
            let error = panel.error.clone();
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("automation editor");
            };
            (editor.poll_mut().form(cx), editor.review().cloned(), error)
        })
    }

    /// D20: New GitHub issues, an account, `o/r` and 5 minutes build one
    /// App source; opening it again shows the same fields; a malformed
    /// repository is refused before anything is sent.
    #[gpui::test]
    fn the_app_source_round_trips_its_form(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, APPS_PROTOCOL);
        // A routine on Ada's page: her agent goes as saved.
        panel.update_in(cx, |panel, window, cx| {
            panel.open_routine(None, "a", window, cx)
        });
        assert_eq!(
            panel.read_with(cx, |panel, _| panel.poll_apps_requests_for_test()),
            vec!["GET /api/apps".to_string()]
        );
        panel.update(cx, |panel, cx| {
            panel.answer_poll_apps_for_test(Ok(listing()), cx)
        });
        let app_kind: &'static str = Box::leak(format!("reactive-{STARTS_WHEN_APP}").into());
        click(cx, app_kind);
        for selector in [
            "desktop-app-poll-watch",
            "desktop-app-poll-account",
            "desktop-app-poll-repo",
            "desktop-app-poll-minutes",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector}");
        }
        assert!(cx.debug_bounds("desktop-app-poll-off").is_none());
        click(cx, "desktop-app-poll-watch-github_pulls");
        click(cx, "desktop-app-poll-watch-github_issues");
        click(cx, "desktop-app-poll-account-v1-g");
        type_poll(&panel, cx, "o/r", "5");
        click(cx, "reactive-Review automation");
        let (form, review, error) = reviewed(&panel, cx);
        assert_eq!(error, None);
        assert_eq!(
            (form.watch, form.account.as_str()),
            (PollWatch::GithubIssues, "v1-g")
        );
        let app = SourceKind::AppPoll {
            app: "github".into(),
            account: "v1-g".into(),
            query: AppQuery::GithubIssues { repo: "o/r".into() },
            poll_ms: 300_000,
        };
        let setup = review.expect("reviewed");
        assert_eq!(setup.source.kind, app);
        // Opening it again shows the same fields.
        panel.update_in(cx, |panel, window, cx| {
            panel.open_routine(Some(setup.clone()), "a", window, cx)
        });
        let (form, review, _) = reviewed(&panel, cx);
        assert_eq!(review, None);
        assert_eq!(
            (
                form.watch,
                form.account.as_str(),
                form.repo.as_str(),
                form.minutes.as_str()
            ),
            (PollWatch::GithubIssues, "v1-g", "o/r", "5")
        );
        type_poll(&panel, cx, "o", "5");
        click(cx, "reactive-Review automation");
        let (_, review, error) = reviewed(&panel, cx);
        assert_eq!(review, None);
        assert_eq!(error.as_deref(), Some(REPO_SHAPE));
        // A gone account is refused too, and nothing was ever sent.
        type_poll(&panel, cx, "o/r", "5");
        panel.update(cx, |panel, cx| {
            panel.answer_poll_apps_for_test(Ok(AppsListing::default()), cx)
        });
        click(cx, "reactive-Review automation");
        let (_, review, error) = reviewed(&panel, cx);
        assert_eq!(review, None);
        assert_eq!(error.as_deref(), Some(APP_POLL_ACCOUNT_GONE));
        panel.read_with(cx, |panel, _| assert!(panel.sent_for_test().is_empty()));
    }

    /// D10: an older service keeps an existing App routine on screen, off
    /// with its reason, and refuses to save it.
    #[gpui::test]
    fn an_older_service_shows_an_app_routine_off(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, VAULT_PROTOCOL);
        let setup = setup_with(EventSource {
            id: "s".into(),
            name: "Issues".into(),
            enabled: true,
            kind: SourceKind::AppPoll {
                app: "github".into(),
                account: "v1-g".into(),
                query: AppQuery::GithubIssues { repo: "o/r".into() },
                poll_ms: 300_000,
            },
        });
        panel.update_in(cx, |panel, window, cx| {
            panel.open_automation(Some(setup), window, cx)
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-app-poll-off").is_some());
        click(cx, "reactive-Review automation");
        let (_, review, error) = reviewed(&panel, cx);
        assert_eq!(review, None);
        assert_eq!(
            error.as_deref(),
            bruhs_models::crew::apps_reason(Some(VAULT_PROTOCOL))
        );
    }

    /// Ruling A Task 5: a newer Bruhs' source changes type only after
    /// Change type, and Keep leaves it as it is; its row has no Pause or
    /// Enable, and it is never saved.
    #[gpui::test]
    fn a_newer_source_asks_before_its_type_changes(cx: &mut gpui::TestAppContext) {
        let (panel, cx) = open(cx, APPS_PROTOCOL);
        let other = EventSource {
            id: "s".into(),
            name: "Later".into(),
            enabled: true,
            kind: SourceKind::Other,
        };
        panel.update_in(cx, |panel, window, cx| {
            let mut snapshot = ReactiveSnapshot {
                protocol: APPS_PROTOCOL,
                ..ReactiveSnapshot::default()
            };
            snapshot.sources.push(other.clone());
            panel.set_snapshot(snapshot, cx);
            panel.advanced_open = true;
            panel.open_source(Some(other.clone()), window, cx);
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(cx.debug_bounds("reactive-Other").is_some());
        assert!(cx.debug_bounds("reactive-Pause").is_none());
        assert!(cx.debug_bounds("reactive-Delete source").is_some());
        click(cx, "reactive-Folder");
        let kind = |panel: &Entity<ReactiveAutomations>, cx: &mut VisualTestContext| {
            panel.read_with(cx, |panel, _| match &panel.editor {
                Some(Editor::Source(editor)) => (editor.kind, editor.change_type),
                _ => panic!("source editor"),
            })
        };
        assert_eq!(kind(&panel, cx), ("other", Some("folder")));
        assert!(cx.debug_bounds("desktop-newer-source-confirm").is_some());
        click(cx, "desktop-newer-source-keep");
        assert_eq!(kind(&panel, cx), ("other", None));
        assert!(cx.debug_bounds("desktop-newer-source-confirm").is_none());
        click(cx, "reactive-Save source");
        panel.read_with(cx, |panel, _| assert!(panel.sent_for_test().is_empty()));
        click(cx, "reactive-Folder");
        click(cx, "desktop-newer-source-change");
        assert_eq!(kind(&panel, cx), ("folder", None));
        // The routine editor asks too.
        panel.update_in(cx, |panel, window, cx| {
            panel.open_automation(Some(setup_with(other.clone())), window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("automation editor");
            };
            assert!(ask_before_kind(editor.source_mut(), "folder"));
            assert_eq!(editor.source_mut().kind, "other");
            assert!(!ask_before_kind(editor.source_mut(), "other"));
        });
        click(cx, "reactive-Review automation");
        let error = panel.read_with(cx, |panel, _| panel.error.clone());
        assert_eq!(error.as_deref(), Some(FROM_NEWER_SOURCE));
    }

    /// D10: the App source is offered only where the service keeps one; a
    /// routine that has one still shows it, as does a newer Bruhs' source.
    #[test]
    fn starts_when_offers_the_app_source_only_when_the_service_keeps_one() {
        let values = |apps, current| {
            starts_when_options(apps, current)
                .into_iter()
                .map(|(value, _)| value)
                .collect::<Vec<_>>()
        };
        assert!(!values(false, "folder").contains(&"app_poll"));
        assert!(values(true, "folder").contains(&"app_poll"));
        assert!(values(false, "app_poll").contains(&"app_poll"));
        assert!(!values(true, "folder").contains(&"other"));
        assert_eq!(values(true, "other").last(), Some(&"other"));
        assert_eq!(
            super::source_type_options("other").last(),
            Some(&("other", "Other"))
        );
        assert_eq!(super::source_type_options("folder").len(), 8);
    }
}
