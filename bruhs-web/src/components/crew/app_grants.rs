//! The apps a bruh's Settings grant it (D22): its own app accounts as
//! lines, then one switch per shared app account and per connected
//! mailbox. Unlike the Vault section, a switch sends `SetAppGrant` at once
//! (D9), then the section reads the listing again; Save never carries app
//! grants. The rows are `bruhs_models::crew::app_grant_rows`, so the
//! desktop section matches. Nothing about apps reaches browser storage.

use bruhs_models::agent::AgentProvider;
use bruhs_models::apps::{open_apps_from_own_address, AppsListing, APPS_LOADING};
use bruhs_models::crew::{
    app_grant_rows, apps_ready, apps_reason, apps_unavailable, current_settings, AppGrant,
    AppGrantRow, ServiceNotice, BRUH_APPS_HINT, BRUH_APPS_LABEL, BRUH_APPS_NONE,
};
use bruhs_models::vault::{VaultNotice, LOAD_RETRY, REFRESH, WAIT_FOR_CHANGE};
use leptos::prelude::*;

use super::data::use_crew_snapshot;
use crate::components::apps_settings::{focus_dropped, focus_selector_after_render, set_grant};
use crate::ui::{Button, ButtonSize, Switch};

const HINT: &str = "text-label-sm text-rambutan-500";
const NOTICE: &str = "text-label-sm text-durian-300";
const HINT_ID: &str = "bruh-apps-hint";
const OFF_ID: &str = "bruh-apps-off";
const ASK_ID: &str = "bruh-apps-ask";
const WAIT_ID: &str = "bruh-apps-wait";

#[derive(Clone, PartialEq)]
enum Load {
    Loading,
    Ready(AppsListing),
    Failed(String),
    /// An older server: no `/api/apps`.
    ServingOld,
    /// A page on another address: every Apps route is the owner's (D23).
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

/// A row's key, so a flip keeps the switch (and its focus) in place.
fn row_key(row: &AppGrantRow) -> String {
    match row {
        AppGrantRow::Own(line) => format!("own {line}"),
        AppGrantRow::Account { id, .. } => format!("app {id}"),
        AppGrantRow::Mailbox { id, .. } => format!("mail {id}"),
    }
}

#[component]
pub fn AppGrants(agent_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    let agent_id = StoredValue::new(agent_id);
    let load = RwSignal::new(Load::Loading);
    // A flip on its way, and the switch it shows until the listing answers.
    let busy = RwSignal::new(false);
    let pending = RwSignal::new(None::<(String, bool)>);
    let error = RwSignal::new(None::<String>);
    let may_have_changed = RwSignal::new(false);

    let fetch = move || {
        load.set(Load::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            let next = match crate::api::apps_listing().await {
                Ok(listing) => Load::Ready(listing),
                Err(error) if error.is_status(404) => Load::ServingOld,
                Err(error) if error.is_status(403) => Load::OwnerOnly,
                Err(error) => Load::Failed(crate::api::vault_message(&error)),
            };
            // Settings may have closed while the listing came.
            load.try_set(next);
        });
    };
    fetch();

    let rows = Memo::new(move |_| {
        load.with(|load| match load {
            Load::Ready(listing) => agent_id.with_value(|id| app_grant_rows(listing, id)),
            _ => Vec::new(),
        })
    });
    let off = Memo::new(move |_| load.with(|load| switches_off(data.apps_reason(), load)));
    let reason =
        Signal::derive(move || off.get().or_else(|| busy.get().then_some(WAIT_FOR_CHANGE)));
    // The stored level and model decide, since a switch saves at once.
    let need_ask = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            let settings = agent_id.with_value(|id| current_settings(snapshot.as_ref()?, id))?;
            apps_unavailable(
                settings.access.level,
                AgentProvider::for_model(&settings.model),
            )
        })
    });
    let described = move || {
        let mut ids = vec![HINT_ID, WAIT_ID];
        if off.with(Option::is_some) {
            ids.push(OFF_ID);
        }
        if need_ask.with(Option::is_some) {
            ids.push(ASK_ID);
        }
        ids.join(" ")
    };

    let flip = move |row: AppGrantRow, on: bool| {
        let Some(grant) = row.grant() else {
            return;
        };
        if busy.get_untracked() || off.get_untracked().is_some() {
            return;
        }
        busy.set(true);
        error.set(None);
        may_have_changed.set(false);
        pending.set(Some((row_key(&row), on)));
        let target = match &grant {
            AppGrant::Token { entry_id } => entry_id,
            AppGrant::Mailbox { mailbox_id } => mailbox_id,
        };
        let focus = format!("[data-bruh-app-grant=\"{target}\"]");
        let agent = agent_id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let sent = crate::api::set_app_grant(&agent, &grant, on).await;
            // The listing shows what the service holds, a flip that got no
            // answer included.
            let next = match &sent {
                Ok(()) => Some(crate::api::apps_listing().await),
                Err(err) if crate::api::vault_failure(err).may_have_landed() => {
                    Some(crate::api::apps_listing().await)
                }
                Err(_) => None,
            };
            match next {
                Some(Ok(listing)) => {
                    load.try_set(Load::Ready(listing));
                }
                // The grant landed but the listing can't be read: show it
                // as sent, kept as the service keeps it.
                Some(Err(_)) if sent.is_ok() => {
                    load.try_update(|load| {
                        if let Load::Ready(listing) = load {
                            set_grant(listing, &agent, &grant, on);
                        }
                    });
                }
                _ => {}
            }
            match sent {
                Ok(()) => {}
                Err(err) if crate::api::vault_failure(&err).may_have_landed() => {
                    may_have_changed.try_set(true);
                }
                Err(err) => {
                    error.try_set(Some(crate::api::vault_message(&err)));
                }
            }
            pending.try_set(None);
            busy.try_set(false);
            // The switch was disabled while it waited, which drops focus.
            if focus_dropped() {
                focus_selector_after_render(focus);
            }
        });
    };

    let own = |line: String| {
        view! {
            <li data-bruh-app-own="" class="flex min-h-control min-w-0 items-center break-words text-label-sm text-rambutan-300">{line}</li>
        }
    };
    let shared = move |row: AppGrantRow| {
        let (AppGrantRow::Account { id, label, .. } | AppGrantRow::Mailbox { id, label, .. }) =
            row.clone()
        else {
            return ().into_any();
        };
        let key = row_key(&row);
        let checked = Signal::derive(move || {
            pending
                .with(|pending| {
                    pending
                        .as_ref()
                        .filter(|(at, _)| *at == key)
                        .map(|(_, on)| *on)
                })
                .unwrap_or_else(|| {
                    rows.with(|rows| {
                        rows.iter().any(|shown| {
                            row_key(shown) == key
                                && matches!(
                                    shown,
                                    AppGrantRow::Account { on: true, .. }
                                        | AppGrantRow::Mailbox { on: true, .. }
                                )
                        })
                    })
                })
        });
        view! {
            <li class="flex min-h-control min-w-0 items-center justify-between gap-3">
                <span class="min-w-0 break-words text-label-sm text-rambutan-200">{label.clone()}</span>
                // The label makes the whole 44px touch target press the switch.
                <label class="bruhs-control inline-flex shrink-0 items-center justify-center">
                    <Switch
                        label=label
                        checked=checked
                        disabled=Signal::derive(move || reason.with(Option::is_some))
                        on_change=Callback::new(move |on| flip(row.clone(), on))
                        attr:data-bruh-app-grant=id
                        attr:title=move || reason.get()
                        attr:aria-describedby=described
                    />
                </label>
            </li>
        }
        .into_any()
    };
    let row = move |row: AppGrantRow| match row {
        AppGrantRow::Own(line) => own(line).into_any(),
        row => shared(row),
    };
    let ready = Memo::new(move |_| load.with(|load| matches!(load, Load::Ready(_))));
    let none =
        move || !rows.with(|rows| rows.iter().any(|row| !matches!(row, AppGrantRow::Own(_))));

    view! {
        <div role="group" aria-labelledby="bruh-apps-label" data-bruh-apps="" class="min-w-0 space-y-1">
            <div class="min-w-0">
                <span id="bruh-apps-label" class="text-label-sm text-rambutan-300">{BRUH_APPS_LABEL}</span>
                <p id=HINT_ID class=HINT>{BRUH_APPS_HINT}</p>
                {move || off.get().map(|reason| view! { <p id=OFF_ID data-bruh-apps-off="" class=NOTICE>{reason}</p> })}
                {move || need_ask.get().map(|reason| view! { <p id=ASK_ID data-bruh-apps-need-ask="" class=NOTICE>{reason}</p> })}
                // Always drawn, so every switch's `aria-describedby` names
                // an element; empty while nothing runs.
                <p id=WAIT_ID role="status" class=format!("{HINT} empty:hidden")>{move || busy.get().then_some(WAIT_FOR_CHANGE)}</p>
            </div>
            {move || match load.get() {
                Load::Loading => view! { <p role="status" class=HINT>{APPS_LOADING}</p> }.into_any(),
                Load::Failed(message) => view! {
                    <div class="flex min-w-0 flex-wrap items-center gap-3">
                        <p role="alert" class="min-w-0 text-label-sm text-lychee-300">{format!("{message} {LOAD_RETRY}")}</p>
                        <Button size=ButtonSize::Small on_click=Callback::new(move |_| fetch()) attr:data-bruh-apps-refresh="">{REFRESH}</Button>
                    </div>
                }
                .into_any(),
                // An older service already says so through `off`.
                Load::ServingOld => off
                    .get()
                    .is_none()
                    .then(|| view! { <p role="status" data-bruh-apps-notice="" class=NOTICE>{ServiceNotice::ServingOld.message()}</p> })
                    .into_any(),
                Load::OwnerOnly => view! { <p role="status" data-bruh-apps-notice="" class=NOTICE>{open_apps_from_own_address(crate::api::BASE_URL)}</p> }.into_any(),
                Load::Ready(_) => ().into_any(),
            }}
            <Show when=move || ready.get()>
                // A gap keeps neighbouring touch targets apart.
                <ul class="min-w-0 space-y-1">
                    <For each=move || rows.get() key=row_key children=row />
                </ul>
                <Show when=none>
                    <p data-bruh-apps-none="" class=HINT>{BRUH_APPS_NONE}</p>
                </Show>
            </Show>
            {move || may_have_changed.get().then(|| view! { <p role="status" data-bruh-apps-may-have-changed="" class=NOTICE>{VaultNotice::MayHaveChanged.message()}</p> })}
            {move || error.get().map(|message| view! { <p role="alert" class="text-label-sm text-lychee-300">{message}</p> })}
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::apps::{AppAccount, AppMailbox, AppsBruh};
    use bruhs_models::connections::ServiceProvider;
    use bruhs_models::crew::APPS_PROTOCOL;
    use bruhs_models::secrets::SavedCredentialState;
    use bruhs_models::vault::CredentialScope;

    fn listing(protocol: u32) -> AppsListing {
        let account = |id: &str, app: &str, name: &str, scope: CredentialScope| AppAccount {
            id: id.into(),
            app: app.into(),
            name: name.into(),
            account: "octo".into(),
            scope,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        AppsListing {
            accounts: vec![
                account("v1-n", "notion", "Notion ws", CredentialScope::Global),
                account(
                    "v1-o",
                    "github",
                    "GitHub octo",
                    CredentialScope::Bruh {
                        agent_id: "a".into(),
                    },
                ),
            ],
            mailboxes: vec![AppMailbox {
                mailbox_id: "mailbox-v1-gmail".into(),
                provider: ServiceProvider::Gmail,
                account: "me@x.test".into(),
            }],
            bruhs: vec![AppsBruh {
                agent_id: "a".into(),
                name: "Ada".into(),
                apps: vec!["v1-n".into()],
                mail: Vec::new(),
                unavailable: None,
            }],
            protocol,
        }
    }

    #[test]
    fn grant_rows_list_own_accounts_then_shared_ones_then_mailboxes() {
        assert_eq!(
            app_grant_rows(&listing(APPS_PROTOCOL), "a"),
            vec![
                AppGrantRow::Own("GitHub octo: its own".into()),
                AppGrantRow::Account {
                    id: "v1-n".into(),
                    label: "Notion ws, Notion".into(),
                    on: true,
                },
                AppGrantRow::Mailbox {
                    id: "mailbox-v1-gmail".into(),
                    label: "me@x.test, Gmail".into(),
                    on: false,
                },
            ]
        );
        // Keys stay apart even when an account and a mailbox share an id.
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
    fn an_older_service_turns_every_switch_off() {
        let ready = Load::Ready(listing(APPS_PROTOCOL));
        assert_eq!(switches_off(None, &ready), None);
        let old = Load::Ready(listing(APPS_PROTOCOL - 1));
        assert_eq!(
            switches_off(None, &old),
            Some(ServiceNotice::ServingOld.message())
        );
        assert_eq!(switches_off(Some("older"), &ready), Some("older"));
        assert_eq!(switches_off(None, &Load::ServingOld), None);
    }
}
