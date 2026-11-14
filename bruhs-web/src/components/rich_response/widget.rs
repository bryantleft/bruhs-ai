use std::sync::{Arc, Mutex, PoisonError};

use bruhs_models::logic::rich_response::{
    interactive_url_with_nonce, widget_state_key, WidgetAction, WidgetEvent, WidgetState,
    MAX_WIDGET_HEIGHT, MIN_WIDGET_HEIGHT,
};
use leptos::html::{Iframe, Section};
use wasm_bindgen::JsCast;

use super::*;
use crate::state::ComposerDraft;

#[derive(Default)]
struct PendingState {
    next: Option<WidgetState>,
    running: bool,
}

#[derive(Clone)]
struct StateWriter {
    key: String,
    pending: Arc<Mutex<PendingState>>,
    status: RwSignal<Option<String>>,
}

impl StateWriter {
    fn queue(&self, state: WidgetState) {
        let recovery = client::cache_state(&self.key, &state).err();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next = Some(state);
        self.drain();
        if let Some(error) = recovery {
            let _ = self.status.try_set(Some(error));
        }
    }

    fn drain(&self) {
        {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if pending.running || pending.next.is_none() {
                return;
            }
            pending.running = true;
        }
        let writer = self.clone();
        let _ = writer.status.try_set(Some("Saving widget state…".into()));
        leptos::task::spawn_local(async move {
            loop {
                let next = {
                    let mut pending = writer
                        .pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    match pending.next.take() {
                        Some(state) => state,
                        None => {
                            pending.running = false;
                            drop(pending);
                            break;
                        }
                    }
                };
                match client::save_state(&writer.key, &next).await {
                    Ok(_) => {
                        let status = client::clear_cached_state(&writer.key, &next).err();
                        let _ = writer.status.try_set(status);
                    }
                    Err(error) => {
                        let mut pending = writer
                            .pending
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        if pending.next.is_none() {
                            pending.next = Some(next);
                        }
                        pending.running = false;
                        drop(pending);
                        let _ = writer.status.try_set(Some(format!("Widget values are kept locally. Could not save them to the server: {error}")));
                        break;
                    }
                }
            }
        });
    }
}

fn load_state(
    key: String,
    state: RwSignal<Option<WidgetState>>,
    error: RwSignal<Option<String>>,
    writer: StateWriter,
) {
    leptos::task::spawn_local(async move {
        match client::cached_state(&key) {
            Ok(Some(recovered)) => {
                let _ = state.try_set(Some(recovered.clone()));
                writer.queue(recovered);
                return;
            }
            Err(reason) => {
                let _ = error.try_set(Some(reason));
            }
            Ok(None) => {}
        }
        match client::load_state(&key).await {
            Ok(saved) => {
                let _ = state.try_set(Some(saved));
            }
            Err(reason) => {
                let _ = error.try_set(Some(format!(
                    "Could not load saved widget values: {reason}"
                )));
            }
        }
    });
}

#[component]
pub(super) fn InteractiveResponse(
    html: HtmlBlock,
    #[prop(default = String::new())] scope: String,
    #[prop(optional)] artifact_id: Option<String>,
) -> impl IntoView {
    let show_source = RwSignal::new(false);
    let generation = RwSignal::new(0_u64);
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let errors = RwSignal::new(Vec::<String>::new());
    let status = RwSignal::new(None::<String>);
    let diagnostic_status = RwSignal::new(None::<String>);
    let proposal = RwSignal::new(None::<WidgetAction>);
    let height = RwSignal::new(320.0_f64);
    let section = NodeRef::<Section>::new();
    let source = StoredValue::new(html.html);
    let title = if html.title.is_empty() {
        "Interactive response".into()
    } else {
        html.title
    };
    let key = StoredValue::new(widget_state_key(&scope, &source.get_value()));
    let widget_scope = StoredValue::new(scope);
    let reporter = StoredValue::new(
        artifact_id
            .clone()
            .map(|id| diagnostics::DiagnosticsWriter::new(id, diagnostic_status)),
    );
    let artifact_id = StoredValue::new(artifact_id);
    #[cfg(target_arch = "wasm32")]
    let state = RwSignal::new(None::<WidgetState>);
    #[cfg(not(target_arch = "wasm32"))]
    let state = RwSignal::new(Some(WidgetState::default()));
    let writer = StateWriter {
        key: key.get_value(),
        pending: Arc::new(Mutex::new(PendingState::default())),
        status,
    };
    #[cfg(target_arch = "wasm32")]
    load_state(key.get_value(), state, error, writer.clone());
    let writer = StoredValue::new(writer);
    let interaction = use_context::<ChatInteraction>();
    Effect::new(move |_| {
        if let Some(saved) = state.get() {
            client::remember_state(&widget_scope.get_value(), &key.get_value(), &saved);
        }
    });
    on_cleanup(move || client::forget_state(&key.get_value()));
    let on_event = Callback::new(move |event: WidgetEvent| match event {
        WidgetEvent::Ready => {
            loading.set(false);
            if let Some(reporter) = reporter.get_value() {
                reporter.queue(errors.get_untracked());
            }
        }
        WidgetEvent::Resize { height: next } if next.is_finite() => {
            height.set(next.clamp(MIN_WIDGET_HEIGHT, MAX_WIDGET_HEIGHT))
        }
        WidgetEvent::Resize { .. } => {}
        WidgetEvent::State { state: next } => {
            if let Some(interaction) = interaction {
                interaction.0.run(());
            }
            client::remember_state(&widget_scope.get_value(), &key.get_value(), &next);
            state.set(Some(next.clone()));
            writer.get_value().queue(next);
        }
        WidgetEvent::Error { message } => {
            errors.update(|errors| {
                if !errors.contains(&message) {
                    errors.push(message.clone());
                }
            });
            error.set(Some(message));
            if let Some(reporter) = reporter.get_value() {
                reporter.queue(errors.get_untracked());
            }
        }
        WidgetEvent::Action { action } => {
            if let Some(interaction) = interaction {
                interaction.0.run(());
            }
            proposal.set(Some(action));
        }
    });
    let draft = use_context::<ComposerDraft>().map(|draft| draft.0);
    view! {
        <section node_ref=section on:pointerdown=move |_| { if let Some(interaction)=interaction {interaction.0.run(());} } class="my-2 min-w-0 overflow-hidden rounded-grape bg-longan-900/40 inset-ring inset-ring-longan-800 [&:fullscreen]:overflow-auto [&:fullscreen]:bg-longan-950 [&:fullscreen]:p-3" data-response-interactive="" data-widget-state-key=key.get_value()>
            <div class="flex min-h-8 flex-wrap items-center gap-1 px-2">
                <span class="min-w-0 flex-1 truncate text-label-sm text-rambutan-300" title=title>{title.clone()}</span>
                <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") aria-pressed=move || show_source.get().to_string() on:click=move |_| show_source.update(|value| *value = !*value)>{move || if show_source.get() { "Preview" } else { "Source" }}</button>
                <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") aria-label="Fullscreen interactive response" on:click=move |_| { if let Some(node) = section.get_untracked() { bridge::fullscreen((*node).clone(), error); } }>"Fullscreen"</button>
                <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") aria-label="Reload interactive response" on:click=move |_| { loading.set(true); error.set(None); errors.set(Vec::new()); generation.update(|value| *value = value.wrapping_add(1)); }>"Reload"</button>
                <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") aria-label="Reset saved widget values" on:click=move |_| { let empty = WidgetState::default(); state.set(Some(empty.clone())); writer.get_value().queue(empty); loading.set(true); error.set(None); errors.set(Vec::new()); generation.update(|value| *value = value.wrapping_add(1)); }>"Reset values"</button>
                <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| {
                    let theme=match bridge::theme() { Ok(theme)=>theme,Err(reason)=>{error.set(Some(reason));return;} };
                    let document = bruhs_models::logic::rich_response::standalone_widget_document(&source.get_value(), &WidgetState::default(), &theme);
                    if let Err(reason) = client::download(document.as_bytes(), "text/html", "widget.html") { error.set(Some(reason)); }
                }>"Download HTML"</button>
            </div>
            <Show when=move || loading.get() && !show_source.get()><p class="px-3 py-1 text-label-sm text-rambutan-500" role="status">"Loading interactive response…"</p></Show>
            <Show when=move || error.get().is_some()>
                <div class="px-3 py-2 text-label-sm text-lychee-300" role="status">
                    <p>{move || error.get()}</p>
                    <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| { error.set(None); load_state(key.get_value(), state, error, writer.get_value()); }>"Retry saved state"</button>
                </div>
            </Show>
            <div class="flex min-h-7 items-center gap-1 px-3 text-label-sm text-rambutan-400"><span role="status">{move || status.get()}</span><Show when=move || status.with(|status|status.as_deref().is_some_and(|status|status!="Saving widget state…"))><button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| writer.get_value().drain()>"Retry save"</button></Show></div>
            <Show when=move || diagnostic_status.get().is_some()><div class="flex items-center gap-1 px-3 py-1 text-label-sm text-rambutan-400" role="status"><span>{move || diagnostic_status.get()}</span><button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| { if let Some(reporter)=reporter.get_value() { reporter.drain(); } }>"Retry preview feedback"</button></div></Show>
            <div class:hidden=move || show_source.get() style=move || format!("height:{}px;min-height:80px;resize:vertical;overflow:hidden", height.get())>
                <Show when=move || state.get().is_some()>
                    <For each=move || [generation.get()] key=|generation| *generation children=move |_| view! {
                        <WidgetFrame html=source.get_value() state=state.get_untracked().unwrap_or_default() artifact_id=artifact_id.get_value() on_event />
                    } />
                </Show>
            </div>
            <Show when=move || show_source.get()><pre class="max-h-96 overflow-auto whitespace-pre-wrap break-words p-3 text-code-sm text-rambutan-300"><code>{source.get_value()}</code></pre></Show>
            {move || proposal.get().map(|action| {
                let text = action.message.clone();
                view! {
                    <div class="border-t border-longan-800 p-3" data-widget-action-review="">
                        <p class="text-label-sm text-rambutan-200">{action.label}</p>
                        <pre class="my-2 max-h-48 overflow-auto whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{action.message}</pre>
                        <div class="flex flex-wrap gap-2">
                            <button type="button" disabled=draft.is_none() class=button_class(ButtonVariant::Outline, ButtonSize::Small, "") on:click=move |_| {
                                if let Some(draft) = draft {
                                    draft.update(|draft| { if !draft.is_empty() { draft.push_str("\n\n"); } draft.push_str(&text); });
                                    proposal.set(None);
                                    if let Some(input) = web_sys::window().and_then(|window| window.document()).and_then(|document| document.get_element_by_id("chat-input")).and_then(|input| input.dyn_into::<web_sys::HtmlElement>().ok()) { let _ = input.focus(); }
                                }
                            }>"Insert into composer"</button>
                            <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| proposal.set(None)>"Dismiss"</button>
                        </div>
                    </div>
                }
            })}
        </section>
    }
}

#[component]
fn WidgetFrame(
    html: String,
    state: WidgetState,
    artifact_id: Option<String>,
    on_event: Callback<WidgetEvent>,
) -> impl IntoView {
    let frame = NodeRef::<Iframe>::new();
    let interaction = use_context::<ChatInteraction>();
    let nonce = uuid::Uuid::new_v4().to_string();
    let src = match artifact_id {
        Some(id) => format!(
            "{BASE_URL}/api/agent/artifacts/{}/preview?nonce={}",
            crate::api::segment(&id),
            crate::api::segment(&nonce)
        ),
        None => interactive_url_with_nonce(BASE_URL, &html, &nonce),
    };
    let nonce = StoredValue::new(nonce);
    let state = StoredValue::new(state);
    Effect::new(move |_| {
        if let Some(node) = frame.get() {
            bridge::install(
                (*node).clone().into(),
                nonce.get_value(),
                state.get_value(),
                on_event,
            );
            bridge::watch_interaction(
                (*node).clone().into(),
                interaction.map(|interaction| interaction.0),
            );
        }
    });
    view! {
        <iframe node_ref=frame src=src title="Interactive response" sandbox="allow-scripts" allow="camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'; display-capture 'none'; fullscreen *" allowfullscreen referrerpolicy="no-referrer" class="h-full w-full border-0"></iframe>
    }
}

#[component]
pub(crate) fn ArtifactResponse(
    artifact: bruhs_models::logic::rich_response::ArtifactBlock,
    scope: String,
) -> impl IntoView {
    let result = RwSignal::new(None::<Result<client::Artifact, String>>);
    let artifact = StoredValue::new(artifact);
    let reload = Callback::new(move |()| {
        result.set(None);
        let id = artifact.get_value().id;
        leptos::task::spawn_local(async move {
            let loaded = client::artifact(&id)
                .await
                .map_err(|error| error.to_string());
            let _ = result.try_set(Some(loaded));
        });
    });
    #[cfg(target_arch = "wasm32")]
    reload.run(());
    let scope = StoredValue::new(scope);
    view! {
        <div data-response-artifact=artifact.get_value().id>
            {move || match result.get() {
                None => view! { <p class="py-2 text-label-sm text-rambutan-500" role="status">"Loading artifact…"</p> }.into_any(),
                Some(Err(error)) => view! { <div class="py-2 text-label-sm text-lychee-300" role="status"><p>{error}</p><button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") on:click=move |_| reload.run(())>"Retry artifact"</button></div> }.into_any(),
                Some(Ok(saved)) => {
                    let title = if artifact.get_value().title.is_empty() { saved.title } else { artifact.get_value().title };
                    view! { <InteractiveResponse html=HtmlBlock { html: saved.html, title } scope=scope.get_value() artifact_id=saved.id /> }.into_any()
                }
            }}
        </div>
    }
}
