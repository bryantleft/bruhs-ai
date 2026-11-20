//! Settings, Apps' inline forms: Connect (name, Used by, the app's values,
//! a Get a token link, then Test and connect) and Replace token (the app's
//! secrets only). The rules are `bruhs_models::apps`' (ruling F17): the
//! fields in page order, the checked request, the copy.
//!
//! A token or key input is never filled, is read and emptied in one step
//! on submit, and is emptied on cancel and unmount.

use std::collections::BTreeMap;

use bruhs_models::apps::{
    connect_request, form_fields, replace_fields, replace_request, AppAccount, AppConnector,
    AppParam, AppsBruh, ConnectForm, APP_NAME_HINT, APP_SCOPE_OPTIONS as SCOPES, CONNECT,
    GET_TOKEN, KEEP_KEY_HINT, NO_BRUHS_FOR_APPS, REPLACE_TOKEN, TEST_AND_CONNECT,
};
use bruhs_models::secrets::review_text;
use bruhs_models::vault::{
    option_for, option_key, FormScope, VaultBusy as Busy, CANCEL, CHOOSE_BRUH, FIELD_BRUH,
    FIELD_NAME, FIELD_USED_BY,
};
use leptos::{html, prelude::*};
use wasm_bindgen::JsCast;

use crate::components::vault_settings::{unique_id, SecretInput, BUTTON, FIELD, HINT, LABEL};

/// A text link drawn as a button, for Get a token.
const LINK: &str = "bruhs-control inline-flex min-h-control shrink-0 items-center rounded-seed px-3 text-label-sm text-blueberry-300 underline-offset-2 inset-ring inset-ring-longan-700 transition-colors duration-150 hover:bg-longan-800 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400";
const FORM: &str =
    "mt-3 space-y-3 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50";

/// One secret input per secret value, in page order.
type SecretNodes = Vec<(&'static str, NodeRef<html::Input>)>;

fn secret_nodes(fields: &[&'static AppParam]) -> SecretNodes {
    fields
        .iter()
        .filter(|param| param.secret)
        .map(|param| (param.id, NodeRef::new()))
        .collect()
}

/// Reads every secret input and empties it in one step.
fn take_secrets(nodes: &SecretNodes, values: &mut BTreeMap<&'static str, String>) {
    for (id, node) in nodes {
        let value = node
            .get_untracked()
            .map(|input| {
                let value = input.value();
                input.set_value("");
                value
            })
            .unwrap_or_default();
        values.insert(id, value);
    }
}

fn clear_secrets(nodes: &SecretNodes) {
    for (_, node) in nodes {
        if let Some(Some(input)) = node.try_get_untracked() {
            input.set_value("");
        }
    }
}

/// Focus the form's first field once it has drawn.
fn focus_first(form: NodeRef<html::Form>) {
    request_animation_frame(move || {
        let first = form
            .try_get_untracked()
            .flatten()
            .and_then(|form| form.query_selector("input, select").ok().flatten())
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(first) = first {
            let _ = first.focus();
        }
    });
}

/// A secret input, or a text one bound to `values[param.id]`, with the
/// param's hint under it.
fn param_field(
    param: &'static AppParam,
    prefix: &str,
    values: RwSignal<BTreeMap<&'static str, String>>,
    nodes: &SecretNodes,
    described_by: Option<String>,
    cancel: Callback<()>,
) -> AnyView {
    let hint_id = format!("{prefix}-{}-hint", param.id);
    let hint = (!param.hint.is_empty())
        .then(|| view! { <p id=hint_id.clone() class=HINT>{param.hint}</p> });
    let described = match (&hint, described_by) {
        (Some(_), Some(other)) => Some(format!("{hint_id} {other}")),
        (Some(_), None) => Some(hint_id),
        (None, other) => other,
    };
    if param.secret {
        let node = nodes
            .iter()
            .find(|(id, _)| *id == param.id)
            .map(|(_, node)| *node)
            .unwrap_or_default();
        return view! {
            <div>
                <SecretInput label=param.label node=node described_by=described on_escape=cancel />
                {hint}
            </div>
        }
        .into_any();
    }
    let kind = if param.id == "email" { "email" } else { "text" };
    let id = param.id;
    view! {
        <div>
            <label class=LABEL>{param.label}
                <input type=kind autocomplete="off" autocapitalize="none" spellcheck="false" aria-describedby=described class=FIELD prop:value=move || values.with(|values| values.get(id).cloned().unwrap_or_default()) on:input=move |event| { let value = event_target_value(&event); values.update(|values| { values.insert(id, value); }); } />
            </label>
            {hint}
        </div>
    }
    .into_any()
}

#[component]
pub(super) fn ConnectFormView(
    app: &'static AppConnector,
    bruhs: Signal<Vec<AppsBruh>>,
    busy: RwSignal<Busy>,
    wait: Signal<Option<&'static str>>,
    on_connected: Callback<AppAccount>,
    on_close: Callback<()>,
    /// The connect got no answer: the page reloads, which closes this form.
    on_unanswered: Callback<()>,
    /// The page's wait line, which every button points at.
    wait_id: &'static str,
) -> impl IntoView {
    let fields = form_fields(app);
    let nodes = StoredValue::new(secret_nodes(&fields));
    let name = RwSignal::new(String::new());
    let scope = RwSignal::new(FormScope::Global);
    let bruh = RwSignal::new(String::new());
    // The values that aren't secrets: site, shop, email.
    let values = RwSignal::new(BTreeMap::<&'static str, String>::new());
    let error = RwSignal::new(None::<String>);
    let form_ref = NodeRef::<html::Form>::new();
    let prefix = unique_id("apps-form");
    let name_hint_id = format!("{prefix}-name");
    let scope_id = format!("{prefix}-scope");
    let bruh_id = format!("{prefix}-bruh");
    let no_bruhs_id = format!("{prefix}-no-bruhs");
    let cancel = Callback::new(move |_: ()| {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        nodes.with_value(clear_secrets);
        on_close.run(());
    });
    on_cleanup(move || {
        if let Some(nodes) = nodes.try_get_value() {
            clear_secrets(&nodes);
        }
    });
    let submit = move || {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        let mut typed = values.get_untracked();
        nodes.with_value(|nodes| take_secrets(nodes, &mut typed));
        let form = ConnectForm {
            app: app.id,
            name: name.get_untracked(),
            scope: scope.get_untracked(),
            bruh: bruh.get_untracked(),
            values: typed,
        };
        let request = connect_request(&form);
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
            let result = crate::api::connect_app(&request).await;
            drop(request);
            busy.try_set(Busy::Idle);
            match result {
                Ok(account) => {
                    // The page may be gone: leaving Settings drops it.
                    on_connected.try_run(account);
                    on_close.try_run(());
                }
                Err(err) if crate::api::vault_failure(&err).may_have_landed() => {
                    on_unanswered.try_run(());
                }
                Err(err) => {
                    error.try_set(Some(crate::api::vault_message(&err)));
                }
            }
        });
    };
    focus_first(form_ref);
    let param_views = nodes.with_value(|nodes| {
        fields
            .iter()
            .map(|param| param_field(param, &prefix, values, nodes, None, cancel))
            .collect_view()
    });
    view! {
        <form node_ref=form_ref data-apps-form="" aria-label=format!("{CONNECT} {}", app.name) novalidate=true class=FORM
            on:submit=move |event| { event.prevent_default(); submit(); }
            on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { event.prevent_default(); event.stop_propagation(); cancel.run(()); }>
            <div>
                <label class=LABEL>{FIELD_NAME}
                    <input type="text" autocomplete="off" spellcheck="false" aria-describedby=name_hint_id.clone() class=FIELD prop:value=move || name.get() on:input=move |event| name.set(event_target_value(&event)) />
                </label>
                <p id=name_hint_id class=HINT>{APP_NAME_HINT}</p>
            </div>
            <div class="space-y-3">
                <div>
                    <label for=scope_id.clone() class=LABEL>{FIELD_USED_BY}</label>
                    <select id=scope_id class=FIELD aria-describedby={ let id = no_bruhs_id.clone(); move || bruhs.with(Vec::is_empty).then(|| id.clone()) } prop:value=move || option_key(&SCOPES, scope.get()) on:change=move |event| scope.set(option_for(&SCOPES, &event_target_value(&event)))>
                        {SCOPES.iter().map(|(choice, key, text)| { let choice = *choice; view! {
                            <option value=*key selected=move || scope.get() == choice disabled=move || choice == FormScope::Bruh && bruhs.with(Vec::is_empty)>{*text}</option>
                        } }).collect_view()}
                    </select>
                </div>
                {move || bruhs.with(Vec::is_empty).then(|| view! { <p id=no_bruhs_id.clone() class=HINT>{NO_BRUHS_FOR_APPS}</p> })}
                <Show when=move || scope.get() == FormScope::Bruh>
                    <div>
                        <label for=bruh_id.clone() class=LABEL>{FIELD_BRUH}</label>
                        <select id=bruh_id.clone() class=FIELD prop:value=move || bruh.get() on:change=move |event| bruh.set(event_target_value(&event))>
                            <option value="" selected=move || bruh.with(String::is_empty)>{CHOOSE_BRUH}</option>
                            {move || bruhs.get().into_iter().map(|choice| { let id = choice.agent_id.clone(); let label = review_text(&choice.name); view! {
                                <option value=choice.agent_id selected=move || bruh.with(|bruh| *bruh == id)>{label}</option>
                            } }).collect_view()}
                        </select>
                    </div>
                </Show>
            </div>
            {param_views}
            {move || error.get().map(|message| view! { <p role="alert" class="text-label-sm text-lychee-300">{message}</p> })}
            <div class="flex flex-wrap items-center gap-2">
                <button type="submit" class=BUTTON aria-describedby=wait_id disabled=move || wait.get().is_some() title=move || wait.get()>{TEST_AND_CONNECT}</button>
                <a href=app.token_help_url target="_blank" rel="noopener noreferrer" data-apps-token-help="" class=LINK>{GET_TOKEN}</a>
                <button type="button" class=BUTTON aria-describedby=wait_id disabled=move || wait.get().is_some() title=move || wait.get() on:click=move |_| cancel.run(())>{CANCEL}</button>
            </div>
        </form>
    }
    .into_any()
}

#[component]
pub(super) fn ReplaceFormView(
    app: &'static AppConnector,
    /// The account's vault entry id.
    id: String,
    busy: RwSignal<Busy>,
    wait: Signal<Option<&'static str>>,
    on_replaced: Callback<AppAccount>,
    on_close: Callback<()>,
    /// The replace got no answer: the page reloads, which closes this form.
    on_unanswered: Callback<()>,
    wait_id: &'static str,
) -> impl IntoView {
    let fields = replace_fields(app);
    let nodes = StoredValue::new(secret_nodes(&fields));
    let id = StoredValue::new(id);
    let error = RwSignal::new(None::<String>);
    let form_ref = NodeRef::<html::Form>::new();
    let prefix = unique_id("apps-replace");
    let keep_id = format!("{prefix}-keep");
    let cancel = Callback::new(move |_: ()| {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        nodes.with_value(clear_secrets);
        on_close.run(());
    });
    on_cleanup(move || {
        if let Some(nodes) = nodes.try_get_value() {
            clear_secrets(&nodes);
        }
    });
    let submit = move || {
        if busy.get_untracked() != Busy::Idle {
            return;
        }
        let mut typed = BTreeMap::new();
        nodes.with_value(|nodes| take_secrets(nodes, &mut typed));
        let request = replace_request(app, &typed);
        drop(typed);
        let request = match request {
            Ok(request) => request,
            Err(message) => {
                error.set(Some(message.into()));
                return;
            }
        };
        busy.set(Busy::Changing);
        error.set(None);
        let id = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::replace_app_token(&id, &request).await;
            drop(request);
            busy.try_set(Busy::Idle);
            match result {
                Ok(account) => {
                    on_replaced.try_run(account);
                    on_close.try_run(());
                }
                Err(err) if crate::api::vault_failure(&err).may_have_landed() => {
                    on_unanswered.try_run(());
                }
                Err(err) => {
                    error.try_set(Some(crate::api::vault_message(&err)));
                }
            }
        });
    };
    focus_first(form_ref);
    let values = RwSignal::new(BTreeMap::new());
    let param_views = nodes.with_value(|nodes| {
        fields
            .iter()
            .map(|param| {
                let keep = (param.id == "key").then(|| keep_id.clone());
                param_field(param, &prefix, values, nodes, keep, cancel)
            })
            .collect_view()
    });
    let keeps_key = fields.iter().any(|param| param.id == "key");
    view! {
        <form node_ref=form_ref data-apps-form="" data-apps-replace-form="" aria-label=format!("{REPLACE_TOKEN}: {}", app.name) novalidate=true class=FORM
            on:submit=move |event| { event.prevent_default(); submit(); }
            on:keydown=move |event: web_sys::KeyboardEvent| if event.key() == "Escape" { event.prevent_default(); event.stop_propagation(); cancel.run(()); }>
            {param_views}
            {keeps_key.then(|| view! { <p id=keep_id.clone() class=HINT>{KEEP_KEY_HINT}</p> })}
            {move || error.get().map(|message| view! { <p role="alert" class="text-label-sm text-lychee-300">{message}</p> })}
            <div class="flex flex-wrap items-center gap-2">
                <button type="submit" class=BUTTON aria-describedby=wait_id disabled=move || wait.get().is_some() title=move || wait.get()>{REPLACE_TOKEN}</button>
                <a href=app.token_help_url target="_blank" rel="noopener noreferrer" class=LINK>{GET_TOKEN}</a>
                <button type="button" class=BUTTON aria-describedby=wait_id disabled=move || wait.get().is_some() title=move || wait.get() on:click=move |_| cancel.run(())>{CANCEL}</button>
            </div>
        </form>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::apps::connector;

    /// Every secret value gets a masked input, and nothing else does.
    #[test]
    fn only_secrets_get_masked_inputs() {
        let ids = |fields: &[&'static AppParam]| {
            secret_nodes(fields)
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        let jira = connector("jira").unwrap();
        assert_eq!(ids(&form_fields(jira)), ["token"]);
        let trello = connector("trello").unwrap();
        assert_eq!(ids(&form_fields(trello)), ["key", "token"]);
        assert_eq!(ids(&replace_fields(trello)), ["key", "token"]);
    }
}
