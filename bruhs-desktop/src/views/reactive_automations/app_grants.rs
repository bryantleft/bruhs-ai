//! The apps a bruh's Settings grant it (D22; web `crew/app_grants.rs`):
//! its own app accounts as lines, then one switch per shared app account
//! and per connected mailbox. Unlike the Vault section, a switch sends
//! `SetAppGrant` at once (D9), then the section reads the listing again;
//! Save never carries app grants. The rows are
//! `bruhs_models::crew::app_grant_rows`, so the web section matches. The
//! listing is read when Settings opens and forgotten when it closes; it
//! holds names and grants, never a token.
use super::bruh_page::{page_button, page_switch, small_text};
use super::create_bruh::hint;
use super::*;
use bruhs_models::apps::{open_apps_from_own_address, AppsListing, APPS_LOADING};
use bruhs_models::crew::{
    app_grant_rows, apps_ready, apps_reason, set_listing_grant, AppGrant, AppGrantRow,
    BRUH_APPS_HINT, BRUH_APPS_LABEL, BRUH_APPS_NONE,
};
use bruhs_models::vault::{VaultNotice, LOAD_RETRY, REFRESH, WAIT_FOR_CHANGE};

/// A switch's id (`crate::views::kept_selector`).
fn grant_selector(id: &str) -> &'static str {
    crate::views::kept_selector(format!("desktop-bruh-app-grant-{id}"))
}

/// A row's key, so a flip's switch shows its new state until the listing
/// answers. An account and a mailbox never share one.
fn row_key(row: &AppGrantRow) -> String {
    match row {
        AppGrantRow::Own(line) => format!("own {line}"),
        AppGrantRow::Account { id, .. } => format!("app {id}"),
        AppGrantRow::Mailbox { id, .. } => format!("mail {id}"),
    }
}

#[derive(Clone, Default, PartialEq)]
enum Load {
    /// Not asked yet: Settings asks once it shows.
    #[default]
    Idle,
    Loading,
    Ready(AppsListing),
    Failed(String),
    /// An older server: no `/api/apps`.
    ServingOld,
    /// The server refused this client as not the owner's app (D23).
    OwnerOnly,
}

/// Why every switch is off: the crew service's reason, else the listing's
/// own protocol (ruling F19). `None` while grants work.
fn switches_off(crew_reason: Option<&'static str>, load: &Load) -> Option<&'static str> {
    crew_reason.or_else(|| match load {
        Load::Ready(listing) if !apps_ready(listing.protocol) => {
            Some(apps_reason(Some(listing.protocol)).unwrap_or(ServiceNotice::ServingOld.message()))
        }
        _ => None,
    })
}

/// The listing behind the section, while a bruh's Settings show.
#[derive(Default)]
pub(crate) struct AppGrantsState {
    load: Load,
    /// A flip on its way: its row's key and the switch it shows until the
    /// listing answers.
    pending: Option<(String, bool)>,
    /// Why the last flip was refused.
    error: Option<String>,
    /// The last flip got no answer, so it may have landed.
    may_have_changed: bool,
    /// The listing request or flip in flight; dropping it drops the answer.
    task: Option<Task<()>>,
    /// Every request, as `METHOD path`; tests never reach a server.
    #[cfg(test)]
    requests: Vec<String>,
}

impl AppGrantsState {
    /// Settings closed: forget the listing and any answer still coming.
    pub(super) fn reset(&mut self) {
        *self = Self {
            #[cfg(test)]
            requests: std::mem::take(&mut self.requests),
            ..Self::default()
        };
    }

    pub(super) fn idle(&self) -> bool {
        self.load == Load::Idle
    }

    /// Whether the service refused the listing as not the owner's app.
    pub(super) fn owner_only(&self) -> bool {
        self.load == Load::OwnerOnly
    }

    fn record(&mut self, request: String) {
        #[cfg(test)]
        self.requests.push(request);
        #[cfg(not(test))]
        drop(request);
    }

    fn answered(&mut self, result: Result<AppsListing, ServerError>) {
        self.task = None;
        self.load = match result {
            Ok(listing) => Load::Ready(listing),
            Err(error) if error.is_not_found() => Load::ServingOld,
            Err(ServerError::Status { code, .. }) if code == reqwest::StatusCode::FORBIDDEN => {
                Load::OwnerOnly
            }
            Err(error) => Load::Failed(error.vault_message()),
        };
    }

    /// A flip came back, with the listing read again when it may have
    /// landed. The listing shows what the service holds; a grant that
    /// landed while the listing can't be read shows as sent.
    fn flipped(
        &mut self,
        agent_id: &str,
        grant: &AppGrant,
        on: bool,
        sent: Result<(), ServerError>,
        next: Option<Result<AppsListing, ServerError>>,
    ) {
        self.task = None;
        self.pending = None;
        match next {
            Some(Ok(listing)) => self.load = Load::Ready(listing),
            Some(Err(_)) if sent.is_ok() => {
                if let Load::Ready(listing) = &mut self.load {
                    set_listing_grant(listing, agent_id, grant, on);
                }
            }
            _ => {}
        }
        match sent {
            Ok(()) => {}
            Err(error) if error.vault_may_have_landed() => self.may_have_changed = true,
            Err(error) => self.error = Some(error.vault_message()),
        }
    }
}

impl ReactiveAutomations {
    /// Read the Apps listing for the open bruh's Settings.
    pub(super) fn load_bruh_apps(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let Some(state) = self.bruh_apps_mut() else {
            return;
        };
        state.load = Load::Loading;
        state.record("GET /api/apps".into());
        cx.notify();
        if cfg!(test) {
            return;
        }
        let operation = spawn_tokio(cx, async move { client.apps_listing().await });
        let task = cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The panel may be gone by now; a closed Settings or another
            // bruh's page dropped this task with its state.
            let _ = this.update(cx, |this, cx| this.bruh_apps_answered(result, cx));
        });
        if let Some(state) = self.bruh_apps_mut() {
            state.task = Some(task);
        }
    }

    fn bruh_apps_answered(
        &mut self,
        result: Result<AppsListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(state) = self.bruh_apps_mut() {
            state.answered(result);
            cx.notify();
        }
    }

    /// Flip the open bruh's switch for `row`: `SetAppGrant` goes at once,
    /// then the listing is read again. Nothing goes while a flip runs or
    /// the service predates app grants.
    fn flip_bruh_app(&mut self, row: &AppGrantRow, on: bool, cx: &mut Context<'_, Self>) {
        let Some(grant) = row.grant() else {
            return;
        };
        let Some(agent_id) = self.bruh_page_agent().map(str::to_owned) else {
            return;
        };
        let crew_reason = self.apps_reason();
        let client = self.client.clone();
        let Some(state) = self.bruh_apps_mut() else {
            return;
        };
        if state.pending.is_some() || switches_off(crew_reason, &state.load).is_some() {
            return;
        }
        state.pending = Some((row_key(row), on));
        state.error = None;
        state.may_have_changed = false;
        state.record(format!(
            "POST {}",
            crate::server_client::apps::COMMANDS_PATH
        ));
        cx.notify();
        if cfg!(test) {
            #[cfg(test)]
            self.sent.push(ReactiveCommand::SetAppGrant {
                agent_id,
                grant,
                granted: on,
            });
            return;
        }
        let (target_bruh, target_grant) = (agent_id.clone(), grant.clone());
        let operation = spawn_tokio(cx, async move {
            let sent = client.set_app_grant(&target_bruh, &target_grant, on).await;
            let next = match &sent {
                Ok(()) => Some(client.apps_listing().await),
                Err(error) if error.vault_may_have_landed() => Some(client.apps_listing().await),
                Err(_) => None,
            };
            (sent, next)
        });
        let task = cx.spawn(async move |this, cx| {
            let (sent, next) = operation.await;
            // The panel, or this bruh's Settings, may be gone by now.
            let _ = this.update(cx, |this, cx| {
                this.bruh_app_flipped(&agent_id, &grant, on, sent, next, cx);
            });
        });
        if let Some(state) = self.bruh_apps_mut() {
            state.task = Some(task);
        }
    }

    fn bruh_app_flipped(
        &mut self,
        agent_id: &str,
        grant: &AppGrant,
        on: bool,
        sent: Result<(), ServerError>,
        next: Option<Result<AppsListing, ServerError>>,
        cx: &mut Context<'_, Self>,
    ) {
        // Only the page the flip was for takes its answer.
        if self.bruh_page_agent() != Some(agent_id) {
            return;
        }
        if let Some(state) = self.bruh_apps_mut() {
            state.flipped(agent_id, grant, on, sent, next);
            cx.notify();
        }
    }

    /// Answer the listing request as the server would.
    #[cfg(test)]
    pub(super) fn answer_bruh_apps_for_test(
        &mut self,
        result: Result<AppsListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        self.bruh_apps_answered(result, cx);
    }

    /// Answer the flip on its way as the server would.
    #[cfg(test)]
    pub(super) fn answer_bruh_app_flip_for_test(
        &mut self,
        sent: Result<(), ServerError>,
        next: Option<Result<AppsListing, ServerError>>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(ReactiveCommand::SetAppGrant {
            agent_id,
            grant,
            granted,
        }) = self.sent.last().cloned()
        else {
            panic!("no flip on its way");
        };
        self.bruh_app_flipped(&agent_id, &grant, granted, sent, next, cx);
    }

    #[cfg(test)]
    pub(super) fn bruh_apps_requests_for_test(&self) -> Vec<String> {
        self.bruh_apps()
            .map(|state| state.requests.clone())
            .unwrap_or_default()
    }
}

/// The section: label, hint and why it is off, then the listing's state or
/// its rows, 4px apart so neighbouring touch targets stay apart.
/// `disabled` is the crew service's reason; `need_ask` says the bruh can't
/// use apps at its stored level and model, while its switches still send.
pub(crate) fn app_grants(
    state: &AppGrantsState,
    agent_id: &str,
    disabled: Option<&'static str>,
    need_ask: Option<&'static str>,
    server: &str,
    cx: &Context<'_, ReactiveAutomations>,
) -> impl IntoElement {
    let off = switches_off(disabled, &state.load);
    let busy = state.pending.is_some();
    let notice_line = |id: &'static str, text: SharedString| {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(Role::Status)
            .child(small_text(text, theme::durian_300()))
    };
    let head = div()
        .min_w_0()
        .flex()
        .flex_col()
        .child(small_text(BRUH_APPS_LABEL, theme::rambutan_300()))
        .child(hint(BRUH_APPS_HINT))
        .when_some(off, |el, reason| {
            el.child(notice_line("desktop-bruh-apps-off", reason.into()))
        })
        .when_some(need_ask, |el, reason| {
            el.child(notice_line("desktop-bruh-apps-need-ask", reason.into()))
        })
        .when(busy, |el| {
            el.child(
                div()
                    .id("desktop-bruh-apps-wait")
                    .debug_selector(|| "desktop-bruh-apps-wait".to_string())
                    .role(Role::Status)
                    .child(hint(WAIT_FOR_CHANGE)),
            )
        });
    let body = match &state.load {
        Load::Idle => None,
        Load::Loading => Some(
            div()
                .id("desktop-bruh-apps-loading")
                .debug_selector(|| "desktop-bruh-apps-loading".to_string())
                .role(Role::Status)
                .child(hint(APPS_LOADING))
                .into_any_element(),
        ),
        Load::Failed(message) => Some(
            div()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .id("desktop-bruh-apps-error")
                        .debug_selector(|| "desktop-bruh-apps-error".to_string())
                        .role(Role::Alert)
                        .min_w_0()
                        .child(small_text(
                            format!("{message} {LOAD_RETRY}"),
                            theme::lychee_300(),
                        )),
                )
                .child(page_button(
                    "desktop-bruh-apps-refresh",
                    REFRESH,
                    WebButtonVariant::Primary,
                    false,
                    cx,
                    |this, _, cx| this.load_bruh_apps(cx),
                ))
                .into_any_element(),
        ),
        // An older service already says so through `off`.
        Load::ServingOld => off.is_none().then(|| {
            notice_line(
                "desktop-bruh-apps-notice",
                ServiceNotice::ServingOld.message().into(),
            )
            .into_any_element()
        }),
        Load::OwnerOnly => Some(
            notice_line(
                "desktop-bruh-apps-notice",
                open_apps_from_own_address(server).into(),
            )
            .into_any_element(),
        ),
        Load::Ready(listing) => {
            let rows = app_grant_rows(listing, agent_id);
            let none = rows.iter().all(|row| matches!(row, AppGrantRow::Own(_)));
            Some(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().min_w_0().flex().flex_col().gap_1().children(
                        rows.into_iter().enumerate().map(|(index, row)| {
                            grant_row(index, row, state.pending.as_ref(), off.is_some(), cx)
                        }),
                    ))
                    .when(none, |el| {
                        el.child(
                            div()
                                .debug_selector(|| "desktop-bruh-apps-none".to_string())
                                .child(hint(BRUH_APPS_NONE)),
                        )
                    })
                    .into_any_element(),
            )
        }
    };
    div()
        .id("desktop-bruh-apps")
        .debug_selector(|| "desktop-bruh-apps".to_string())
        .role(Role::Group)
        .aria_label(BRUH_APPS_LABEL)
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(head)
        .children(body)
        .when(state.may_have_changed, |el| {
            el.child(notice_line(
                "desktop-bruh-apps-may-have-changed",
                VaultNotice::MayHaveChanged.message().into(),
            ))
        })
        .when_some(state.error.clone(), |el, message| {
            el.child(
                div()
                    .id("desktop-bruh-apps-alert")
                    .debug_selector(|| "desktop-bruh-apps-alert".to_string())
                    .role(Role::Alert)
                    .child(small_text(message, theme::lychee_300())),
            )
        })
}

/// One row: an own account as a line, or a shared account or mailbox with
/// its switch. A flip on its way shows on its switch until the listing
/// answers. While it waits the other switches stay pressable, since a
/// disabled switch drops keyboard focus, and a press does nothing.
fn grant_row(
    index: usize,
    row: AppGrantRow,
    pending: Option<&(String, bool)>,
    off: bool,
    cx: &Context<'_, ReactiveAutomations>,
) -> Div {
    let line = div().min_h(px(theme::CONTROL_PX)).flex().items_center();
    let key = row_key(&row);
    let (id, label, on) = match &row {
        AppGrantRow::Own(text) => {
            return line
                .debug_selector(move || format!("desktop-bruh-app-own-{index}"))
                .child(small_text(text.clone(), theme::rambutan_300()));
        }
        AppGrantRow::Account { id, label, on } | AppGrantRow::Mailbox { id, label, on } => {
            (id.clone(), label.clone(), *on)
        }
    };
    let checked = pending
        .filter(|(at, _)| *at == key)
        .map_or(on, |(_, on)| *on);
    line.justify_between()
        .gap_3()
        .child(small_text(label.clone(), theme::rambutan_200()))
        .child(page_switch(
            grant_selector(&id),
            label,
            checked,
            off,
            cx,
            move |this, _, cx| this.flip_bruh_app(&row, !checked, cx),
        ))
}

#[cfg(test)]
mod tests {
    use super::{grant_selector, row_key, switches_off, Load};
    use bruhs_models::apps::AppsListing;
    use bruhs_models::crew::{AppGrantRow, ServiceNotice, APPS_PROTOCOL, VAULT_PROTOCOL};

    #[test]
    fn app_grant_switches_keep_one_selector_each() {
        assert_eq!(grant_selector("v1-n"), "desktop-bruh-app-grant-v1-n");
        assert!(std::ptr::eq(grant_selector("v1-n"), grant_selector("v1-n")));
        let account = AppGrantRow::Account {
            id: "x".into(),
            label: String::new(),
            on: false,
        };
        let mailbox = AppGrantRow::Mailbox {
            id: "x".into(),
            label: String::new(),
            on: false,
        };
        assert_ne!(row_key(&account), row_key(&mailbox));
    }

    /// D10, ruling F19: an older service turns every switch off with its
    /// reason, from the crew snapshot or the listing's own protocol.
    #[test]
    fn app_grants_go_off_on_an_older_service() {
        let ready = |protocol| {
            Load::Ready(AppsListing {
                protocol,
                ..AppsListing::default()
            })
        };
        assert_eq!(switches_off(None, &ready(APPS_PROTOCOL)), None);
        assert_eq!(
            switches_off(None, &ready(VAULT_PROTOCOL)),
            Some(ServiceNotice::ServingOld.message())
        );
        assert_eq!(
            switches_off(Some("older"), &ready(APPS_PROTOCOL)),
            Some("older")
        );
        assert_eq!(switches_off(None, &Load::ServingOld), None);
    }
}
