//! Account controls share the same Connect flow in settings and operation approvals.
use bruhs_models::{
    connections::*,
    secrets::{SecretDecision, SecretRequest, SecretSubmission},
};
use leptos::prelude::*;

const BUTTON: &str = "min-h-11 rounded-seed px-3 text-label-sm inset-ring inset-ring-longan-700 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-50 cursor-pointer sm:min-h-10";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Idle,
    SignIn,
    Disconnect,
}

#[component]
pub fn ServiceConnections() -> impl IntoView {
    view! {
        <div class="mb-4 border-b border-longan-800 pb-4">
            <h3 class="font-medium text-rambutan-100">"Connected accounts"</h3>
            <p class="mt-1 text-label-sm text-rambutan-400">"Connect in your browser to reuse an existing sign-in. Your provider may still require consent or MFA. Each agent operation needs approval."</p>
            {ServiceProvider::ALL.into_iter().map(|provider| view! { <ConnectionControl provider=provider show_disconnect_note=false /> }).collect_view()}
            <p class="mt-3 text-label-sm text-rambutan-400">"Disconnect removes the local connection. It keeps your browser signed in and does not revoke access at the service."</p>
        </div>
    }
}

#[component]
pub fn ConnectionControl(
    provider: ServiceProvider,
    #[prop(optional)] login: Option<ConnectionLogin>,
    #[prop(optional)] on_change: Option<Callback<ServiceConnection>>,
    #[prop(default = true)] show_disconnect_note: bool,
) -> impl IntoView {
    let current = RwSignal::new(None::<ServiceConnection>);
    let work = RwSignal::new(Work::Idle);
    let busy = Signal::derive(move || work.get() != Work::Idle);
    let attempt = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let link = RwSignal::new(None::<String>);
    let alive = RwSignal::new(true);
    let generation = RwSignal::new(0u64);
    let read_generation = RwSignal::new(0u64);
    let login = StoredValue::new(login.unwrap_or(ConnectionLogin {
        thread_id: None,
        request_id: None,
    }));
    let refresh = Callback::new(move |_: ()| {
        read_generation.update(|value| *value = value.wrapping_add(1));
        let read = read_generation.get_untracked();
        let run = generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            if alive.try_get_untracked() != Some(true) {
                return;
            }
            let result = crate::api::service_connection(provider).await;
            if alive.try_get_untracked() != Some(true)
                || generation.try_get_untracked() != Some(run)
                || read_generation.try_get_untracked() != Some(read)
            {
                return;
            }
            match result {
                Ok(row) => {
                    if let Some(callback) = on_change {
                        callback.run(row.clone());
                    }
                    current.try_set(Some(row));
                }
                Err(err) => {
                    error.try_set(Some(err.to_string()));
                }
            }
        });
    });
    Effect::new(move |_| refresh.run(()));
    on_cleanup(move || {
        alive.set(false);
        if let Some(id) = attempt.get_untracked() {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = crate::api::cancel_connection(&id).await;
            });
        }
    });
    let connect = move |_| {
        if busy.get_untracked() {
            return;
        }
        // Open synchronously on the click so popup blockers do not discard sign-in.
        let popup = web_sys::window().and_then(|window| {
            window
                .open_with_url_and_target("about:blank", "_blank")
                .ok()
                .flatten()
        });
        if let Some(popup) = &popup {
            let _ = popup.set_opener(&wasm_bindgen::JsValue::NULL);
        }
        work.set(Work::SignIn);
        error.set(None);
        link.set(None);
        generation.update(|value| *value = value.wrapping_add(1));
        let run = generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let started = match crate::api::connect_service(provider, &login.get_value()).await {
                Ok(started) => started,
                Err(err) => {
                    if let Some(popup) = popup {
                        let _ = popup.close();
                    }
                    if alive.try_get_untracked() == Some(true)
                        && generation.try_get_untracked() == Some(run)
                    {
                        error.try_set(Some(err.to_string()));
                        work.try_set(Work::Idle);
                    }
                    return;
                }
            };
            if alive.try_get_untracked() != Some(true)
                || generation.try_get_untracked() != Some(run)
            {
                let _ = crate::api::cancel_connection(&started.id).await;
                if let Some(popup) = popup {
                    let _ = popup.close();
                }
                return;
            }
            attempt.set(Some(started.id.clone()));
            let opened = popup
                .as_ref()
                .is_some_and(|popup| popup.location().set_href(&started.auth_url).is_ok());
            if !opened {
                link.set(Some(started.auth_url));
            }
            loop {
                if alive.try_get_untracked() != Some(true)
                    || attempt.try_get_untracked().flatten().as_deref() != Some(&started.id)
                {
                    break;
                }
                let result = crate::api::connection_attempt(&started.id).await;
                if alive.try_get_untracked() != Some(true)
                    || generation.try_get_untracked() != Some(run)
                {
                    break;
                }
                match result {
                    Ok(status) if status.state == AttemptState::Pending => {}
                    Ok(status) => {
                        if matches!(status.state, AttemptState::Failed | AttemptState::Cancelled) {
                            error.try_set(status.message);
                        }
                        break;
                    }
                    Err(err) => {
                        error.try_set(Some(err.to_string()));
                        break;
                    }
                }
                // Same UI polling cadence as the existing Calendar connection flow.
                gloo_timers::future::TimeoutFuture::new(2_000).await;
            }
            if alive.try_get_untracked() == Some(true)
                && generation.try_get_untracked() == Some(run)
            {
                attempt.set(None);
                link.set(None);
                work.set(Work::Idle);
                refresh.run(());
            }
        });
    };
    let cancel = move |_| {
        if work.get_untracked() != Work::SignIn {
            return;
        }
        generation.update(|value| *value = value.wrapping_add(1));
        if let Some(id) = attempt.get_untracked() {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = crate::api::cancel_connection(&id).await;
            });
        }
        attempt.set(None);
        work.set(Work::Idle);
        link.set(None);
    };
    let disconnect = move |_| {
        if busy.get_untracked() {
            return;
        }
        work.set(Work::Disconnect);
        error.set(None);
        generation.update(|value| *value = value.wrapping_add(1));
        let run = generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::disconnect_service(provider).await;
            if alive.try_get_untracked() != Some(true)
                || generation.try_get_untracked() != Some(run)
            {
                return;
            }
            if let Err(err) = result {
                error.try_set(Some(err.to_string()));
            }
            work.try_set(Work::Idle);
            refresh.run(());
        });
    };
    view! {
        <div class="mt-3 min-w-0 border-t border-longan-800 pt-3" aria-label=format!("{} connection", provider.name())>
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="min-w-0 w-full sm:w-auto sm:flex-1">
                    <p class="font-medium">{provider.name()}</p>
                    <p class="text-label-sm text-rambutan-400">{format!("Requested access: {}", provider.access_summary())}</p>
                    <p class="break-all text-label-sm text-rambutan-400">{move || current.get().map_or_else(|| "Loading account…".into(), |row| row.account.unwrap_or_else(|| "No account connected".into()))}</p>
                    <p class="text-label-sm text-rambutan-400" role="status">{move || if work.get() == Work::Disconnect { "Disconnecting…" } else if busy.get() { "Waiting for browser sign-in…" } else { current.with(|row| match row.as_ref() {
                        Some(row) if !row.configured => "This service is not set up on this device.",
                        Some(row) => match row.state { ConnectionState::Connected => "Connected", ConnectionState::NeedsSignIn => "Needs sign-in", ConnectionState::Unavailable => "Keychain needs attention", ConnectionState::Disconnected => "Not connected" }, None => "",
                    }) }}</p>
                </div>
                <button type="button" class=BUTTON disabled=move || busy.get() || !current.with(|row| row.as_ref().is_some_and(|row| row.configured)) on:click=connect>{move || if current.with(|row| row.as_ref().is_some_and(|row| row.id.is_some())) { "Reconnect" } else { "Connect" }}</button>
                <Show when=move || work.get() == Work::SignIn><button type="button" class=BUTTON on:click=cancel>"Cancel"</button></Show>
                <Show when=move || !busy.get() && current.with(|row| row.as_ref().is_some_and(|row| row.id.is_some()))><button type="button" class=BUTTON on:click=disconnect>"Disconnect"</button></Show>
                <button type="button" class=BUTTON disabled=move || busy.get() on:click=move |_| refresh.run(())>"Refresh"</button>
            </div>
            <details class="mt-2 text-label-sm text-rambutan-400"><summary class="cursor-pointer">"Permissions"</summary><p class="mt-1 break-all">{move || current.get().map(|row| row.scopes.join(", ")).unwrap_or_default()}</p></details>
            {move || link.get().map(|url| view! { <a class="mt-2 inline-block text-blueberry-300 underline" href=url target="_blank" rel="noopener noreferrer">"Continue in your browser"</a> })}
            {move || error.get().map(|message| view! { <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}</p> })}
            {show_disconnect_note.then(|| view! { <p class="mt-2 text-label-sm text-rambutan-400">"Disconnect removes this local connection. It keeps your browser signed in and does not revoke access at the service."</p> })}
        </div>
    }
}

#[component]
pub fn ServiceOperationPrompt(request: SecretRequest) -> impl IntoView {
    let connection = request.connection.as_deref().cloned();
    let current = RwSignal::new(connection.clone());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let request = StoredValue::new(request);
    let crate::state::PendingSecretsSignal(pending) =
        expect_context::<crate::state::PendingSecretsSignal>();
    let respond = Callback::new(move |approve: bool| {
        if busy.get_untracked() {
            return;
        }
        let decision = if approve {
            let Some(id) = current
                .get_untracked()
                .filter(|c| c.state == ConnectionState::Connected)
                .and_then(|c| c.id)
            else {
                return;
            };
            SecretDecision::UseConnection { id }
        } else {
            SecretDecision::Decline
        };
        busy.set(true);
        error.set(None);
        let request = request.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let body = SecretSubmission {
                thread_id: request.thread_id,
                request_id: request.id,
                decision,
            };
            match crate::api::submit_secret(&body).await {
                Ok(true) => {
                    pending.try_update(|items| items.retain(|item| item.id != body.request_id));
                }
                Ok(false) => {}
                Err(err) => {
                    error.try_set(Some(err.to_string()));
                }
            }
            busy.try_set(false);
        });
    });
    view! {
        <section aria-label="Service connection request" class="mx-auto mb-3 max-h-[60vh] w-full max-w-3xl overflow-y-auto rounded-grape bg-longan-900 inset-ring inset-ring-blueberry-400/30 p-3 text-body-sm text-rambutan-200">
            <h3 class="font-medium">"Account access needed"</h3>
            <p class="mt-1">{request.get_value().operation.reason}</p>
            <pre class="my-2 whitespace-pre-wrap break-all text-label-sm">{format!("Command: {}\nFolder: {}\nVariable: {}", serde_json::to_string(&request.get_value().operation.command).unwrap_or_default(), bruhs_models::secrets::review_text(&request.get_value().cwd), request.get_value().operation.env_var)}</pre>
            {connection.map(|connection| view! { <ConnectionControl provider=connection.provider login=ConnectionLogin { thread_id: Some(request.get_value().thread_id), request_id: Some(request.get_value().id) } on_change=Callback::new(move |connection| current.set(Some(connection))) /> })}
            <p class="my-3 text-label-sm text-rambutan-400">"This command can access the account token. Only its exit status returns to the AI. Connecting an account does not approve this command."</p>
            <div class="flex flex-wrap gap-2">
                <button type="button" class=BUTTON disabled=move || busy.get() || !current.with(|row| row.as_ref().is_some_and(|c| c.state == ConnectionState::Connected && c.configured)) on:click=move |_| respond.run(true)>"Approve operation"</button>
                <button type="button" class=BUTTON disabled=move || busy.get() on:click=move |_| respond.run(false)>"Decline"</button>
            </div>
            {move || error.get().map(|message| view! { <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}</p> })}
        </section>
    }
}
