//! Settings, Apps (D21): the connected mailboxes a bruh may send from, then
//! the fourteen apps in catalog order, each with its accounts and a Connect
//! form. Shared accounts and mailboxes carry one switch per bruh. It
//! matches the web's `components/apps_settings.rs`: the same fields in the
//! same order, the same copy, states and disabled reasons, and Disconnect
//! asks first.
//!
//! The server owns every token: the page lists and changes accounts
//! through `/api/apps`, and a switch sends `SetAppGrant` at once. Tokens
//! are write-only. Their fields are masked, never filled, and emptied on
//! submit, on cancel and when the form goes away, Settings closing
//! included. Every rule and line of copy is `bruhs_models::apps`' and
//! `bruhs_models::crew`'s.

mod form;

use super::vault_settings::{button, is_forbidden, panel, small, CredentialsChanged};
use super::*;
use crate::app_state::spawn_tokio;
use crate::server_client::ServerError;
use bruhs_models::apps::{
    app_scope_line, apps_wait_reason, connected_as, disconnect_confirm, open_apps_from_own_address,
    AppAccount, AppConnector, AppsListing, APPS_LOADING, CONNECT, CONNECTORS, DISCONNECT,
    GRANTS_LABEL, MAIL_GROUP, MAIL_HINT, NO_BRUHS_FOR_APPS, NO_MAILBOXES, REPLACE_TOKEN, TEST,
};
use bruhs_models::crew::{
    apps_ready, apps_reason, grant_switch_on, set_listing_grant, AppGrant, ServiceNotice,
    APPS_NEED_ASK,
};
use bruhs_models::secrets::{review_text, SavedCredentialState};
use bruhs_models::vault::{
    CredentialScope, VaultBusy, VaultNotice, KEEP, LOAD_RETRY, REFRESH, STATE_NEEDS_ATTENTION,
    STATE_SAVED,
};
use bruhs_ui::{press::Interactivity, switch::switch};
use form::AppFormState;
use std::collections::HashMap;

/// Web `rounded-grape` and the Mail group's `bg-longan-950/40` with a
/// `longan-700/50` ring.
const GROUP_RADIUS_PX: f32 = 10.0;
const GROUP_FILL_OPACITY: f32 = 0.4;
const GROUP_RING_OPACITY: f32 = 0.5;

/// A switch's id (`crate::views::kept_selector`).
fn grant_selector(target: &str, agent_id: &str) -> &'static str {
    crate::views::kept_selector(format!("desktop-apps-grant-{target}-{agent_id}"))
}

/// The id a grant names: an app account's entry id or a mailbox's id.
fn grant_target(grant: &AppGrant) -> &str {
    match grant {
        AppGrant::Token { entry_id } => entry_id,
        AppGrant::Mailbox { mailbox_id } => mailbox_id,
    }
}

#[derive(Clone, PartialEq)]
enum Load {
    /// Not asked yet: the first draw asks.
    Idle,
    Loading,
    Ready,
    Failed(String),
    /// An older server: no `/api/apps`.
    ServingOld,
    /// A client on another address: every Apps route is the owner's (D23).
    OwnerOnly,
}

/// Which form is open: one app's Connect, or one account's Replace token.
/// One at a time, as on the Vault page.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Open {
    Connect(&'static str),
    Replace(String),
}

/// The line above the apps after a change.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Notice {
    /// It landed: what happened, in pandan.
    Done(String),
    /// It got no answer, and the listing reloads.
    MayHaveChanged,
}

/// One account row's buttons that focus returns to.
pub(super) struct RowFocus {
    pub(super) replace: FocusHandle,
    pub(super) disconnect: FocusHandle,
    pub(super) keep: FocusHandle,
}

/// A grant flip on its way: which switch, and where it goes.
struct Pending {
    key: String,
    on: bool,
}

pub(super) struct AppsSettings {
    client: ServerClient,
    listing: AppsListing,
    load: Load,
    busy: VaultBusy,
    notice: Option<Notice>,
    pub(super) form: Option<AppFormState>,
    /// The account whose Disconnect is asking.
    asking: Option<String>,
    /// What an account's last Test said: whether it passed, and its message.
    tested: HashMap<String, (bool, String)>,
    /// An account's failed Test or Disconnect.
    row_errors: HashMap<String, String>,
    /// A refused grant, by the account or mailbox it was for.
    grant_errors: HashMap<String, String>,
    pending: Option<Pending>,
    /// Each app's Connect, by connector id.
    pub(super) connect_focus: HashMap<&'static str, FocusHandle>,
    pub(super) rows_focus: HashMap<String, RowFocus>,
    /// Each grant switch, by `{target}:{agent}`.
    switch_focus: HashMap<String, FocusHandle>,
    /// Focus anywhere on the page, so an answer knows it may move it.
    page_focus: FocusHandle,
    next_serial: u64,
    task: Option<Task<()>>,
    /// A quiet listing read for the other section's change (BC M4).
    reread_task: Option<Task<()>>,
    /// Every request, as `METHOD path`; tests never reach a server.
    #[cfg(test)]
    apps_requests: Vec<String>,
    /// The last grant a switch would send, held for a test.
    #[cfg(test)]
    last_grant: Option<(String, AppGrant, bool)>,
}

impl AppsSettings {
    pub(super) fn new(
        client: ServerClient,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        Self {
            client,
            listing: AppsListing::default(),
            load: Load::Idle,
            busy: VaultBusy::Idle,
            notice: None,
            form: None,
            asking: None,
            tested: HashMap::new(),
            row_errors: HashMap::new(),
            grant_errors: HashMap::new(),
            pending: None,
            connect_focus: CONNECTORS
                .iter()
                .map(|app| (app.id, cx.focus_handle().tab_stop(true)))
                .collect(),
            rows_focus: HashMap::new(),
            switch_focus: HashMap::new(),
            page_focus: cx.focus_handle(),
            next_serial: 0,
            task: None,
            reread_task: None,
            #[cfg(test)]
            apps_requests: Vec::new(),
            #[cfg(test)]
            last_grant: None,
        }
    }

    /// A form or a Disconnect question is open.
    pub(super) fn has_editor(&self) -> bool {
        self.form.is_some() || self.asking.is_some()
    }

    /// Close the form, emptying its tokens, and any Disconnect question.
    pub(super) fn clear_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.close_form(window, cx);
        self.asking = None;
        cx.notify();
    }

    fn record(&mut self, request: String) {
        #[cfg(test)]
        self.apps_requests.push(request);
        #[cfg(not(test))]
        drop(request);
    }

    /// A press that starts a wait disables its own control, whose focus
    /// would then point at nothing and leave every key dead. Focus on the
    /// page moves to the page itself, and a text field keeps it.
    fn hold_focus_on_page(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.page_focus.contains_focused(window, cx) && !window.has_focused_input(cx) {
            self.page_focus.focus(window, cx);
        }
    }

    /// Ask the server for the listing again. Closes any form, as the web
    /// does, and waits while a change runs.
    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        self.clear_editor(window, cx);
        self.notice = None;
        self.tested.clear();
        self.row_errors.clear();
        self.grant_errors.clear();
        self.load = Load::Loading;
        self.busy = VaultBusy::Changing;
        self.hold_focus_on_page(window, cx);
        self.record("GET /api/apps".into());
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.apps_listing().await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The page may be gone by now.
            let _ = this.update(cx, |this, cx| this.listing_answered(result, cx));
        }));
    }

    fn listing_answered(
        &mut self,
        result: Result<AppsListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        let (listing, load) = match result {
            Ok(listing) => (listing, Load::Ready),
            Err(error) if error.is_not_found() => (AppsListing::default(), Load::ServingOld),
            Err(error) if is_forbidden(&error) => (AppsListing::default(), Load::OwnerOnly),
            Err(error) => (AppsListing::default(), Load::Failed(error.vault_message())),
        };
        self.listing = listing;
        self.load = load;
        self.busy = VaultBusy::Idle;
        cx.notify();
    }

    /// The listing's state line, if it has one (web `#apps-status`), and so
    /// why nothing can change yet.
    pub(super) fn status_line(&self) -> Option<String> {
        match &self.load {
            Load::Idle | Load::Ready => None,
            Load::Loading => Some(APPS_LOADING.into()),
            Load::Failed(message) => Some(format!("{message} {LOAD_RETRY}")),
            Load::ServingOld => Some(ServiceNotice::ServingOld.message().into()),
            Load::OwnerOnly => Some(open_apps_from_own_address(self.client.base_url())),
        }
    }

    fn wait(&self) -> Option<&'static str> {
        apps_wait_reason(self.busy, self.load == Load::Loading)
    }

    /// Connect waits for a running change and for a listing it can add to.
    fn blocked(&self) -> bool {
        self.wait().is_some() || self.load != Load::Ready
    }

    /// Why the grant switches are off, if they are: a service whose
    /// protocol predates app grants (ruling F19).
    pub(super) fn grants_off(&self) -> Option<&'static str> {
        let protocol = self.listing.protocol;
        (!apps_ready(protocol))
            .then(|| apps_reason(Some(protocol)).unwrap_or(ServiceNotice::ServingOld.message()))
    }

    /// A change that got no answer may have landed: the listing reloads,
    /// which closes any form, and the page says so.
    fn unanswered(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.refresh(window, cx);
        self.notice = Some(Notice::MayHaveChanged);
        cx.emit(CredentialsChanged);
    }

    /// The Vault section deleted an app token (final review BC M4): read
    /// the listing again. A loaded page keeps its open form and rows; one
    /// that isn't loaded reloads in full, and one not shown yet waits to be.
    /// Waits out a running change.
    pub(super) fn reread(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        match self.load {
            // Not shown yet: it reads the listing when it shows.
            Load::Idle => return,
            Load::Ready => {}
            _ => {
                self.refresh(window, cx);
                return;
            }
        }
        if self.busy != VaultBusy::Idle {
            return;
        }
        self.record("GET /api/apps".into());
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.apps_listing().await });
        self.reread_task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if let (Ok(listing), VaultBusy::Idle) = (result, this.busy) {
                    this.listing = listing;
                    cx.notify();
                }
            });
        }));
    }

    fn row_focus(&mut self, id: &str, cx: &mut Context<'_, Self>) -> &RowFocus {
        self.rows_focus
            .entry(id.to_owned())
            .or_insert_with(|| RowFocus {
                replace: cx.focus_handle().tab_stop(true),
                disconnect: cx.focus_handle().tab_stop(true),
                keep: cx.focus_handle().tab_stop(true),
            })
    }

    fn switch_key(grant: &AppGrant, agent_id: &str) -> String {
        format!("{}:{agent_id}", grant_target(grant))
    }

    /// The question Disconnect asks, while it asks.
    pub(super) fn confirm_line(&self) -> Option<String> {
        let id = self.asking.as_ref()?;
        self.listing
            .accounts
            .iter()
            .find(|account| &account.id == id)
            .map(|account| disconnect_confirm(&review_text(&account.name)))
    }

    // ── Accounts ────────────────────────────────────────────────────────

    fn ask_disconnect(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.wait().is_some() {
            return;
        }
        self.row_errors.remove(&id);
        let keep = self.row_focus(&id, cx).keep.clone();
        self.asking = Some(id);
        keep.focus(window, cx);
        cx.notify();
    }

    fn keep(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(id) = self.asking.take() else {
            return;
        };
        let disconnect = self.row_focus(&id, cx).disconnect.clone();
        disconnect.focus(window, cx);
        cx.notify();
    }

    fn disconnect(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        let Some(id) = self.asking.clone() else {
            return;
        };
        self.busy = VaultBusy::Changing;
        self.notice = None;
        self.row_errors.remove(&id);
        self.hold_focus_on_page(window, cx);
        self.record(format!(
            "DELETE {}",
            crate::server_client::vault::vault_entry_path(&id)
        ));
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let target = id.clone();
        let operation = spawn_tokio(cx, async move { client.delete_vault_entry(&target).await });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.disconnected(id, result, window, cx);
            });
        }));
    }

    /// The server answered a Disconnect. Gone either way on a 404.
    fn disconnected(
        &mut self,
        id: String,
        result: Result<(), ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        match result {
            Ok(()) => self.forget(&id, window, cx),
            Err(error) if error.is_not_found() => self.forget(&id, window, cx),
            Err(error) if error.vault_may_have_landed() => self.unanswered(window, cx),
            Err(error) => {
                self.row_errors.insert(id, error.vault_message());
            }
        }
        cx.notify();
    }

    /// An account is gone: its row, its form and its question go, and the
    /// server took it out of every bruh's grants with it. Focus goes to the
    /// app's Connect if it was on the page; an answer that lands after
    /// Settings closed leaves focus where it is.
    fn forget(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let held = self.page_focus.contains_focused(window, cx);
        let app = self
            .listing
            .accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| account.app.clone());
        if matches!(&self.form, Some(form) if form.open == Open::Replace(id.to_owned())) {
            self.close_form(window, cx);
        }
        if self.asking.as_deref() == Some(id) {
            self.asking = None;
        }
        self.listing.accounts.retain(|account| account.id != id);
        for bruh in &mut self.listing.bruhs {
            bruh.apps.retain(|grant| grant != id);
        }
        self.tested.remove(id);
        self.row_errors.remove(id);
        self.grant_errors.remove(id);
        self.rows_focus.remove(id);
        cx.emit(CredentialsChanged);
        let connect = app.and_then(|app| self.connect_focus.get(app.as_str()).cloned());
        if let (true, Some(connect)) = (held, connect) {
            connect.focus(window, cx);
        }
    }

    fn test(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        self.busy = VaultBusy::Checking;
        self.notice = None;
        self.tested.remove(&id);
        self.row_errors.remove(&id);
        self.hold_focus_on_page(window, cx);
        self.record(format!(
            "POST {}",
            crate::server_client::apps::app_test_path(&id)
        ));
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let target = id.clone();
        let operation = spawn_tokio(cx, async move { client.test_app_account(&target).await });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.test_answered(id, result, window, cx);
            });
        }));
    }

    /// The server answered a Test. A refused token is a 200 with `ok:
    /// false`, so the row reads `ok`.
    fn test_answered(
        &mut self,
        id: String,
        result: Result<bruhs_models::apps::AppTestResult, ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        match result {
            Ok(result) => {
                if result.ok {
                    if let Some(account) = self
                        .listing
                        .accounts
                        .iter_mut()
                        .find(|account| account.id == id)
                    {
                        account.attention = None;
                    }
                }
                self.tested.insert(id, (result.ok, result.message));
            }
            Err(error) if error.is_not_found() => self.forget(&id, window, cx),
            Err(error) => {
                self.row_errors.insert(id, error.vault_message());
            }
        }
        cx.notify();
    }

    // ── Grants ──────────────────────────────────────────────────────────

    /// Flip one bruh's switch for `grant`: `SetAppGrant` goes at once, then
    /// the listing reloads; a refusal snaps the switch back and says why.
    fn flip(
        &mut self,
        agent_id: String,
        grant: AppGrant,
        on: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy != VaultBusy::Idle || self.grants_off().is_some() {
            return;
        }
        let key = Self::switch_key(&grant, &agent_id);
        self.busy = VaultBusy::Changing;
        self.notice = None;
        self.grant_errors.remove(grant_target(&grant));
        self.pending = Some(Pending {
            key: key.clone(),
            on,
        });
        self.hold_focus_on_page(window, cx);
        self.record(format!(
            "POST {}",
            crate::server_client::apps::COMMANDS_PATH
        ));
        cx.notify();
        #[cfg(test)]
        {
            drop(key);
            self.last_grant = Some((agent_id, grant, on));
        }
        #[cfg(not(test))]
        self.send_flip(key, agent_id, grant, on, window, cx);
    }

    /// Send a flip, then read the listing again once it went through; the
    /// answer comes back to `flipped`.
    #[cfg(not(test))]
    fn send_flip(
        &mut self,
        key: String,
        agent_id: String,
        grant: AppGrant,
        on: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let client = self.client.clone();
        let (target_bruh, target_grant) = (agent_id.clone(), grant.clone());
        let operation = spawn_tokio(cx, async move {
            let sent = client.set_app_grant(&target_bruh, &target_grant, on).await;
            let next = match &sent {
                Ok(()) => client.apps_listing().await.ok(),
                Err(_) => None,
            };
            (sent, next)
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let (sent, next) = operation.await;
            // The page or its window may be gone by now.
            let _ = this.update_in(cx, |this, window, cx| {
                this.flipped(&key, &agent_id, &grant, on, sent, next, window, cx);
            });
        }));
    }

    /// The server answered a flip, and the listing was read again if it
    /// went through.
    #[expect(
        clippy::too_many_arguments,
        reason = "one flip's whole answer, as the web's closure holds it"
    )]
    fn flipped(
        &mut self,
        key: &str,
        agent_id: &str,
        grant: &AppGrant,
        on: bool,
        sent: Result<(), ServerError>,
        next: Option<AppsListing>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        self.pending = None;
        // The switch was disabled while it waited, which moved focus to
        // the page: it goes back to the switch.
        if self.page_focus.is_focused(window) {
            if let Some(handle) = self.switch_focus.get(key) {
                handle.focus(window, cx);
            }
        }
        match sent {
            // The listing shows what the server holds; if it can't be
            // read, the grant shows as sent.
            Ok(()) => match next {
                Some(next) => self.listing = next,
                None => set_listing_grant(&mut self.listing, agent_id, grant, on),
            },
            Err(error) if error.vault_may_have_landed() => self.unanswered(window, cx),
            Err(error) => {
                self.grant_errors
                    .insert(grant_target(grant).to_owned(), error.vault_message());
            }
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn apps_requests_for_test(&self) -> Vec<String> {
        self.apps_requests.clone()
    }

    /// Show `listing` as if the server had sent it.
    #[cfg(test)]
    pub(super) fn show_listing_for_test(
        &mut self,
        listing: AppsListing,
        cx: &mut Context<'_, Self>,
    ) {
        self.answer_listing_for_test(Ok(listing), cx);
    }

    /// Answer the listing request as the server would.
    #[cfg(test)]
    pub(super) fn answer_listing_for_test(
        &mut self,
        result: Result<AppsListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        self.listing_answered(result, cx);
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────

/// Web `NOTE`: `mt-1 text-label-sm text-rambutan-400`.
fn note(text: impl Into<SharedString>) -> Div {
    small(text, theme::rambutan_400()).mt_1()
}

/// Web `text-label text-rambutan-100`, for an app's, account's or
/// mailbox's name.
fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
        .text_color(theme::rambutan_100())
        .child(text.into())
}

/// A status or alert line with its own id, for `role`.
fn line(id: String, text: impl Into<SharedString>, color: Hsla, role: Role) -> Stateful<Div> {
    let selector = id.clone();
    small(text, color)
        .id(SharedString::from(id))
        .debug_selector(move || selector)
        .role(role)
}

impl AppsSettings {
    /// One switch per bruh for `grant`, with the bruh's name and why it
    /// can't use apps yet.
    fn render_grants(&self, grant: &AppGrant, cx: &Context<'_, Self>) -> AnyElement {
        let target = grant_target(grant).to_owned();
        let reason = self.wait().or_else(|| self.grants_off());
        let switches = self.listing.bruhs.iter().map(|bruh| {
            let key = Self::switch_key(grant, &bruh.agent_id);
            let checked = match &self.pending {
                Some(pending) if pending.key == key => pending.on,
                _ => grant_switch_on(bruh, grant),
            };
            let name = review_text(&bruh.name);
            let mut toggle = switch(
                grant_selector(&target, &bruh.agent_id),
                name.clone(),
                checked,
                Interactivity::disabled_if(reason.is_some()),
            );
            if let Some(handle) = self.switch_focus.get(&key) {
                toggle = toggle.track_focus(handle);
            }
            let (agent_id, grant) = (bruh.agent_id.clone(), grant.clone());
            let toggle = on_press_view(toggle, cx, move |this, window, cx| {
                this.flip(agent_id.clone(), grant.clone(), !checked, window, cx);
            });
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
                        .child(small(name, theme::rambutan_200()))
                        .when(bruh.unavailable.is_some(), |el| {
                            let selector = format!("desktop-apps-need-ask-{key}");
                            el.child(
                                small(APPS_NEED_ASK, theme::durian_300())
                                    .debug_selector(move || selector),
                            )
                        }),
                )
                .child(toggle)
        });
        let no_bruhs = self.load == Load::Ready && self.listing.bruhs.is_empty();
        div()
            .id(SharedString::from(format!("desktop-apps-grants-{target}")))
            .role(Role::Group)
            .aria_label(GRANTS_LABEL)
            .mt_2()
            .min_w_0()
            .flex()
            .flex_col()
            .child(small(GRANTS_LABEL, theme::rambutan_300()))
            .when(no_bruhs, |el| el.child(note(NO_BRUHS_FOR_APPS)))
            .children(switches)
            .when_some(self.grant_errors.get(&target).cloned(), |el, message| {
                el.child(
                    line(
                        format!("desktop-apps-grant-error-{target}"),
                        message,
                        theme::lychee_300(),
                        Role::Alert,
                    )
                    .mt_1(),
                )
            })
            .into_any_element()
    }

    fn render_mail(&self, cx: &Context<'_, Self>) -> AnyElement {
        let empty = self.load == Load::Ready && self.listing.mailboxes.is_empty();
        let rows = self
            .listing
            .mailboxes
            .iter()
            .enumerate()
            .map(|(index, mailbox)| {
                let selector = format!("desktop-apps-mailbox-{}", mailbox.mailbox_id);
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .role(Role::Group)
                    .aria_label(review_text(&mailbox.account))
                    .min_w_0()
                    .py_3()
                    .when(index > 0, |el| {
                        el.border_t_1().border_color(theme::longan_800())
                    })
                    .child(heading(review_text(&mailbox.account)))
                    .child(note(mailbox.provider.name()))
                    .child(self.render_grants(
                        &AppGrant::Mailbox {
                            mailbox_id: mailbox.mailbox_id.clone(),
                        },
                        cx,
                    ))
            });
        div()
            .id("desktop-apps-mail")
            .debug_selector(|| "desktop-apps-mail".into())
            .role(Role::Group)
            .aria_label(MAIL_GROUP)
            .mt_4()
            .min_w_0()
            .p_3()
            .rounded(px(GROUP_RADIUS_PX))
            .bg(theme::longan_950().opacity(GROUP_FILL_OPACITY))
            .border_1()
            .border_color(theme::longan_700().opacity(GROUP_RING_OPACITY))
            .flex()
            .flex_col()
            .child(heading(MAIL_GROUP))
            .child(note(MAIL_HINT))
            .when(empty, |el| el.child(note(NO_MAILBOXES)))
            .child(div().min_w_0().flex().flex_col().children(rows))
            .into_any_element()
    }

    fn render_app(
        &self,
        index: usize,
        app: &'static AppConnector,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let connecting = matches!(&self.form, Some(form) if form.open == Open::Connect(app.id));
        let mut connect = button(
            SharedString::from(format!("desktop-apps-connect-{}", app.id)),
            format!("desktop-apps-connect-{}", app.id),
            CONNECT,
            WebButtonVariant::Outline,
            self.blocked(),
        )
        .aria_expanded(connecting);
        if let Some(handle) = self.connect_focus.get(app.id) {
            connect = connect.track_focus(handle);
        }
        let connect = on_press_view(connect, cx, move |this, window, cx| {
            this.open_connect(app.id, window, cx);
        });
        let accounts = self
            .listing
            .accounts
            .iter()
            .filter(|account| account.app == app.id)
            .enumerate()
            .map(|(index, account)| self.render_account(index, account, cx));
        let selector = format!("desktop-apps-app-{}", app.id);
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .role(Role::Group)
            .aria_label(app.name)
            .min_w_0()
            .py_4()
            .when(index > 0, |el| {
                el.border_t_1().border_color(theme::longan_800())
            })
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(heading(app.name))
                            .child(note(app.scopes_hint)),
                    )
                    .child(connect),
            )
            .when(connecting, |el| {
                el.children(self.form.as_ref().map(|form| self.render_form(form, cx)))
            })
            .child(div().min_w_0().flex().flex_col().children(accounts))
            .into_any_element()
    }

    fn render_account(
        &self,
        index: usize,
        account: &AppAccount,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = account.id.clone();
        let waiting = self.wait().is_some();
        let asking = self.asking.as_deref() == Some(id.as_str());
        let replacing = matches!(&self.form, Some(form) if form.open == Open::Replace(id.clone()));
        let tested = self.tested.get(&id).cloned();
        let focus = self.rows_focus.get(&id);
        let stuck = account.state == SavedCredentialState::NeedsAttention;
        // An earlier Test's line stands in for the stored attention.
        let attention = account
            .attention
            .filter(|_| tested.is_none())
            .map(|attention| attention.message());
        let info = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(heading(review_text(&account.name)))
            .child(
                div()
                    .mt_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .gap_x_3()
                    .child(small(review_text(&account.account), theme::rambutan_300()))
                    .child(small(
                        app_scope_line(&account.scope, &self.listing.bruhs),
                        theme::rambutan_300(),
                    )),
            )
            .child(if stuck {
                small(STATE_NEEDS_ATTENTION, theme::durian_300()).mt_1()
            } else {
                note(STATE_SAVED)
            })
            .when_some(attention, |el, attention| {
                let selector = format!("desktop-apps-attention-{id}");
                el.child(
                    small(attention, theme::durian_300())
                        .mt_1()
                        .debug_selector(move || selector),
                )
            });
        let test = {
            let id = id.clone();
            on_press_view(
                button(
                    SharedString::from(format!("desktop-apps-test-{id}")),
                    format!("desktop-apps-test-{id}"),
                    TEST,
                    WebButtonVariant::Outline,
                    waiting,
                ),
                cx,
                move |this, window, cx| this.test(id.clone(), window, cx),
            )
        };
        let replace = {
            let id = id.clone();
            let mut replace = button(
                SharedString::from(format!("desktop-apps-replace-{id}")),
                format!("desktop-apps-replace-{id}"),
                REPLACE_TOKEN,
                WebButtonVariant::Outline,
                waiting,
            )
            .aria_expanded(replacing);
            if let Some(focus) = focus {
                replace = replace.track_focus(&focus.replace);
            }
            on_press_view(replace, cx, move |this, window, cx| {
                this.open_replace(id.clone(), window, cx);
            })
        };
        let disconnect = (!asking).then(|| {
            let id = id.clone();
            let mut disconnect = button(
                SharedString::from(format!("desktop-apps-disconnect-{id}")),
                format!("desktop-apps-disconnect-{id}"),
                DISCONNECT,
                WebButtonVariant::Outline,
                waiting,
            );
            if let Some(focus) = focus {
                disconnect = disconnect.track_focus(&focus.disconnect);
            }
            on_press_view(disconnect, cx, move |this, window, cx| {
                this.ask_disconnect(id.clone(), window, cx);
            })
        });
        let confirm = asking.then(|| {
            let mut keep = button(
                "desktop-apps-keep",
                "desktop-apps-keep".into(),
                KEEP,
                WebButtonVariant::Outline,
                waiting,
            );
            if let Some(focus) = focus {
                keep = keep.track_focus(&focus.keep);
            }
            let mut question = panel().id("desktop-apps-question").role(Role::Group);
            // Named by the question, as the web's group is.
            if let Some(line) = self.confirm_line() {
                question = question.aria_label(line);
            }
            question
                .on_action(cx.listener(|this, _: &CloseOverlays, window, cx| {
                    this.keep(window, cx);
                }))
                .child(
                    div()
                        .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                        .text_color(theme::rambutan_200())
                        .children(self.confirm_line()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(on_press_view(
                            button(
                                "desktop-apps-confirm",
                                "desktop-apps-confirm".into(),
                                DISCONNECT,
                                WebButtonVariant::Destructive,
                                waiting,
                            ),
                            cx,
                            |this, window, cx| this.disconnect(window, cx),
                        ))
                        .child(on_press_view(keep, cx, |this, window, cx| {
                            this.keep(window, cx);
                        })),
                )
        });
        let selector = format!("desktop-apps-account-{id}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .role(Role::Group)
            .aria_label(review_text(&account.name))
            .min_w_0()
            .py_3()
            .when(index > 0, |el| {
                el.border_t_1().border_color(theme::longan_800())
            })
            .flex()
            .flex_col()
            .child(
                div().flex().items_start().gap_3().child(info).child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .gap_2()
                        .child(test)
                        .child(replace)
                        .children(disconnect),
                ),
            )
            .children(confirm)
            .when(replacing, |el| {
                el.children(self.form.as_ref().map(|form| self.render_form(form, cx)))
            })
            .when_some(tested, |el, (ok, message)| {
                el.child(
                    line(
                        format!("desktop-apps-tested-{id}"),
                        message,
                        if ok {
                            theme::pandan_300()
                        } else {
                            theme::durian_300()
                        },
                        Role::Status,
                    )
                    .mt_2(),
                )
            })
            .when_some(self.row_errors.get(&id).cloned(), |el, message| {
                el.child(
                    line(
                        format!("desktop-apps-error-{id}"),
                        message,
                        theme::lychee_300(),
                        Role::Alert,
                    )
                    .mt_2(),
                )
            })
            // A bruh's own account is its own; only shared ones are granted.
            .when(account.scope == CredentialScope::Global, |el| {
                el.child(self.render_grants(&AppGrant::Token { entry_id: id }, cx))
            })
            .into_any_element()
    }
}

impl gpui::EventEmitter<CredentialsChanged> for AppsSettings {}

impl Render for AppsSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.load == Load::Idle {
            self.refresh(window, cx);
        }
        // Every row's buttons and every switch keep their focus handles
        // while they show.
        let ids: Vec<String> = self.listing.accounts.iter().map(|a| a.id.clone()).collect();
        self.rows_focus.retain(|id, _| ids.contains(id));
        for id in &ids {
            self.row_focus(id, cx);
        }
        let mut targets: Vec<AppGrant> = self
            .listing
            .accounts
            .iter()
            .filter(|account| account.scope == CredentialScope::Global)
            .map(|account| AppGrant::Token {
                entry_id: account.id.clone(),
            })
            .collect();
        targets.extend(
            self.listing
                .mailboxes
                .iter()
                .map(|mailbox| AppGrant::Mailbox {
                    mailbox_id: mailbox.mailbox_id.clone(),
                }),
        );
        let keys: HashSet<String> = targets
            .iter()
            .flat_map(|grant| {
                self.listing
                    .bruhs
                    .iter()
                    .map(move |bruh| Self::switch_key(grant, &bruh.agent_id))
            })
            .collect();
        self.switch_focus.retain(|key, _| keys.contains(key));
        for key in keys {
            self.switch_focus
                .entry(key)
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        let wait = self.wait();
        let status = self.status_line().map(|text| {
            let notice = matches!(self.load, Load::ServingOld | Load::OwnerOnly);
            let (color, role) = match &self.load {
                Load::Failed(_) => (theme::lychee_300(), Role::Alert),
                _ if notice => (theme::durian_300(), Role::Status),
                _ => (theme::rambutan_400(), Role::Status),
            };
            small(text, color)
                .mt_3()
                .id("desktop-apps-status")
                .role(role)
                .when(notice, |el| {
                    el.debug_selector(|| "desktop-apps-notice".into())
                })
        });
        let grants_off = self
            .grants_off()
            .filter(|_| self.load == Load::Ready)
            .map(|reason| {
                line(
                    "desktop-apps-grants-off".into(),
                    reason,
                    theme::durian_300(),
                    Role::Status,
                )
                .mt_3()
            });
        let notice = self.notice.clone().map(|notice| {
            let (text, color) = match notice {
                Notice::Done(text) => (text, theme::pandan_300()),
                Notice::MayHaveChanged => (
                    VaultNotice::MayHaveChanged.message().to_owned(),
                    theme::durian_300(),
                ),
            };
            line("desktop-apps-page-notice".into(), text, color, Role::Status).mt_2()
        });
        let refresh = on_press_view(
            button(
                "desktop-apps-refresh",
                "desktop-apps-refresh".into(),
                REFRESH,
                WebButtonVariant::Outline,
                wait.is_some(),
            ),
            cx,
            |this, window, cx| this.refresh(window, cx),
        );
        let apps: Vec<AnyElement> = CONNECTORS
            .iter()
            .enumerate()
            .map(|(index, app)| self.render_app(index, app, cx))
            .collect();
        div()
            .id("desktop-apps")
            .track_focus(&self.page_focus)
            .p_4()
            .min_w_0()
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
            .text_color(theme::rambutan_200())
            .flex()
            .flex_col()
            .child(div().flex().items_center().gap_2().child(refresh))
            .children(status)
            .children(grants_off)
            // A load says so in the status line above.
            .when_some(wait.filter(|_| self.load != Load::Loading), |el, reason| {
                el.child(
                    line(
                        "desktop-apps-wait".into(),
                        reason,
                        theme::rambutan_400(),
                        Role::Status,
                    )
                    .mt_2(),
                )
            })
            .children(notice)
            .child(self.render_mail(cx))
            .child(div().mt_4().min_w_0().flex().flex_col().children(apps))
    }
}

#[cfg(test)]
mod tests;
