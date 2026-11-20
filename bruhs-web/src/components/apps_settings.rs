//! Settings, Apps (D21): the connected mailboxes a bruh may send from, then
//! the fourteen apps in catalog order, each with its accounts and a Connect
//! form. Shared accounts and mailboxes carry one switch per bruh.
//!
//! Tokens are write-only. Their inputs are never filled, are cleared on
//! submit, cancel and unmount, and keep their keys from the app's
//! shortcuts. Nothing about apps reaches IndexedDB or any browser storage:
//! the page holds the listing only while it is open. Every rule and line of
//! copy is `bruhs_models::apps`', so the desktop page matches.
//!
//! Each component ends in `.into_any()`: without it the page's nested view
//! types make the debug wasm link (`rust-lld`) crash.

mod form;

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
    CredentialScope, VaultBusy as Busy, VaultNotice, KEEP, LOAD_RETRY, REFRESH,
    STATE_NEEDS_ATTENTION, STATE_SAVED,
};
use leptos::{html, prelude::*};
use wasm_bindgen::JsCast;

use super::vault_settings::{
    focus_after_render, focus_on_page, unique_id, CredentialSync, BUTTON, DANGER_BUTTON,
};
use crate::ui::Switch;
use form::{ConnectFormView, ReplaceFormView};

/// Why a disabled control waits: the listing's state or a running change.
const STATUS_ID: &str = "apps-status";
const WAIT_ID: &str = "apps-wait";
/// Why every grant switch is off: an older service.
const GRANTS_OFF_ID: &str = "apps-grants-off";
const NOTE: &str = "mt-1 text-label-sm text-rambutan-400";

#[derive(Clone, PartialEq)]
enum Load {
    Loading,
    Ready,
    Failed(String),
    /// An older server: no `/api/apps`.
    ServingOld,
    /// A page on another address: every Apps route is the owner's (D23).
    OwnerOnly,
}

impl Load {
    /// What the status line says, and so why nothing can change yet; `None`
    /// once the listing is in.
    fn message(&self) -> Option<String> {
        match self {
            Self::Loading => Some(APPS_LOADING.to_owned()),
            Self::Ready => None,
            Self::Failed(message) => Some(format!("{message} {LOAD_RETRY}")),
            Self::ServingOld => Some(ServiceNotice::ServingOld.message().to_owned()),
            Self::OwnerOnly => Some(open_apps_from_own_address(crate::api::BASE_URL)),
        }
    }
}

/// Whether focus fell to the page's body, as it does when the focused
/// control is disabled while it waits.
pub(crate) fn focus_dropped() -> bool {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return false;
    };
    match (document.active_element(), document.body()) {
        (None, _) => true,
        (Some(active), Some(body)) => AsRef::<web_sys::Element>::as_ref(&body) == &active,
        (Some(_), None) => false,
    }
}

/// Focus the element `selector` names once the view has drawn.
pub(crate) fn focus_selector_after_render(selector: String) {
    request_animation_frame(move || {
        let target = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(&selector).ok().flatten())
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(target) = target {
            let _ = target.focus();
        }
    });
}

/// Which form is open: none, one app's Connect, or one account's Replace
/// token. One at a time, as on the Vault page.
#[derive(Clone, PartialEq)]
enum Open {
    None,
    Connect(&'static str),
    Replace(String),
}

/// The line above the apps after a change.
#[derive(Clone, PartialEq)]
enum Notice {
    /// It landed: what happened, in pandan.
    Done(String),
    /// It got no answer, and the listing reloads.
    MayHaveChanged,
}

/// The page's state, shared by every section, row and form.
#[derive(Clone, Copy)]
struct Page {
    listing: RwSignal<AppsListing>,
    load: RwSignal<Load>,
    /// Bumped by each full reload, so rows drop what an earlier test said.
    generation: RwSignal<u64>,
    busy: RwSignal<Busy>,
    open: RwSignal<Open>,
    notice: RwSignal<Option<Notice>>,
    wait: Signal<Option<&'static str>>,
    /// The whole page, for the late-answer focus guard.
    page_ref: NodeRef<html::Div>,
    refresh: Callback<()>,
    /// The Vault section on the same Settings page, when there is one.
    sync: Option<CredentialSync>,
}

impl Page {
    /// An account was added, replaced or removed, or may have been: the
    /// Vault section reads its listing again (final review BC M4).
    fn vault_changed(self) {
        if let Some(sync) = self.sync {
            CredentialSync::bump(sync.vault);
        }
    }

    fn ready(self) -> bool {
        self.load.with(|load| *load == Load::Ready)
    }

    /// Why a change can't start now, if it can't: a running change, or the
    /// status line's reason the listing isn't loaded.
    fn blocked(self) -> Option<String> {
        self.wait
            .get()
            .map(str::to_owned)
            .or_else(|| self.load.with(Load::message))
    }

    /// Why the grant switches are off, if they are: a service whose
    /// protocol predates app grants (ruling F19).
    fn grants_off(self) -> Option<&'static str> {
        let protocol = self.listing.with(|listing| listing.protocol);
        (!apps_ready(protocol))
            .then(|| apps_reason(Some(protocol)).unwrap_or(ServiceNotice::ServingOld.message()))
    }

    /// A change that got no answer may have landed: the listing reloads,
    /// which closes any form, and the page says so.
    fn unanswered(self) {
        self.refresh.try_run(());
        self.notice.try_set(Some(Notice::MayHaveChanged));
        self.vault_changed();
    }
}

#[component]
pub fn AppsSettings() -> impl IntoView {
    let listing = RwSignal::new(AppsListing::default());
    let load = RwSignal::new(Load::Loading);
    let generation = RwSignal::new(0u64);
    let busy = RwSignal::new(Busy::Idle);
    let open = RwSignal::new(Open::None);
    let notice = RwSignal::new(None::<Notice>);
    let page_ref = NodeRef::<html::Div>::new();
    let wait = Signal::derive(move || {
        load.with(|load| apps_wait_reason(busy.get(), *load == Load::Loading))
    });
    let refresh = Callback::new(move |_: ()| {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        busy.set(Busy::Changing);
        notice.set(None);
        open.set(Open::None);
        load.set(Load::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            let (next, state) = match crate::api::apps_listing().await {
                Ok(next) => (next, Load::Ready),
                Err(error) if error.is_status(404) => (AppsListing::default(), Load::ServingOld),
                Err(error) if error.is_status(403) => (AppsListing::default(), Load::OwnerOnly),
                Err(error) => (
                    AppsListing::default(),
                    Load::Failed(crate::api::vault_message(&error)),
                ),
            };
            listing.try_set(next);
            generation.try_update(|generation| *generation = generation.wrapping_add(1));
            load.try_set(state);
            busy.try_set(Busy::Idle);
        });
    });
    let page = Page {
        listing,
        load,
        generation,
        busy,
        open,
        notice,
        wait,
        page_ref,
        refresh,
        sync: use_context::<CredentialSync>(),
    };
    Effect::new(move |_| refresh.run(()));
    // The Vault deleted an app token: read the listing again. A loaded page
    // keeps its open form and rows; one that isn't loaded reloads in full.
    if let Some(sync) = page.sync {
        CredentialSync::on_bump(sync.apps, move || {
            if load.with_untracked(|load| *load != Load::Ready) {
                refresh.run(());
                return;
            }
            if busy.get_untracked() != Busy::Idle {
                return;
            }
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(next) = crate::api::apps_listing().await {
                    if busy.try_get_untracked() == Some(Busy::Idle) {
                        listing.try_set(next);
                    }
                }
            });
        });
    }
    let connectors: &'static [AppConnector] = &CONNECTORS;
    let mailboxes = move || {
        let generation = generation.get();
        listing.with(|listing| {
            listing
                .mailboxes
                .iter()
                .map(|mailbox| (generation, mailbox.mailbox_id.clone()))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <div node_ref=page_ref class="p-4 text-body-sm text-rambutan-200">
            <div class="flex flex-wrap items-center gap-2">
                <button type="button" data-apps-refresh="" class=BUTTON aria-describedby=format!("{STATUS_ID} {WAIT_ID}") disabled=move || wait.get().is_some() title=move || wait.get() on:click=move |_| refresh.run(())>{REFRESH}</button>
            </div>
            <div id=STATUS_ID>{move || match load.get() {
                Load::Loading => view! { <p role="status" class="mt-3 text-label-sm text-rambutan-400">{APPS_LOADING}</p> }.into_any(),
                state @ Load::Failed(_) => view! { <p role="alert" class="mt-3 text-label-sm text-lychee-300">{state.message()}</p> }.into_any(),
                state @ (Load::ServingOld | Load::OwnerOnly) => view! { <p role="status" data-apps-notice="" class="mt-3 text-label-sm text-durian-300">{state.message()}</p> }.into_any(),
                Load::Ready => ().into_any(),
            }}</div>
            {move || (page.ready()).then(|| page.grants_off()).flatten().map(|reason| view! { <p id=GRANTS_OFF_ID role="status" data-apps-grants-off="" class="mt-3 text-label-sm text-durian-300">{reason}</p> })}
            // Always drawn, so every control's `aria-describedby` names an
            // element; empty while nothing runs. A load says so above.
            <p id=WAIT_ID role="status" class="mt-2 text-label-sm text-rambutan-400 empty:hidden">{move || wait.get().filter(|_| load.with(|load| *load != Load::Loading))}</p>
            {move || notice.get().map(|notice| {
                let (text, warns) = match notice {
                    Notice::Done(text) => (text, false),
                    Notice::MayHaveChanged => (VaultNotice::MayHaveChanged.message().to_owned(), true),
                };
                view! { <p role="status" data-apps-page-notice="" class=if warns { "mt-2 text-label-sm text-durian-300" } else { "mt-2 text-label-sm text-pandan-300" }>{text}</p> }
            })}
            <section data-apps-mail="" aria-labelledby="apps-mail-title" class="mt-4 min-w-0 rounded-grape bg-longan-950/40 p-3 inset-ring inset-ring-longan-700/50">
                <h3 id="apps-mail-title" class="text-label text-rambutan-100">{MAIL_GROUP}</h3>
                <p class=NOTE>{MAIL_HINT}</p>
                {move || (page.ready() && listing.with(|listing| listing.mailboxes.is_empty())).then(|| view! { <p role="status" class=NOTE>{NO_MAILBOXES}</p> })}
                <div class="divide-y divide-longan-800">
                    <For
                        each=mailboxes
                        key=|key| key.clone()
                        children=move |(_, id)| view! { <MailboxRow id=id page=page /> }
                    />
                </div>
            </section>
            <div class="mt-4 divide-y divide-longan-800">
                {connectors.iter().map(|app| view! { <AppSection app=app page=page /> }).collect_view()}
            </div>
        </div>
    }
    .into_any()
}

#[component]
fn MailboxRow(id: String, page: Page) -> impl IntoView {
    let mailbox = page.listing.with_untracked(|listing| {
        listing
            .mailboxes
            .iter()
            .find(|mailbox| mailbox.mailbox_id == id)
            .cloned()
    });
    let (provider, account) = mailbox.map_or_else(Default::default, |mailbox| {
        (mailbox.provider.name(), review_text(&mailbox.account))
    });
    let heading = unique_id("apps-mailbox");
    let (row_id, labelled_by) = (id.clone(), heading.clone());
    view! {
        <article data-apps-mailbox=row_id aria-labelledby=labelled_by class="min-w-0 py-3">
            <h4 id=heading.clone() class="break-words text-label text-rambutan-100">{account}</h4>
            <p class=NOTE>{provider}</p>
            <GrantList grant=AppGrant::Mailbox { mailbox_id: id } heading=heading page=page />
        </article>
    }
    .into_any()
}

#[component]
fn AppSection(app: &'static AppConnector, page: Page) -> impl IntoView {
    let connect_ref = NodeRef::<html::Button>::new();
    let heading = format!("apps-app-{}", app.id);
    let connecting = Signal::derive(move || page.open.with(|open| *open == Open::Connect(app.id)));
    let accounts = move || {
        let generation = page.generation.get();
        page.listing.with(|listing| {
            listing
                .accounts
                .iter()
                .filter(|account| account.app == app.id)
                .map(|account| (generation, account.id.clone()))
                .collect::<Vec<_>>()
        })
    };
    // Cancel runs from inside the form; a connect's answer can land anywhere.
    let close = Callback::new(move |_: ()| {
        let held = focus_on_page(page.page_ref);
        page.open.try_set(Open::None);
        if held {
            focus_after_render(connect_ref);
        }
    });
    let connected = Callback::new(move |account: AppAccount| {
        let line = connected_as(&account.account);
        page.listing.try_update(|listing| {
            listing.accounts.retain(|row| row.id != account.id);
            listing.accounts.push(account);
        });
        page.notice.try_set(Some(Notice::Done(line)));
        page.vault_changed();
    });
    view! {
        <section data-apps-app=app.id aria-labelledby=heading class="min-w-0 py-4">
            <div class="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                <div class="min-w-0 flex-1">
                    <h3 id=heading.clone() class="text-label text-rambutan-100">{app.name}</h3>
                    <p class=NOTE>{app.scopes_hint}</p>
                </div>
                <button type="button" node_ref=connect_ref data-apps-connect="" class=BUTTON aria-expanded=move || if connecting.get() { "true" } else { "false" } aria-describedby=format!("{STATUS_ID} {WAIT_ID}") disabled=move || page.blocked().is_some() title=move || page.blocked() on:click=move |_| { page.notice.set(None); page.open.set(Open::Connect(app.id)); }>{CONNECT}</button>
            </div>
            <Show when=move || connecting.get()>
                <ConnectFormView app=app bruhs=Signal::derive(move || page.listing.with(|listing| listing.bruhs.clone())) busy=page.busy wait=page.wait on_connected=connected on_close=close on_unanswered=Callback::new(move |_: ()| page.unanswered()) wait_id=WAIT_ID />
            </Show>
            <div class="divide-y divide-longan-800">
                <For
                    each=accounts
                    key=|key| key.clone()
                    children=move |(_, id)| view! { <AccountRow id=id app=app page=page connect_ref=connect_ref /> }
                />
            </div>
        </section>
    }
    .into_any()
}

#[component]
fn AccountRow(
    id: String,
    app: &'static AppConnector,
    page: Page,
    /// The app's Connect, which takes focus once the account is gone.
    connect_ref: NodeRef<html::Button>,
) -> impl IntoView {
    let id = StoredValue::new(id);
    let account = Memo::new(move |_| {
        page.listing.with(|listing| {
            id.with_value(|id| listing.accounts.iter().find(|row| &row.id == id).cloned())
        })
    });
    let shared = account.with_untracked(|account| {
        account
            .as_ref()
            .is_some_and(|account| account.scope == CredentialScope::Global)
    });
    let name = account.with_untracked(|account| {
        account
            .as_ref()
            .map(|account| account.name.clone())
            .unwrap_or_default()
    });
    let replacing = Signal::derive(move || {
        page.open
            .with(|open| id.with_value(|id| matches!(open, Open::Replace(open) if open == id)))
    });
    let asking = RwSignal::new(false);
    let tested = RwSignal::new(None::<(bool, String)>);
    let error = RwSignal::new(None::<String>);
    let replace_ref = NodeRef::<html::Button>::new();
    let disconnect_ref = NodeRef::<html::Button>::new();
    let keep_ref = NodeRef::<html::Button>::new();
    let heading = unique_id("apps-account");
    let question_id = unique_id("apps-disconnect-question");
    let confirm = disconnect_confirm(&review_text(&name));
    let disabled = move || page.wait.get().is_some();
    // Runs after an await: the row may be gone by now. The server took the
    // account out of every bruh's grants with it.
    let forget = move || {
        let Some(id) = id.try_get_value() else {
            return;
        };
        let held = focus_on_page(page.page_ref);
        page.open.try_update(|open| {
            if matches!(open, Open::Replace(open) if *open == id) {
                *open = Open::None;
            }
        });
        page.listing.try_update(|listing| {
            listing.accounts.retain(|account| account.id != id);
            for bruh in &mut listing.bruhs {
                bruh.apps.retain(|grant| *grant != id);
            }
        });
        page.vault_changed();
        if held {
            focus_after_render(connect_ref);
        }
    };
    let test = move |_| {
        if page.busy.get_untracked() != Busy::Idle {
            return;
        }
        page.busy.set(Busy::Checking);
        page.notice.set(None);
        tested.set(None);
        error.set(None);
        let id = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::test_app_account(&id).await {
                // A refused token answers 200 with `ok: false`.
                Ok(result) => {
                    if result.ok {
                        page.listing.try_update(|listing| {
                            if let Some(account) =
                                listing.accounts.iter_mut().find(|account| account.id == id)
                            {
                                account.attention = None;
                            }
                        });
                    }
                    tested.try_set(Some((result.ok, result.message)));
                }
                Err(err) if err.is_status(404) => forget(),
                Err(err) => {
                    error.try_set(Some(crate::api::vault_message(&err)));
                }
            }
            page.busy.try_set(Busy::Idle);
        });
    };
    let disconnect = move |_| {
        if page.busy.get_untracked() != Busy::Idle {
            return;
        }
        page.busy.set(Busy::Changing);
        page.notice.set(None);
        error.set(None);
        let id = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::delete_vault_entry(&id).await;
            page.busy.try_set(Busy::Idle);
            match result {
                Ok(()) => forget(),
                Err(err) if err.is_status(404) => forget(),
                Err(err) if crate::api::vault_failure(&err).may_have_landed() => page.unanswered(),
                Err(err) => {
                    error.try_set(Some(crate::api::vault_message(&err)));
                }
            }
        });
    };
    let keep = move || {
        asking.set(false);
        focus_after_render(disconnect_ref);
    };
    let close_replace = Callback::new(move |_: ()| {
        let held = focus_on_page(page.page_ref);
        page.open.try_set(Open::None);
        if held {
            focus_after_render(replace_ref);
        }
    });
    let replaced = Callback::new(move |replaced: AppAccount| {
        let line = connected_as(&replaced.account);
        page.listing.try_update(|listing| {
            if let Some(row) = listing
                .accounts
                .iter_mut()
                .find(|row| row.id == replaced.id)
            {
                *row = replaced;
            }
        });
        tested.try_set(None);
        page.notice.try_set(Some(Notice::Done(line)));
        page.vault_changed();
    });
    view! {
        <article data-apps-account=id.get_value() aria-labelledby=heading class="min-w-0 py-3">
            <div class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                {
                    let heading = heading.clone();
                    move || account.get().map(|account| {
                        let stuck = account.state == SavedCredentialState::NeedsAttention;
                        let attention = account.attention.filter(|_| tested.with(Option::is_none)).map(|attention| attention.message());
                        let scope = page.listing.with(|listing| app_scope_line(&account.scope, &listing.bruhs));
                        view! {
                            <div class="min-w-0 flex-1">
                                <h4 id=heading.clone() class="break-words text-label text-rambutan-100">{review_text(&account.name)}</h4>
                                <p class="mt-1 flex flex-wrap gap-x-3 text-label-sm text-rambutan-300"><span class="min-w-0 break-all">{review_text(&account.account)}</span><span class="min-w-0 break-words">{scope}</span></p>
                                <p class=if stuck { "mt-1 text-label-sm text-durian-300" } else { NOTE }>{if stuck { STATE_NEEDS_ATTENTION } else { STATE_SAVED }}</p>
                                {attention.map(|attention| view! { <p data-apps-attention="" class="mt-1 text-label-sm text-durian-300">{attention}</p> })}
                            </div>
                        }
                    })
                }
                <div class="flex flex-wrap gap-2">
                    <button type="button" data-apps-test="" class=BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || page.wait.get() on:click=test>{TEST}</button>
                    <button type="button" node_ref=replace_ref data-apps-replace="" class=BUTTON aria-expanded=move || if replacing.get() { "true" } else { "false" } aria-describedby=WAIT_ID disabled=disabled title=move || page.wait.get() on:click=move |_| { page.notice.set(None); asking.set(false); tested.set(None); page.open.set(Open::Replace(id.get_value())); }>{REPLACE_TOKEN}</button>
                    <Show when=move || !asking.get()>
                        <button type="button" node_ref=disconnect_ref data-apps-disconnect="" class=BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || page.wait.get() on:click=move |_| { error.set(None); asking.set(true); focus_after_render(keep_ref); }>{DISCONNECT}</button>
                    </Show>
                </div>
            </div>
            <Show when=move || asking.get()>
                <div role="group" aria-labelledby=question_id.clone() class="mt-3 space-y-2 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50" on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { event.prevent_default(); event.stop_propagation(); keep(); }>
                    <p id=question_id.clone() class="text-body-sm text-rambutan-200">{confirm.clone()}</p>
                    <div class="flex flex-wrap items-center gap-2">
                        <button type="button" data-apps-confirm="" class=DANGER_BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || page.wait.get() on:click=disconnect>{DISCONNECT}</button>
                        <button type="button" node_ref=keep_ref class=BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || page.wait.get() on:click=move |_| keep()>{KEEP}</button>
                    </div>
                </div>
            </Show>
            <Show when=move || replacing.get()>
                <ReplaceFormView app=app id=id.get_value() busy=page.busy wait=page.wait on_replaced=replaced on_close=close_replace on_unanswered=Callback::new(move |_: ()| page.unanswered()) wait_id=WAIT_ID />
            </Show>
            {move || tested.get().map(|(ok, message)| view! { <p role="status" data-apps-tested="" class=if ok { "mt-2 text-label-sm text-pandan-300" } else { "mt-2 text-label-sm text-durian-300" }>{message}</p> })}
            {move || error.get().map(|message| view! { <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}</p> })}
            // A bruh's own account is its own; only shared ones are granted.
            {shared.then(|| view! { <GrantList grant=AppGrant::Token { entry_id: id.get_value() } heading=heading.clone() page=page /> })}
        </article>
    }
    .into_any()
}

/// One switch per bruh for `grant`. A flip sends `SetAppGrant` at once and
/// reloads the listing; a refusal snaps the switch back and says why.
#[component]
fn GrantList(grant: AppGrant, heading: String, page: Page) -> impl IntoView {
    let label_id = unique_id("apps-grants");
    let error = RwSignal::new(None::<String>);
    let grant = StoredValue::new(grant);
    let bruhs = move || {
        page.listing.with(|listing| {
            listing
                .bruhs
                .iter()
                .map(|bruh| bruh.agent_id.clone())
                .collect::<Vec<_>>()
        })
    };
    view! {
        <div role="group" aria-labelledby=format!("{heading} {label_id}") class="mt-2 min-w-0">
            <p id=label_id.clone() class="text-label-sm text-rambutan-300">{GRANTS_LABEL}</p>
            {move || (page.ready() && page.listing.with(|listing| listing.bruhs.is_empty())).then(|| view! { <p role="status" class=NOTE>{NO_BRUHS_FOR_APPS}</p> })}
            <ul class="min-w-0">
                <For
                    each=bruhs
                    key=|id| id.clone()
                    children=move |agent_id| view! { <GrantSwitch agent_id=agent_id grant=grant page=page error=error /> }
                />
            </ul>
            {move || error.get().map(|message| view! { <p role="alert" class="mt-1 text-label-sm text-lychee-300">{message}</p> })}
        </div>
    }
    .into_any()
}

#[component]
fn GrantSwitch(
    agent_id: String,
    grant: StoredValue<AppGrant>,
    page: Page,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let agent_id = StoredValue::new(agent_id);
    let bruh = Memo::new(move |_| {
        page.listing.with(|listing| {
            agent_id.with_value(|id| {
                listing
                    .bruhs
                    .iter()
                    .find(|bruh| &bruh.agent_id == id)
                    .cloned()
            })
        })
    });
    // The flip being sent, shown until the listing answers.
    let pending = RwSignal::new(None::<bool>);
    let checked = Signal::derive(move || {
        pending.get().unwrap_or_else(|| {
            bruh.with(|bruh| {
                bruh.as_ref()
                    .is_some_and(|bruh| grant.with_value(|grant| grant_switch_on(bruh, grant)))
            })
        })
    });
    let reason = Signal::derive(move || page.wait.get().or_else(|| page.grants_off()));
    let reason_id = unique_id("apps-grant-reason");
    let target = grant.with_value(|grant| match grant {
        AppGrant::Token { entry_id } => entry_id.clone(),
        AppGrant::Mailbox { mailbox_id } => mailbox_id.clone(),
    });
    let selector = StoredValue::new(format!("{target}:{}", agent_id.get_value()));
    let name = Signal::derive(move || {
        bruh.with(|bruh| {
            bruh.as_ref()
                .map(|bruh| review_text(&bruh.name))
                .unwrap_or_default()
        })
    });
    let needs_ask = Signal::derive(move || {
        bruh.with(|bruh| bruh.as_ref().is_some_and(|bruh| bruh.unavailable.is_some()))
    });
    let described = {
        let reason_id = reason_id.clone();
        move || {
            let mut ids = vec![WAIT_ID.to_owned()];
            if page.grants_off().is_some() {
                ids.push(GRANTS_OFF_ID.to_owned());
            }
            if needs_ask.get() {
                ids.push(reason_id.clone());
            }
            ids.join(" ")
        }
    };
    let flip = Callback::new(move |on: bool| {
        if page.busy.get_untracked() != Busy::Idle || page.grants_off().is_some() {
            return;
        }
        let (target_bruh, target_grant) = (agent_id.get_value(), grant.get_value());
        page.busy.set(Busy::Changing);
        page.notice.set(None);
        error.set(None);
        pending.set(Some(on));
        let focus = format!("[data-apps-grant=\"{}\"]", selector.get_value());
        wasm_bindgen_futures::spawn_local(async move {
            let sent = crate::api::set_app_grant(&target_bruh, &target_grant, on).await;
            let next = match &sent {
                Ok(_) => crate::api::apps_listing().await.ok(),
                Err(_) => None,
            };
            page.busy.try_set(Busy::Idle);
            pending.try_set(None);
            // The switch was disabled while it waited, which drops focus.
            if focus_dropped() {
                focus_selector_after_render(focus);
            }
            match sent {
                // The listing shows what the server holds; if it can't be
                // read, the grant shows as sent.
                Ok(_) => {
                    page.listing.try_update(|listing| match next {
                        Some(next) => *listing = next,
                        None => {
                            if let (Some(agent_id), Some(grant)) =
                                (agent_id.try_get_value(), grant.try_get_value())
                            {
                                set_listing_grant(listing, &agent_id, &grant, on);
                            }
                        }
                    });
                }
                Err(err) if crate::api::vault_failure(&err).may_have_landed() => page.unanswered(),
                Err(err) => {
                    error.try_set(Some(crate::api::vault_message(&err)));
                }
            }
        });
    });
    view! {
        <li class="flex min-h-control min-w-0 items-center justify-between gap-3">
            <div class="min-w-0">
                <span class="break-words text-label-sm text-rambutan-200">{move || name.get()}</span>
                {
                    move || needs_ask.get().then(|| view! { <p id=reason_id.clone() data-apps-need-ask="" class="text-label-sm text-durian-300">{APPS_NEED_ASK}</p> })
                }
            </div>
            // The label makes the whole 44px touch target press the switch.
            <label class="bruhs-control inline-flex shrink-0 items-center justify-center">
                <Switch
                    label=name.get_untracked()
                    checked=checked
                    disabled=Signal::derive(move || reason.get().is_some())
                    on_change=flip
                    attr:data-apps-grant=selector.get_value()
                    attr:title=move || reason.get()
                    attr:aria-describedby=described
                />
            </label>
        </li>
    }
    .into_any()
}
