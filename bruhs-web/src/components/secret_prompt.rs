//! Inline private input. No credential value is stored in a signal, draft, or message.

use crate::{
    state::{ActiveThreadId, PendingSecretsSignal},
    ui::Icon,
};
use bruhs_models::secrets::{SecretDecision, SecretRequest, SecretSubmission};
use leptos::{html::Input, prelude::*};

#[component]
pub fn SecretPrompts() -> impl IntoView {
    let PendingSecretsSignal(pending) = expect_context::<PendingSecretsSignal>();
    let ActiveThreadId(active) = expect_context::<ActiveThreadId>();
    view! {
        <For
            each=move || { pending.get().into_iter().filter(|request| active.get().as_deref() == Some(request.thread_id.as_str())).collect::<Vec<_>>() }
            key=|request| request.id.clone()
            children=|request| view! { <SecretPrompt request=request /> }
        />
    }
}

#[component]
fn SecretPrompt(request: SecretRequest) -> impl IntoView {
    let PendingSecretsSignal(pending) = expect_context::<PendingSecretsSignal>();
    let input = NodeRef::<Input>::new();
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let revealed = RwSignal::new(false);
    let id = StoredValue::new(request.id.clone());
    let thread_id = StoredValue::new(request.thread_id);
    let saved = Signal::derive(move || {
        pending.with(|items| {
            items
                .iter()
                .any(|item| item.id == id.get_value() && item.saved_available)
        })
    });
    on_cleanup(move || {
        if let Some(input) = input.get_untracked() {
            input.set_value("");
        }
    });

    let respond = Callback::new(move |action: &'static str| {
        if busy.get_untracked() {
            return;
        }
        let value = input
            .get_untracked()
            .map(|input| input.value())
            .unwrap_or_default();
        if matches!(action, "once" | "save") && value.is_empty() {
            error.set(Some("Enter a credential to continue.".into()));
            return;
        }
        let decision = match action {
            "once" | "save" => SecretDecision::Provide {
                value,
                save: action == "save",
            },
            "saved" => SecretDecision::UseSaved,
            "forget" => SecretDecision::ForgetSaved,
            _ => SecretDecision::Decline,
        };
        // Clear before network I/O, including a decline or storage failure.
        if let Some(input) = input.get_untracked() {
            input.set_value("");
        }
        revealed.set(false);
        busy.set(true);
        error.set(None);
        let body = SecretSubmission {
            thread_id: thread_id.get_value(),
            request_id: id.get_value(),
            decision,
        };
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::submit_secret(&body).await {
                Ok(true) => {
                    pending.try_update(|items| items.retain(|item| item.id != body.request_id));
                }
                Ok(false) => {
                    pending.try_update(|items| {
                        if let Some(item) = items.iter_mut().find(|item| item.id == body.request_id)
                        {
                            item.saved_available = false;
                        }
                    });
                }
                Err(err) if err.is_status(404) => {
                    pending.try_update(|items| items.retain(|item| item.id != body.request_id));
                }
                Err(err) => {
                    error.try_set(Some(err.to_string()));
                }
            }
            busy.try_set(false);
        });
    });
    // JSON preserves argv boundaries and escapes control characters in the review.
    let command = serde_json::to_string(&request.operation.command).unwrap_or_default();
    let button = "min-h-11 rounded-seed px-3 text-label-sm inset-ring inset-ring-longan-700 hover:bg-longan-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 disabled:opacity-50 cursor-pointer sm:min-h-10";
    view! {
        <section aria-label="Private credential request" class="mx-auto mb-3 max-h-[60vh] w-full max-w-3xl overflow-y-auto rounded-grape bg-longan-900 inset-ring inset-ring-blueberry-400/30 p-3 text-body-sm text-rambutan-200">
            <div class="flex items-center gap-2 font-medium text-rambutan-100">
                <Icon name="shield" size=16 />
                <span>"Credential needed · "{request.operation.name}</span>
            </div>
            <p class="mt-1 break-words text-rambutan-300">{request.operation.reason}</p>
            {request.storage_unavailable.then(|| view! { <p role="status" class="mt-2 text-label-sm text-durian-300">"Saved credentials could not be checked. Unlock your OS keychain or use a credential once."</p> })}
            <dl class="my-3 grid min-w-0 grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-label-sm">
                <dt class="text-rambutan-400">"Command"</dt><dd class="min-w-0 break-all whitespace-pre-wrap font-mono">{command}</dd>
                <dt class="text-rambutan-400">"Folder"</dt><dd class="min-w-0 break-all">{bruhs_models::secrets::review_text(&request.cwd)}</dd>
                <dt class="text-rambutan-400">"Variable"</dt><dd class="min-w-0 break-all">{request.operation.env_var}</dd>
            </dl>
            <p id=move || format!("secret-note-{}", id.get_value()) class="mb-3 text-label-sm text-rambutan-400">
                "This command can access the credential. Only its exit status returns to the AI."
            </p>
            <form on:submit=move |event| { event.prevent_default(); respond.run("once"); }>
                <label class="block text-label-sm text-rambutan-300" for=move || format!("secret-value-{}", id.get_value())>"Credential"</label>
                <div class="mt-1 flex gap-2">
                    <input
                        node_ref=input
                        id=move || format!("secret-value-{}", id.get_value())
                        type=move || if revealed.get() { "text" } else { "password" }
                        autocomplete="off" autocapitalize="none" spellcheck="false"
                        aria-describedby=move || format!("secret-note-{}", id.get_value())
                        placeholder=move || if saved.get() { "Enter a replacement or use saved" } else { "Enter privately" }
                        disabled=move || busy.get()
                        on:keydown=move |event| event.stop_propagation()
                        class="min-h-10 min-w-0 flex-1 rounded-seed bg-longan-950 px-3 text-body-sm text-rambutan-100 inset-ring inset-ring-longan-700 focus:outline focus:outline-2 focus:outline-persimmon-400"
                    />
                    <button type="button" class=button disabled=move || busy.get() aria-pressed=move || revealed.get() on:click=move |_| revealed.update(|value| *value = !*value)>
                        {move || if revealed.get() { "Hide" } else { "Show" }}
                    </button>
                </div>
                {move || error.get().map(|message| view! { <p role="alert" class="mt-2 text-label-sm text-lychee-300">{message}" You can re-enter the credential or decline."</p> })}
                <div class="mt-3 flex flex-wrap items-center gap-2">
                    <button type="submit" class=button disabled=move || busy.get()>"Use once"</button>
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| respond.run("save")>"Save securely and use"</button>
                    <Show when=move || saved.get()>
                        <button type="button" class=button disabled=move || busy.get() on:click=move |_| respond.run("saved")>"Use saved"</button>
                        <button type="button" class=button disabled=move || busy.get() on:click=move |_| respond.run("forget")>"Forget saved"</button>
                    </Show>
                    <button type="button" class=button disabled=move || busy.get() on:click=move |_| respond.run("decline")>"Decline"</button>
                </div>
                <p aria-live="polite" class="mt-2 text-label-sm text-rambutan-400">{move || if busy.get() { "Submitting privately…" } else { "Saved credentials stay in your OS keychain for this project. Every use needs approval." }}</p>
            </form>
        </section>
    }
}
