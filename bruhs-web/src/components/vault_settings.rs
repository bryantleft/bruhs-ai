//! Settings, Vault: the service connections, then every vault entry's
//! metadata, and one inline form that creates or changes an entry.
//!
//! Values are write-only. Their inputs are never filled, are cleared on
//! submit, cancel and unmount, and stop key events from reaching the app's
//! shortcuts. Nothing about the vault reaches IndexedDB or any browser
//! storage: the page holds the listing only while it is open. Every rule and
//! line of copy is `bruhs_models::vault`'s, so the desktop page matches.

use std::sync::atomic::{AtomicU64, Ordering};

use bruhs_models::crew::ServiceNotice;
use bruhs_models::vault::{
    bruh_choice_label, delete_entry_confirm, fixed_hint_after, form_create, form_fields, form_for,
    form_update, kind_label, matches_search, option_for, option_key, row_label, vault_row,
    wait_reason, FormField, FormScope, LoginFields, LoginMeta, VaultBruh, VaultBusy as Busy,
    VaultCreate, VaultEntry, VaultForm, VaultKind, VaultListing, VaultUpdate, CANCEL, CHECK,
    CHOOSE_BRUH, DELETE, EDIT, EMPTY, ENTRY_UPDATED, FIELD_BRUH, FIELD_KIND, FIELD_NAME,
    FIELD_PASSWORD, FIELD_PASSWORD_REF, FIELD_PROJECT, FIELD_REFERENCE, FIELD_REMOVE_TOTP,
    FIELD_SIGN_IN, FIELD_STORED_IN, FIELD_TOTP, FIELD_TOTP_REF, FIELD_USED_BY, FIELD_USERNAME,
    FIELD_USERNAME_REF, FIELD_VALUE, FIXED_HINT, KEEP, KEEP_HINT, KIND_OPTIONS as KINDS, LOADING,
    LOAD_RETRY, NEW_ENTRY, NO_BRUHS, NO_MATCH, OPEN_FROM_OWN_ADDRESS, OP_MISSING_HINT,
    REFERENCE_HINT, REFRESH, SAVE_ENTRY, SCOPE_OPTIONS as SCOPES, SEARCH_PLACEHOLDER,
    SOURCE_OPTIONS as SOURCES, STATE_NEEDS_ATTENTION, TOTP_HINT,
};
use leptos::{html, prelude::*};
use wasm_bindgen::JsCast;

const BUTTON: &str = "bruhs-control min-h-control shrink-0 cursor-pointer rounded-seed px-3 text-label-sm text-rambutan-200 inset-ring inset-ring-longan-700 transition-colors duration-150 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50";
/// The confirm Delete: `BUTTON` with lychee text and ring in place of the
/// neutral ones, since stacking both would leave the stylesheet's order to
/// pick the color.
const DANGER_BUTTON: &str = "bruhs-control min-h-control shrink-0 cursor-pointer rounded-seed px-3 text-label-sm text-lychee-200 inset-ring inset-ring-lychee-700 transition-colors duration-150 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50";
const LABEL: &str = "block min-w-0 text-label-sm text-rambutan-300";
const FIELD: &str = "bruhs-control mt-1 block min-h-control w-full min-w-0 rounded-seed bg-longan-950 px-3 text-body-sm text-rambutan-100 inset-ring inset-ring-longan-700 focus:outline focus:outline-2 focus:outline-persimmon-400 read-only:text-rambutan-400";
const HINT: &str = "mt-1 text-label-sm text-rambutan-500";
/// The page's line that says values are never shown; edited value fields
/// point at it.
const KEEP_HINT_ID: &str = "vault-keep-hint";
/// Why a disabled button waits: the listing's state or a running change.
const STATUS_ID: &str = "vault-status";
const WAIT_ID: &str = "vault-wait";

/// A DOM id prefix no other row or form on the page uses.
fn unique_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Focus `target` once the view has drawn it.
fn focus_after_render(target: NodeRef<html::Button>) {
    request_animation_frame(move || {
        if let Some(Some(button)) = target.try_get_untracked() {
            let _ = button.focus();
        }
    });
}

#[derive(Clone, PartialEq)]
enum Load {
    Loading,
    Ready,
    Failed(String),
    /// An older server: no `/api/vault`.
    ServingOld,
    /// A page on another address (ruling F26): the listing is owner-only.
    OwnerOnly,
}

/// Which form is open: none, a new entry's, or one entry's edit.
#[derive(Clone, PartialEq)]
enum Open {
    None,
    New,
    Entry(String),
}

#[component]
pub fn VaultSettings() -> impl IntoView {
    let listing = RwSignal::new(VaultListing::default());
    let load = RwSignal::new(Load::Loading);
    let generation = RwSignal::new(0u64);
    let busy = RwSignal::new(Busy::Idle);
    let query = RwSignal::new(String::new());
    let open = RwSignal::new(Open::None);
    let notice = RwSignal::new(None::<&'static str>);
    let new_ref = NodeRef::<html::Button>::new();
    let wait =
        Signal::derive(move || load.with(|load| wait_reason(busy.get(), *load == Load::Loading)));
    // New entry also waits for a listing it can add to.
    let new_blocked = Signal::derive(move || {
        wait.get().is_some() || load.with(|load| !matches!(load, Load::Ready))
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
            let (next, state) = match crate::api::vault_listing().await {
                Ok(next) => (next, Load::Ready),
                Err(error) if error.is_status(404) => (VaultListing::default(), Load::ServingOld),
                Err(error) if error.is_status(403) => (VaultListing::default(), Load::OwnerOnly),
                Err(error) => (VaultListing::default(), Load::Failed(error.to_string())),
            };
            listing.try_set(next);
            generation.try_update(|generation| *generation = generation.wrapping_add(1));
            load.try_set(state);
            busy.try_set(Busy::Idle);
        });
    });
    Effect::new(move |_| refresh.run(()));
    let saved = Callback::new(move |entry: VaultEntry| {
        listing.try_update(|listing| {
            match listing.entries.iter_mut().find(|row| row.id == entry.id) {
                Some(row) => *row = entry,
                None => listing.entries.push(entry),
            }
        });
        notice.try_set(Some(ENTRY_UPDATED));
    });
    let close_new = Callback::new(move |_: ()| {
        open.try_set(Open::None);
        focus_after_render(new_ref);
    });
    let bruhs = Signal::derive(move || listing.with(|listing| listing.bruhs.clone()));
    let visible = move || {
        let search = query.get();
        let generation = generation.get();
        listing.with(|listing| {
            listing
                .entries
                .iter()
                .filter(|entry| matches_search(entry, &listing.bruhs, &search))
                .map(|entry| (generation, entry.id.clone()))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <div class="p-4 text-body-sm text-rambutan-200">
            <super::service_connections::ServiceConnections />
            <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
                <input type="search" aria-label=SEARCH_PLACEHOLDER placeholder=SEARCH_PLACEHOLDER autocomplete="off" spellcheck="false" class="bruhs-control min-h-control w-full min-w-0 rounded-seed bg-longan-950 px-3 text-body-sm inset-ring inset-ring-longan-700 focus:outline focus:outline-2 focus:outline-persimmon-400 sm:flex-1" on:input=move |event| query.set(event_target_value(&event)) />
                <div class="flex flex-wrap gap-2">
                    <button type="button" node_ref=new_ref data-vault-new="" class=BUTTON aria-expanded=move || if open.with(|open| *open == Open::New) { "true" } else { "false" } aria-describedby=format!("{STATUS_ID} {WAIT_ID}") disabled=move || new_blocked.get() title=move || wait.get() on:click=move |_| { notice.set(None); open.set(Open::New); }>{NEW_ENTRY}</button>
                    <button type="button" data-vault-refresh="" class=BUTTON aria-describedby=format!("{STATUS_ID} {WAIT_ID}") disabled=move || wait.get().is_some() title=move || wait.get() on:click=move |_| refresh.run(())>{REFRESH}</button>
                </div>
            </div>
            <p id=KEEP_HINT_ID class="mt-2 text-label-sm text-rambutan-400">{KEEP_HINT}</p>
            {move || (load.with(|load| *load == Load::Ready) && !listing.with(|listing| listing.one_password)).then(|| view! { <p data-vault-op-missing="" class="mt-1 text-label-sm text-rambutan-400">{OP_MISSING_HINT}</p> })}
            <div id=STATUS_ID>{move || match load.get() {
                Load::Loading => view! { <p role="status" class="mt-3 text-label-sm text-rambutan-400">{LOADING}</p> }.into_any(),
                Load::Failed(message) => view! { <p role="alert" class="mt-3 text-label-sm text-lychee-300">{format!("{message} {LOAD_RETRY}")}</p> }.into_any(),
                Load::ServingOld => view! { <p role="status" data-vault-notice="" class="mt-3 text-label-sm text-durian-300">{ServiceNotice::ServingOld.message()}</p> }.into_any(),
                Load::OwnerOnly => view! { <p role="status" data-vault-notice="" class="mt-3 text-label-sm text-durian-300">{OPEN_FROM_OWN_ADDRESS}</p> }.into_any(),
                Load::Ready if listing.with(|listing| listing.entries.is_empty()) => view! { <p role="status" class="mt-3 text-label-sm text-rambutan-400">{EMPTY}</p> }.into_any(),
                Load::Ready => ().into_any(),
            }}</div>
            // Always drawn, so every button's `aria-describedby` names an
            // element; empty while nothing runs. A load says so above.
            <p id=WAIT_ID role="status" class="mt-2 text-label-sm text-rambutan-400 empty:hidden">{move || wait.get().filter(|_| load.with(|load| *load != Load::Loading))}</p>
            {move || notice.get().map(|message| view! { <p role="status" class="mt-2 text-label-sm text-pandan-300">{message}</p> })}
            <Show when=move || open.with(|open| *open == Open::New)>
                <VaultFormView entry=None bruhs=bruhs busy=busy wait=wait on_saved=saved on_close=close_new />
            </Show>
            <div class="mt-3 divide-y divide-longan-800">
                <For
                    each=visible
                    key=|key| key.clone()
                    children=move |(_, id)| view! { <VaultRowView id=id listing=listing busy=busy wait=wait open=open notice=notice on_saved=saved new_ref=new_ref /> }
                />
                {move || (!listing.with(|listing| listing.entries.is_empty()) && visible().is_empty()).then(|| view! { <p role="status" class="py-3 text-label-sm text-rambutan-400">{NO_MATCH}</p> })}
            </div>
        </div>
    }
}

#[component]
fn VaultRowView(
    id: String,
    listing: RwSignal<VaultListing>,
    busy: RwSignal<Busy>,
    wait: Signal<Option<&'static str>>,
    open: RwSignal<Open>,
    notice: RwSignal<Option<&'static str>>,
    on_saved: Callback<VaultEntry>,
    new_ref: NodeRef<html::Button>,
) -> impl IntoView {
    let id = StoredValue::new(id);
    let entry = Memo::new(move |_| {
        listing.with(|listing| {
            id.with_value(|id| {
                listing
                    .entries
                    .iter()
                    .find(|entry| &entry.id == id)
                    .cloned()
            })
        })
    });
    let row = Memo::new(move |_| {
        listing.with(|listing| {
            entry.with(|entry| entry.as_ref().map(|e| vault_row(e, &listing.bruhs)))
        })
    });
    // The stored name: the models helpers make it safe to show.
    let name = entry.with_untracked(|entry| {
        entry
            .as_ref()
            .map(|entry| entry.name.clone())
            .unwrap_or_default()
    });
    let editing = Signal::derive(move || {
        open.with(|open| id.with_value(|id| matches!(open, Open::Entry(open) if open == id)))
    });
    let asking = RwSignal::new(false);
    let checked = RwSignal::new(None::<(bool, String)>);
    let error = RwSignal::new(None::<String>);
    let edit_ref = NodeRef::<html::Button>::new();
    let delete_ref = NodeRef::<html::Button>::new();
    let keep_ref = NodeRef::<html::Button>::new();
    let question_id = unique_id("vault-delete-question");
    let label = row_label(&name);
    let confirm = delete_entry_confirm(&name);
    let forget = move || {
        // Runs after an await: the row may be gone by now.
        let Some(id) = id.try_get_value() else {
            return;
        };
        open.try_update(|open| {
            if matches!(open, Open::Entry(open) if *open == id) {
                *open = Open::None;
            }
        });
        listing.try_update(|listing| listing.entries.retain(|entry| entry.id != id));
        focus_after_render(new_ref);
    };
    let check = move |_| {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        busy.set(Busy::Checking);
        notice.set(None);
        checked.set(None);
        error.set(None);
        let id = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::check_vault_entry(&id).await {
                Ok(check) => {
                    if check.ok {
                        listing.try_update(|listing| {
                            if let Some(entry) =
                                listing.entries.iter_mut().find(|entry| entry.id == id)
                            {
                                entry.attention = None;
                            }
                        });
                    }
                    checked.try_set(Some((check.ok, check.message)));
                }
                Err(err) if err.is_status(404) => forget(),
                Err(err) => {
                    error.try_set(Some(err.to_string()));
                }
            }
            busy.try_set(Busy::Idle);
        });
    };
    let delete = move |_| {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        busy.set(Busy::Changing);
        notice.set(None);
        error.set(None);
        let id = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::delete_vault_entry(&id).await {
                Ok(()) => forget(),
                Err(err) if err.is_status(404) => forget(),
                Err(err) => {
                    error.try_set(Some(err.to_string()));
                }
            }
            busy.try_set(Busy::Idle);
        });
    };
    let keep = move || {
        asking.set(false);
        focus_after_render(delete_ref);
    };
    // The form saves through the page's `on_saved`, which outlives this
    // row (a search can filter it out mid-save). Closing drops what an
    // earlier Check said, so the row shows the entry's own state.
    let close = Callback::new(move |_: ()| {
        checked.try_set(None);
        open.try_set(Open::None);
        focus_after_render(edit_ref);
    });
    let disabled = move || wait.get().is_some();
    view! {
        <article data-vault-row=id.get_value() aria-label=label class="min-w-0 py-3">
            <div class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                {move || row.get().map(|row| {
                    let stuck = row.state == STATE_NEEDS_ATTENTION;
                    let attention = row.attention.filter(|_| checked.with(Option::is_none));
                    view! {
                        <div class="min-w-0 flex-1">
                            <h3 class="break-words text-label text-rambutan-100">{row.name}</h3>
                            <p class="mt-1 flex flex-wrap gap-x-3 text-label-sm text-rambutan-300"><span>{row.kind}</span><span class="min-w-0 break-all">{row.scope}</span></p>
                            {row.detail.map(|detail| view! { <p class="mt-1 break-all text-label-sm text-rambutan-400">{detail}</p> })}
                            <p class=if stuck { "mt-1 text-label-sm text-durian-300" } else { "mt-1 text-label-sm text-rambutan-400" }>{row.state}</p>
                            {attention.map(|attention| view! { <p data-vault-attention="" class="mt-1 text-label-sm text-durian-300">{attention}</p> })}
                        </div>
                    }
                })}
                <div class="flex flex-wrap gap-2">
                    <button type="button" node_ref=edit_ref data-vault-edit="" class=BUTTON aria-expanded=move || if editing.get() { "true" } else { "false" } aria-describedby=WAIT_ID disabled=disabled title=move || wait.get() on:click=move |_| { notice.set(None); asking.set(false); checked.set(None); open.set(Open::Entry(id.get_value())); }>{EDIT}</button>
                    <button type="button" data-vault-check="" class=BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || wait.get() on:click=check>{CHECK}</button>
                    <Show when=move || !asking.get()>
                        <button type="button" node_ref=delete_ref data-vault-delete="" class=BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || wait.get() on:click=move |_| { error.set(None); asking.set(true); focus_after_render(keep_ref); }>{DELETE}</button>
                    </Show>
                </div>
            </div>
            <Show when=move || asking.get()>
                <div role="group" aria-labelledby=question_id.clone() class="mt-3 space-y-2 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50" on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { event.prevent_default(); event.stop_propagation(); keep(); }>
                    <p id=question_id.clone() class="text-body-sm text-rambutan-200">{confirm.clone()}</p>
                    <div class="flex flex-wrap items-center gap-2">
                        <button type="button" data-vault-confirm="" class=DANGER_BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || wait.get() on:click=delete>{DELETE}</button>
                        <button type="button" node_ref=keep_ref class=BUTTON aria-describedby=WAIT_ID disabled=disabled title=move || wait.get() on:click=move |_| keep()>{KEEP}</button>
                    </div>
                </div>
            </Show>
            <Show when=move || editing.get()>
                {move || entry.get_untracked().map(|entry| view! { <VaultFormView entry=Some(entry) bruhs=Signal::derive(move || listing.with(|listing| listing.bruhs.clone())) busy=busy wait=wait on_saved=on_saved on_close=close /> })}
            </Show>
            {move || checked.get().map(|(ok, message)| view! { <p role="status" data-vault-checked="" class=if ok { "mt-2 text-label-sm text-pandan-300" } else { "mt-2 text-label-sm text-durian-300" }>{message}</p> })}
            {move || error.get().map(|message| view! { <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}</p> })}
        </article>
    }
}

/// A create or an update, ready to send. Holds values: neither is `Clone`.
enum Request {
    Create(VaultCreate),
    Update(String, VaultUpdate),
}

/// A value field: a password input that is never filled, is cleared when
/// it leaves the page, and keeps its keys from the app's shortcuts.
#[component]
fn SecretInput(
    label: &'static str,
    node: NodeRef<html::Input>,
    described_by: Option<String>,
    on_escape: Callback<()>,
) -> impl IntoView {
    on_cleanup(move || {
        if let Some(Some(input)) = node.try_get_untracked() {
            input.set_value("");
        }
    });
    view! {
        <label class=LABEL>{label}
            <input node_ref=node type="password" autocomplete="off" autocapitalize="none" spellcheck="false" aria-describedby=described_by class=FIELD on:keydown=move |event: web_sys::KeyboardEvent| {
                if event.key() == "Escape" {
                    event.prevent_default();
                    on_escape.run(());
                }
                event.stop_propagation();
            } />
        </label>
    }
}

/// A metadata text field bound to `value`.
fn text_field(
    label: &'static str,
    value: RwSignal<String>,
    described_by: Option<String>,
    kind: &'static str,
) -> impl IntoView {
    view! {
        <label class=LABEL>{label}
            <input type=kind autocomplete="off" autocapitalize="none" spellcheck="false" aria-describedby=described_by class=FIELD prop:value=move || value.get() on:input=move |event| value.set(event_target_value(&event)) />
        </label>
    }
}

/// A fixed field on an edit: read only, saying why.
fn fixed_field(label: &'static str, value: String, hint_id: String) -> impl IntoView {
    view! {
        <label class=LABEL>{label}
            <input type="text" readonly=true aria-describedby=hint_id class=FIELD prop:value=value />
        </label>
    }
}

/// A choice among `options`. Its label names it through `for`: a label
/// around a select would add the chosen option to the select's name.
fn select_field<T: Copy + PartialEq + Default + Send + Sync + 'static>(
    label: &'static str,
    id: String,
    options: &'static [(T, &'static str, &'static str)],
    value: RwSignal<T>,
) -> impl IntoView {
    view! {
        <div>
            <label for=id.clone() class=LABEL>{label}</label>
            <select id=id class=FIELD prop:value=move || option_key(options, value.get()) on:change=move |event| value.set(option_for(options, &event_target_value(&event)))>
                {options.iter().map(|(choice, key, text)| { let choice = *choice; view! { <option value=*key selected=move || value.get() == choice>{*text}</option> } }).collect_view()}
            </select>
        </div>
    }
}

/// What an edit shows in a fixed field: the stored kind, name, scope line
/// or sign-in page (ruling F3), or `None` for a field the owner can change.
fn fixed_text(field: FormField, start: &VaultForm, scope_line: &str) -> Option<String> {
    match field {
        FormField::Kind => Some(kind_label(start.kind).into()),
        FormField::Name => Some(start.name.clone()),
        FormField::UsedBy => Some(scope_line.into()),
        FormField::SignIn => Some(start.url.clone()),
        FormField::StoredIn
        | FormField::Username
        | FormField::Password
        | FormField::Totp
        | FormField::UsernameRef
        | FormField::PasswordRef
        | FormField::TotpRef
        | FormField::Value
        | FormField::Reference => None,
    }
}

/// The inline form: a new entry's when `entry` is `None`, else that entry's
/// edit, with its metadata filled in and every value empty.
#[component]
fn VaultFormView(
    entry: Option<VaultEntry>,
    bruhs: Signal<Vec<VaultBruh>>,
    busy: RwSignal<Busy>,
    wait: Signal<Option<&'static str>>,
    on_saved: Callback<VaultEntry>,
    on_close: Callback<()>,
) -> impl IntoView {
    let editing = entry.is_some();
    let has_totp = matches!(
        &entry,
        Some(VaultEntry {
            kind: VaultKind::Login(LoginMeta {
                fields: LoginFields::Saved { totp: true, .. },
                ..
            }),
            ..
        })
    );
    let scope_text = entry
        .as_ref()
        .map(|entry| bruhs.with_untracked(|bruhs| vault_row(entry, bruhs).scope))
        .unwrap_or_default();
    // Metadata only: `form_for` leaves every value empty.
    let start = entry.as_ref().map(form_for).unwrap_or_default();
    let kind = RwSignal::new(start.kind);
    let scope = RwSignal::new(start.scope);
    let source = RwSignal::new(start.source);
    let name = RwSignal::new(start.name.clone());
    let project = RwSignal::new(start.project.clone());
    let bruh = RwSignal::new(start.bruh.clone());
    let url = RwSignal::new(start.url.clone());
    let username = RwSignal::new(start.username.clone());
    let username_ref = RwSignal::new(start.username_ref.clone());
    let password_ref = RwSignal::new(start.password_ref.clone());
    let totp_ref = RwSignal::new(start.totp_ref.clone());
    let reference = RwSignal::new(start.reference.clone());
    // What an edit shows read only, beside FIXED_HINT.
    let fixed_texts: Vec<(FormField, String)> = if editing {
        FormField::ALL
            .into_iter()
            .filter_map(|field| fixed_text(field, &start, &scope_text).map(|text| (field, text)))
            .collect()
    } else {
        Vec::new()
    };
    drop(start);
    let remove_totp = RwSignal::new(false);
    let existing = StoredValue::new(entry);
    let value_node = NodeRef::<html::Input>::new();
    let password_node = NodeRef::<html::Input>::new();
    let totp_node = NodeRef::<html::Input>::new();
    let form_ref = NodeRef::<html::Form>::new();
    let error = RwSignal::new(None::<String>);
    let prefix = unique_id("vault-form");
    let fixed_id = format!("{prefix}-fixed");
    let totp_hint_id = format!("{prefix}-totp");
    let reference_hint_id = format!("{prefix}-reference");
    let bruh_hint_id = format!("{prefix}-bruh");
    let kind_id = format!("{prefix}-kind");
    let scope_id = format!("{prefix}-scope");
    let bruh_id = format!("{prefix}-bruh-choice");
    let source_id = format!("{prefix}-source");
    let fields = Memo::new(move |_| form_fields(kind.get(), source.get()));
    // Reads a value field and empties it in one step.
    let take = |node: NodeRef<html::Input>| {
        node.get_untracked()
            .map(|input| {
                let value = input.value();
                input.set_value("");
                value
            })
            .unwrap_or_default()
    };
    let clear = move || {
        for node in [value_node, password_node, totp_node] {
            if let Some(Some(input)) = node.try_get_untracked() {
                input.set_value("");
            }
        }
    };
    let cancel = Callback::new(move |_: ()| {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        clear();
        on_close.run(());
    });
    let submit = move || {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        let form = VaultForm {
            name: name.get_untracked(),
            kind: kind.get_untracked(),
            scope: scope.get_untracked(),
            project: project.get_untracked(),
            bruh: bruh.get_untracked(),
            value: take(value_node),
            url: url.get_untracked(),
            source: source.get_untracked(),
            username: username.get_untracked(),
            password: take(password_node),
            totp: take(totp_node),
            remove_totp: remove_totp.get_untracked(),
            reference: reference.get_untracked(),
            username_ref: username_ref.get_untracked(),
            password_ref: password_ref.get_untracked(),
            totp_ref: totp_ref.get_untracked(),
        };
        let request = existing.with_value(|existing| match existing {
            None => form_create(&form).map(Request::Create),
            Some(entry) => {
                form_update(&form, entry).map(|update| Request::Update(entry.id.clone(), update))
            }
        });
        drop(form);
        let request = match request {
            Ok(request) => request,
            Err(message) => {
                error.set(Some(message.into()));
                return;
            }
        };
        busy.set(Busy::Changing);
        error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = match &request {
                Request::Create(create) => crate::api::create_vault_entry(create).await,
                Request::Update(id, update) => crate::api::update_vault_entry(id, update).await,
            };
            drop(request);
            busy.try_set(Busy::Idle);
            match result {
                Ok(entry) => {
                    // The row or the page may be gone: a search can filter
                    // the row out, and leaving Settings drops the page.
                    on_saved.try_run(entry);
                    on_close.try_run(());
                }
                Err(err) => {
                    error.try_set(Some(err.to_string()));
                }
            }
        });
    };
    // The first field the owner can change takes focus.
    request_animation_frame(move || {
        let first = form_ref
            .try_get_untracked()
            .flatten()
            .and_then(|form| {
                form.query_selector("input:not([readonly]), select")
                    .ok()
                    .flatten()
            })
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(first) = first {
            let _ = first.focus();
        }
    });
    let fixed = {
        let fixed_id = fixed_id.clone();
        move |label: &'static str, value: String| fixed_field(label, value, fixed_id.clone())
    };
    let field_view = move |field: FormField| -> AnyView {
        match field {
            // Kind, Name, Used by and a login's Sign-in page are fixed once
            // the entry exists (D2, ruling F3); the hint follows the last.
            field if editing && field.fixed_on_edit() => {
                let label = field.label();
                let text = fixed_texts
                    .iter()
                    .find(|(fixed, _)| *fixed == field)
                    .map(|(_, text)| text.clone())
                    .unwrap_or_default();
                let hint_id = fixed_id.clone();
                view! {
                    <div>
                        {fixed(label, text)}
                        {move || (fields.with(|fields| fixed_hint_after(fields)) == Some(field)).then(|| view! { <p id=hint_id.clone() class=HINT>{FIXED_HINT}</p> })}
                    </div>
                }.into_any()
            }
            FormField::Kind => select_field(FIELD_KIND, kind_id.clone(), &KINDS, kind).into_any(),
            FormField::Name => text_field(FIELD_NAME, name, None, "text").into_any(),
            FormField::UsedBy => {
                let described = bruh_hint_id.clone();
                let hint_id = bruh_hint_id.clone();
                let scope_id = scope_id.clone();
                let bruh_id = bruh_id.clone();
                view! {
                    <div class="space-y-3">
                        <div>
                            <label for=scope_id.clone() class=LABEL>{FIELD_USED_BY}</label>
                            <select id=scope_id class=FIELD aria-describedby=move || bruhs.with(Vec::is_empty).then(|| described.clone()) prop:value=move || option_key(&SCOPES, scope.get()) on:change=move |event| scope.set(option_for(&SCOPES, &event_target_value(&event)))>
                                {SCOPES.iter().map(|(choice, key, text)| { let choice = *choice; view! {
                                    <option value=*key selected=move || scope.get() == choice disabled=move || choice == FormScope::Bruh && bruhs.with(Vec::is_empty)>{*text}</option>
                                } }).collect_view()}
                            </select>
                        </div>
                        {move || bruhs.with(Vec::is_empty).then(|| view! { <p id=hint_id.clone() class=HINT>{NO_BRUHS}</p> })}
                        <Show when=move || scope.get() == FormScope::Project>
                            {text_field(FIELD_PROJECT, project, None, "text")}
                        </Show>
                        <Show when=move || scope.get() == FormScope::Bruh>
                            <div>
                                <label for=bruh_id.clone() class=LABEL>{FIELD_BRUH}</label>
                                <select id=bruh_id.clone() class=FIELD prop:value=move || bruh.get() on:change=move |event| bruh.set(event_target_value(&event))>
                                    <option value="" selected=move || bruh.with(String::is_empty)>{CHOOSE_BRUH}</option>
                                    {move || bruhs.get().into_iter().map(|choice| { let id = choice.agent_id.clone(); let label = bruh_choice_label(&choice); view! {
                                        <option value=choice.agent_id selected=move || bruh.with(|bruh| *bruh == id)>{label}</option>
                                    } }).collect_view()}
                                </select>
                            </div>
                        </Show>
                    </div>
                }.into_any()
            }
            FormField::SignIn => text_field(FIELD_SIGN_IN, url, None, "url").into_any(),
            FormField::StoredIn => select_field(FIELD_STORED_IN, source_id.clone(), &SOURCES, source).into_any(),
            FormField::Username => text_field(FIELD_USERNAME, username, None, "text").into_any(),
            FormField::Password => view! { <SecretInput label=FIELD_PASSWORD node=password_node described_by=editing.then(|| KEEP_HINT_ID.to_owned()) on_escape=cancel /> }.into_any(),
            FormField::Totp => view! {
                <div>
                    <SecretInput label=FIELD_TOTP node=totp_node described_by=Some(totp_hint_id.clone()) on_escape=cancel />
                    <p id=totp_hint_id.clone() class=HINT>{TOTP_HINT}</p>
                    {(editing && has_totp).then(|| view! {
                        <label class="bruhs-control mt-1 flex min-h-control cursor-pointer items-center gap-2 text-label-sm text-rambutan-300">
                            <input type="checkbox" class="size-4 accent-blueberry-400" prop:checked=move || remove_totp.get() on:change=move |event| remove_totp.set(event_target_checked(&event)) />
                            <span>{FIELD_REMOVE_TOTP}</span>
                        </label>
                    })}
                </div>
            }.into_any(),
            FormField::UsernameRef => text_field(FIELD_USERNAME_REF, username_ref, Some(reference_hint_id.clone()), "text").into_any(),
            FormField::PasswordRef => text_field(FIELD_PASSWORD_REF, password_ref, Some(reference_hint_id.clone()), "text").into_any(),
            FormField::TotpRef => view! {
                <div>
                    {text_field(FIELD_TOTP_REF, totp_ref, Some(reference_hint_id.clone()), "text")}
                    <p id=reference_hint_id.clone() class=HINT>{REFERENCE_HINT}</p>
                </div>
            }.into_any(),
            FormField::Value => view! { <SecretInput label=FIELD_VALUE node=value_node described_by=editing.then(|| KEEP_HINT_ID.to_owned()) on_escape=cancel /> }.into_any(),
            FormField::Reference => view! {
                <div>
                    {text_field(FIELD_REFERENCE, reference, Some(reference_hint_id.clone()), "text")}
                    <p id=reference_hint_id.clone() class=HINT>{REFERENCE_HINT}</p>
                </div>
            }.into_any(),
        }
    };
    let field_view = StoredValue::new(field_view);
    view! {
        <form node_ref=form_ref data-vault-form="" novalidate=true class="mt-3 space-y-3 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50"
            on:submit=move |event| { event.prevent_default(); submit(); }
            on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { event.prevent_default(); event.stop_propagation(); cancel.run(()); }>
            {FormField::ALL.into_iter().map(|field| view! {
                <Show when=move || fields.with(|fields| fields.contains(&field))>
                    {move || field_view.with_value(|view| view(field))}
                </Show>
            }).collect_view()}
            {move || error.get().map(|message| view! { <p role="alert" class="text-label-sm text-lychee-300">{message}</p> })}
            <div class="flex flex-wrap gap-2">
                <button type="submit" class=BUTTON aria-describedby=WAIT_ID disabled=move || wait.get().is_some() title=move || wait.get()>{SAVE_ENTRY}</button>
                <button type="button" class=BUTTON aria-describedby=WAIT_ID disabled=move || wait.get().is_some() title=move || wait.get() on:click=move |_| cancel.run(())>{CANCEL}</button>
            </div>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::secrets::SavedCredentialState;
    use bruhs_models::vault::{CredentialScope, FormKind, FormSource};

    /// Ruling F3: an edit draws a login's sign-in page read only, with the
    /// stored page, and the hint after it.
    #[test]
    fn an_edit_shows_the_sign_in_page_fixed() {
        let entry = VaultEntry {
            id: "v1-a".into(),
            name: "Shop".into(),
            scope: CredentialScope::Global,
            kind: VaultKind::Login(LoginMeta {
                url: "https://shop.test/in".into(),
                fields: LoginFields::Saved {
                    username: "me".into(),
                    totp: false,
                },
            }),
            state: SavedCredentialState::Saved,
            attention: None,
        };
        let start = form_for(&entry);
        let fixed: Vec<_> = form_fields(start.kind, start.source)
            .into_iter()
            .filter_map(|field| fixed_text(field, &start, "All projects").map(|text| (field, text)))
            .collect();
        assert_eq!(
            fixed,
            [
                (FormField::Kind, "Login".to_owned()),
                (FormField::Name, "Shop".to_owned()),
                (FormField::UsedBy, "All projects".to_owned()),
                (FormField::SignIn, "https://shop.test/in".to_owned()),
            ]
        );
        assert_eq!(
            fixed_hint_after(&form_fields(FormKind::Login, FormSource::OnePassword)),
            Some(FormField::SignIn)
        );
        assert_eq!(fixed_text(FormField::Username, &start, ""), None);
        // Exactly the fields models fixes on an edit have a fixed text.
        for field in FormField::ALL {
            assert_eq!(
                fixed_text(field, &start, "").is_some(),
                field.fixed_on_edit(),
                "{field:?}"
            );
        }
        // Whatever the page holds, the update keeps the stored page.
        let mut edited = form_for(&entry);
        edited.url = "https://elsewhere.test/in".into();
        let update = form_update(&edited, &entry).unwrap();
        assert!(matches!(
            &update.secret,
            bruhs_models::vault::VaultSecretInput::Login { url, .. } if url == "https://shop.test/in"
        ));
    }
}
