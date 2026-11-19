//! User-controlled permissions and local verification, shared with task agents.
use crate::ui::{Button, ButtonSize, ButtonVariant, Input};
use bruhs_models::local_tools::*;
use leptos::prelude::*;
use serde::{de::DeserializeOwned, Serialize};

pub async fn request<T: DeserializeOwned, B: Serialize + ?Sized>(
    method: &str,
    path: &str,
    body: Option<&B>,
) -> Result<T, String> {
    let url = format!("{}{path}", crate::api::BASE_URL);
    let request = match method {
        "GET" => gloo_net::http::Request::get(&url),
        "POST" => gloo_net::http::Request::post(&url),
        "DELETE" => gloo_net::http::Request::delete(&url),
        _ => return Err("Unsupported local-tool request method".into()),
    };
    let response = match body {
        Some(body) => {
            request
                .json(body)
                .map_err(|error| error.to_string())?
                .send()
                .await
        }
        None => request.send().await,
    }
    .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response
            .text()
            .await
            .unwrap_or_else(|_| format!("Local tools request failed ({})", response.status())));
    }
    response.json().await.map_err(|error| error.to_string())
}

fn button(label: &'static str, busy: RwSignal<bool>, action: Callback<()>) -> impl IntoView {
    view! { <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=Signal::derive(move || busy.get()) on_click=Callback::new(move |_| action.run(()))>{label}</Button> }
}

#[component]
pub fn LocalToolsPanel() -> impl IntoView {
    let snapshot = RwSignal::new(LocalToolsSnapshot::default());
    let principal = RwSignal::new("Workbench".to_string());
    let url = RwSignal::new("http://localhost:3000".to_string());
    let selector = RwSignal::new(String::new());
    let text = RwSignal::new(String::new());
    let x = RwSignal::new("0".to_string());
    let y = RwSignal::new("0".to_string());
    let devices = RwSignal::new(DeviceInventory::default());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let result = RwSignal::new(None::<ToolReply>);
    let refresh = Callback::new(move |_: ()| {
        wasm_bindgen_futures::spawn_local(async move {
            match request::<LocalToolsSnapshot, ()>("GET", "/api/local-tools", None).await {
                Ok(value) => {
                    let _ = snapshot.try_set(value);
                }
                Err(message) => {
                    let _ = error.try_set(Some(message));
                }
            }
        });
    });
    refresh.run(());
    let enable = Callback::new(move |scope: ToolScope| {
        busy.set(true);
        error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let operation = async {
                let grant: ToolGrant = request(
                    "POST",
                    "/api/local-tools/grants",
                    Some(&GrantRequest {
                        principal: principal.get_untracked(),
                        scope,
                    }),
                )
                .await?;
                match scope {
                    ToolScope::Browser => {
                        let _: BrowserSession = request(
                            "POST",
                            "/api/local-tools/browser/start",
                            Some(&BrowserStart {
                                grant_id: grant.id,
                                url: url.get_untracked(),
                            }),
                        )
                        .await?;
                    }
                    ToolScope::Devices => {
                        let inventory: DeviceInventory = request::<_, ()>(
                            "GET",
                            &format!("/api/local-tools/devices?grant_id={}", grant.id),
                            None,
                        )
                        .await?;
                        let _ = devices.try_set(inventory);
                    }
                    ToolScope::Dictation => {}
                }
                Ok::<_, String>(())
            }
            .await;
            if let Err(message) = operation {
                let _ = error.try_set(Some(message));
            }
            let _ = busy.try_set(false);
            refresh.run(());
        });
    });
    let browser_action =
        Callback::new(move |(session, action): (BrowserSession, BrowserAction)| {
            busy.set(true);
            error.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                match request(
                    "POST",
                    "/api/local-tools/browser/command",
                    Some(&BrowserCommand {
                        grant_id: session.grant_id,
                        session_id: session.id,
                        command: action,
                    }),
                )
                .await
                {
                    Ok(reply) => {
                        let _ = result.try_set(Some(reply));
                    }
                    Err(message) => {
                        let _ = error.try_set(Some(message));
                    }
                }
                let _ = busy.try_set(false);
                refresh.run(());
            });
        });
    let device_action = Callback::new(move |(device, command): (Device, DeviceAction)| {
        let grant = snapshot.with_untracked(|snapshot| {
            snapshot
                .grants
                .iter()
                .find(|grant| {
                    grant.scope == ToolScope::Devices
                        && grant.principal == principal.get_untracked()
                })
                .cloned()
        });
        let Some(grant) = grant else {
            error.set(Some("Allow devices for this task first.".into()));
            return;
        };
        busy.set(true);
        error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match request(
                "POST",
                "/api/local-tools/devices/command",
                Some(&DeviceCommand {
                    grant_id: grant.id,
                    device_id: device.id,
                    platform: device.platform,
                    command,
                }),
            )
            .await
            {
                Ok(reply) => {
                    let _ = result.try_set(Some(reply));
                }
                Err(message) => {
                    let _ = error.try_set(Some(message));
                }
            }
            let _ = busy.try_set(false);
        });
    });
    view! { <section class="min-w-0 space-y-4" aria-label="Local verification tools">
        <div class="space-y-1"><h2 class="text-body font-medium text-rambutan-100">"Local verification"</h2><p class="text-label-sm text-rambutan-400">"Allow a named task to use a separate browser or your connected devices. Revoking access closes its browser and blocks new actions."</p></div>
        <label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Task or user"</span><Input aria_label="Task or user for local tool access" value=Signal::derive(move || principal.get()) on_input=Callback::new(move |value| principal.set(value)) /></label>
        <label class="block space-y-1 text-label-sm text-rambutan-300"><span>"Page URL"</span><Input aria_label="Local browser URL" value=Signal::derive(move || url.get()) on_input=Callback::new(move |value| url.set(value)) /></label>
        <div class="flex flex-wrap gap-2">
            {button("Allow browser and open", busy, Callback::new(move |_| enable.run(ToolScope::Browser)))}
            {button("Allow devices and refresh", busy, Callback::new(move |_| enable.run(ToolScope::Devices)))}
            {button("Refresh permissions", busy, refresh)}
        </div>
        <div class="space-y-2">{move || snapshot.get().grants.into_iter().map(|grant| {
            let id=grant.id.clone();
            view! { <div class="flex flex-wrap items-center justify-between gap-2 rounded-grape bg-longan-950 p-2 text-label-sm text-rambutan-200"><span>{format!("{}: {:?}", grant.principal, grant.scope)}</span>
                <Button variant=ButtonVariant::Destructive size=ButtonSize::Small on_click=Callback::new(move |_| {
                    let id=id.clone(); wasm_bindgen_futures::spawn_local(async move { match request::<serde_json::Value, ()>("DELETE", &format!("/api/local-tools/grants/{id}"), None).await { Ok(_) => refresh.run(()), Err(message) => { let _=error.try_set(Some(message)); } } });
                })>"Revoke"</Button></div> }
        }).collect_view()}</div>
        <div class="space-y-3">{move || snapshot.get().browsers.into_iter().map(|session| {
            let actions=[("Inspect page", BrowserAction::Snapshot), ("Screenshot", BrowserAction::Screenshot), ("Console", BrowserAction::Console), ("Network", BrowserAction::Network)];
            let navigate=session.clone(); let click=session.clone(); let fill=session.clone();
            view! { <div class="min-w-0 space-y-2 rounded-grape bg-longan-950 p-3"><p class="break-all text-label-sm text-rambutan-300">{session.url.clone()}</p><div class="flex flex-wrap gap-2">
                {actions.into_iter().map(|(label, action)| { let session=session.clone(); button(label,busy,Callback::new(move |_| browser_action.run((session.clone(),action.clone())))) }).collect_view()}
                {button("Navigate to URL",busy,Callback::new(move |_| browser_action.run((navigate.clone(),BrowserAction::Navigate{url:url.get_untracked()}))))}
            </div>
            <Input aria_label="Browser CSS selector" placeholder="CSS selector" value=Signal::derive(move || selector.get()) on_input=Callback::new(move |value| selector.set(value)) />
            <Input aria_label="Text to fill in browser" placeholder="Text to fill" value=Signal::derive(move || text.get()) on_input=Callback::new(move |value| text.set(value)) />
            <div class="flex flex-wrap gap-2">{button("Click element",busy,Callback::new(move |_| browser_action.run((click.clone(),BrowserAction::Click{selector:selector.get_untracked()}))))}{button("Fill element",busy,Callback::new(move |_| browser_action.run((fill.clone(),BrowserAction::Fill{selector:selector.get_untracked(),text:Some(text.get_untracked()),secret:None,submit:false}))))}</div>
            </div> }
        }).collect_view()}</div>
        <div class="space-y-2">{move || devices.get().diagnostics.into_iter().map(|message| view! { <p class="text-label-sm text-rambutan-400">{message}</p> }).collect_view()}</div>
        <div class="flex flex-wrap gap-2"><Input aria_label="Device tap X" placeholder="X" value=Signal::derive(move || x.get()) on_input=Callback::new(move |value| x.set(value)) /><Input aria_label="Device tap Y" placeholder="Y" value=Signal::derive(move || y.get()) on_input=Callback::new(move |value| y.set(value)) /></div>
        <div class="space-y-3">{move || devices.get().devices.into_iter().map(|device| {
            let tap=device.clone(); let typed=device.clone(); let opened=device.clone();
            view! { <div class="min-w-0 space-y-2 rounded-grape bg-longan-950 p-3"><p class="text-label-sm text-rambutan-200">{format!("{}: {}",device.name,device.state)}</p><div class="flex flex-wrap gap-2">
                {[ ("Screenshot",DeviceAction::Screenshot),("Start simulator",DeviceAction::Boot),("Stop simulator",DeviceAction::Shutdown),("Home",DeviceAction::Home),("Back",DeviceAction::Back) ].into_iter().map(|(label,action)|{let device=device.clone();button(label,busy,Callback::new(move |_|device_action.run((device.clone(),action.clone()))))}).collect_view()}
                {button("Tap coordinates",busy,Callback::new(move |_| match(x.get_untracked().parse(),y.get_untracked().parse()){(Ok(x),Ok(y))=>device_action.run((tap.clone(),DeviceAction::Tap{x,y})),_=>error.set(Some("Enter nonnegative X and Y coordinates.".into()))}))}
                {button("Type text",busy,Callback::new(move |_|device_action.run((typed.clone(),DeviceAction::Type{text:text.get_untracked()}))))}
                {button("Open URL",busy,Callback::new(move |_|device_action.run((opened.clone(),DeviceAction::OpenUrl{url:url.get_untracked()}))))}
            </div></div> }
        }).collect_view()}</div>
        <Show when=move || busy.get()><p class="text-label-sm text-rambutan-400" role="status">"Running local tool"</p></Show>
        {move || error.get().map(|message|view!{<p class="break-words text-label-sm text-lychee-300" role="alert">{message}</p>})}
        {move || result.get().map(|reply| view!{<div class="min-w-0 space-y-2">{reply.image.map(|image|view!{<img class="max-h-[32rem] max-w-full rounded-grape object-contain" src=format!("data:{};base64,{}",image.mime_type,image.base64) alt="Local verification screenshot" />})}<pre class="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-grape bg-longan-950 p-3 text-label-sm text-rambutan-200">{serde_json::to_string_pretty(&reply.value).unwrap_or_default()}</pre></div>})}
    </section> }
}

#[component]
pub fn DictationButton(on_text: Callback<String>) -> impl IntoView {
    let thread = expect_context::<crate::state::ActiveThreadId>().0;
    let owner = StoredValue::new(thread.get_untracked());
    let alive = RwSignal::new(true);
    let recording = StoredValue::new_local(None::<super::voice_enroll::Recorder>);
    let active = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let grant = RwSignal::new(None::<String>);
    let generation = RwSignal::new(0_u64);
    on_cleanup(move || {
        alive.set(false);
        recording.update_value(|value| {
            value.take();
        });
    });
    let stop = Callback::new(move |insert: bool| {
        generation.update(|value| *value = value.wrapping_add(1));
        active.set(false);
        let audio = recording
            .try_update_value(|value| value.take().map(|recording| recording.finish()))
            .flatten();
        let Some(audio) = audio else {
            return;
        };
        if !insert {
            return;
        }
        let audio = match audio {
            Ok(audio) => audio,
            Err(message) => {
                error.set(Some(message.to_string()));
                return;
            }
        };
        let Some(id) = grant.get_untracked() else {
            return;
        };
        let source_thread = owner.get_value();
        let source_generation = generation.get_untracked();
        busy.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let response = gloo_net::http::Request::post(&format!(
                    "{}/api/local-tools/dictation?model=base_en&language=en",
                    crate::api::BASE_URL
                ))
                .header("Content-Type", "audio/wav")
                .header("x-bruhs-tool-grant", &id)
                .body(audio)
                .map_err(|error| error.to_string())?
                .send()
                .await
                .map_err(|error| error.to_string())?;
                if !response.ok() {
                    return Err(response
                        .text()
                        .await
                        .unwrap_or_else(|_| "Local dictation failed".into()));
                }
                response
                    .json::<DictationTranscript>()
                    .await
                    .map_err(|error| error.to_string())
            }
            .await;
            match result {
                Ok(transcript)
                    if alive.try_get_untracked() == Some(true)
                        && thread.try_get_untracked() == Some(source_thread)
                        && generation.try_get_untracked() == Some(source_generation) =>
                {
                    on_text.run(transcript.text)
                }
                Ok(_) => {}
                Err(message) => {
                    let _ = error.try_set(Some(message));
                }
            }
            let _ = busy.try_set(false);
        });
    });
    Effect::new(move |_| {
        let current = thread.get();
        if active.get_untracked() && owner.get_value() != current {
            stop.run(false);
        }
    });
    let start = Callback::new(move |_: ()| {
        if active.get_untracked() {
            stop.run(true);
            return;
        }
        busy.set(true);
        error.set(None);
        let source_thread = thread.get_untracked();
        owner.set_value(source_thread.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let permission: ToolGrant = request(
                    "POST",
                    "/api/local-tools/grants",
                    Some(&GrantRequest {
                        principal: "Composer dictation".into(),
                        scope: ToolScope::Dictation,
                    }),
                )
                .await?;
                let captured = super::voice_enroll::begin_voice_capture()
                    .await
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>((permission, captured))
            }
            .await;
            match result {
                Ok((permission, captured)) => {
                    if thread.try_get_untracked() != Some(source_thread) {
                        drop(captured);
                        let _ = busy.try_set(false);
                        return;
                    }
                    if active.try_set(true).is_some() {
                        drop(captured);
                        return;
                    }
                    grant.set(Some(permission.id));
                    recording.set_value(Some(captured));
                    generation.update(|value| *value = value.wrapping_add(1));
                    let current_generation = generation.get_untracked();
                    // Match the five-minute dictation limit in the researched T3
                    // voice-input contract; stopping always shuts down microphone tracks.
                    wasm_bindgen_futures::spawn_local(async move {
                        gloo_timers::future::TimeoutFuture::new(300_000).await;
                        if active.try_get_untracked() == Some(true)
                            && generation.try_get_untracked() == Some(current_generation)
                        {
                            stop.run(true);
                        }
                    });
                }
                Err(message) => {
                    let _ = error.try_set(Some(message));
                }
            }
            let _ = busy.try_set(false);
        });
    });
    view! {<div class="min-w-0 space-y-1"><div class="flex flex-wrap gap-2">{button("Dictate locally",busy,start)}<Show when=move||active.get()>{button("Stop and insert",busy,Callback::new(move|_|stop.run(true)))}{button("Cancel recording",busy,Callback::new(move|_|stop.run(false)))}</Show></div><p class="text-label-sm text-rambutan-400" role="status">{move||if active.get(){"Recording microphone. Stop inserts text into the draft."}else if busy.get(){"Preparing local dictation"}else{"Audio is transcribed on this computer. Review the draft before sending."}}</p>{move||error.get().map(|message|view!{<p class="text-label-sm text-lychee-300" role="alert">{message}</p>})}</div>}
}
