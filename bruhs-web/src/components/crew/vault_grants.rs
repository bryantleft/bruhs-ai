//! The vault entries a bruh's Settings grant it (D3): its own entries as
//! lines, and one switch per shared entry. The picks live in the form's
//! `grants`, so Save sends them with the rest through
//! `settings_commands_for`, and only when they changed.

use bruhs_models::crew::{
    toggle_vault_grant, vault_grant_rows, ServiceNotice, VaultGrantRow as GrantRow,
    VAULT_GRANTS_HINT, VAULT_GRANTS_LABEL, VAULT_GRANTS_NONE,
};
use bruhs_models::vault::{open_from_own_address, VaultListing, LOADING, LOAD_RETRY, REFRESH};
use leptos::prelude::*;

use crate::ui::{Button, ButtonSize, Switch};

const HINT: &str = "text-label-sm text-rambutan-500";
const REASON_ID: &str = "bruh-vault-reason";

#[derive(Clone, PartialEq)]
enum Load {
    Loading,
    Ready(VaultListing),
    Failed(String),
    /// An older server: no `/api/vault`.
    ServingOld,
    /// A page on another address: the vault and its grants are the owner's
    /// (ruling F26), so this page can't list them.
    OwnerOnly,
}

#[component]
pub fn VaultGrants(
    agent_id: String,
    grants: RwSignal<Vec<String>>,
    /// Why the whole section is off, shown under its hint.
    disabled: Signal<Option<&'static str>>,
) -> impl IntoView {
    let agent_id = StoredValue::new(agent_id);
    let load = RwSignal::new(Load::Loading);
    let fetch = move || {
        load.set(Load::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            let next = match crate::api::vault_listing().await {
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
            Load::Ready(listing) => agent_id
                .with_value(|id| grants.with(|grants| vault_grant_rows(listing, id, grants))),
            _ => Vec::new(),
        })
    });
    let off = Signal::derive(move || disabled.get().is_some());
    let described = move || {
        if disabled.with(Option::is_some) {
            format!("bruh-vault-hint {REASON_ID}")
        } else {
            "bruh-vault-hint".to_owned()
        }
    };

    // Keyed by entry, so a toggle keeps the switch (and its focus) in place.
    let key = |row: &GrantRow| match row {
        GrantRow::Own { id, .. } => format!("own {id}"),
        GrantRow::Shared { id, .. } => id.clone(),
    };
    let own = |line: String| {
        view! {
            <li data-vault-own="" class="flex min-h-control min-w-0 items-center break-words text-label-sm text-rambutan-300">{line}</li>
        }
    };
    let shared = move |id: String, label: String| {
        let pick = id.clone();
        let shown = id.clone();
        let checked = Signal::derive(move || {
            rows.with(|rows| {
                rows.iter()
                    .any(|row| matches!(row, GrantRow::Shared { id, on: true, .. } if *id == shown))
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
                        disabled=off
                        on_change=Callback::new(move |on| grants.update(|grants| toggle_vault_grant(grants, &pick, on)))
                        attr:data-vault-grant=id
                        attr:title=move || disabled.get()
                        attr:aria-describedby=described
                    />
                </label>
            </li>
        }
    };
    let row = move |row: GrantRow| match row {
        GrantRow::Own { line, .. } => own(line).into_any(),
        GrantRow::Shared { id, label, .. } => shared(id, label).into_any(),
    };
    let ready = Memo::new(move |_| load.with(|load| matches!(load, Load::Ready(_))));
    let none = move || {
        !rows.with(|rows| {
            rows.iter()
                .any(|row| matches!(row, GrantRow::Shared { .. }))
        })
    };

    view! {
        <div role="group" aria-labelledby="bruh-vault-label" data-bruh-vault="" class="min-w-0 space-y-1">
            <div class="min-w-0">
                <span id="bruh-vault-label" class="text-label-sm text-rambutan-300">{VAULT_GRANTS_LABEL}</span>
                <p id="bruh-vault-hint" class=HINT>{VAULT_GRANTS_HINT}</p>
                {move || disabled.get().map(|reason| view! { <p id=REASON_ID data-bruh-vault-reason="" class=HINT>{reason}</p> })}
            </div>
            {move || match load.get() {
                Load::Loading => view! { <p role="status" class=HINT>{LOADING}</p> }.into_any(),
                Load::Failed(message) => view! {
                    <div class="flex min-w-0 flex-wrap items-center gap-3">
                        <p role="alert" class="min-w-0 text-label-sm text-lychee-300">{format!("{message} {LOAD_RETRY}")}</p>
                        <Button size=ButtonSize::Small on_click=Callback::new(move |_| fetch()) attr:data-vault-grants-refresh="">{REFRESH}</Button>
                    </div>
                }
                .into_any(),
                // An older service already says so through `disabled`.
                Load::ServingOld => disabled
                    .get()
                    .is_none()
                    .then(|| view! { <p role="status" data-vault-grants-notice="" class="text-label-sm text-durian-300">{ServiceNotice::ServingOld.message()}</p> })
                    .into_any(),
                Load::OwnerOnly => view! { <p role="status" data-vault-grants-notice="" class="text-label-sm text-durian-300">{open_from_own_address(crate::api::BASE_URL)}</p> }.into_any(),
                Load::Ready(_) => ().into_any(),
            }}
            <Show when=move || ready.get()>
                // A gap keeps neighbouring touch targets apart.
                <ul class="min-w-0 space-y-1">
                    <For each=move || rows.get() key=key children=row />
                </ul>
                <Show when=none>
                    <p data-vault-grants-none="" class=HINT>{VAULT_GRANTS_NONE}</p>
                </Show>
            </Show>
        </div>
    }
}
