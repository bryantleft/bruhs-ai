use std::collections::{BTreeMap, BTreeSet};

use leptos::html::{Dialog, Figure, Iframe};

use super::*;

fn bound_media(media: &MediaBlock, bindings: &BTreeMap<String, String>) -> MediaBlock {
    let mut media = media.clone();
    let bind = |source: &str| {
        bindings
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.to_string())
    };
    media.src = bind(&media.src);
    media.poster = media.poster.as_deref().map(bind);
    for track in &mut media.tracks {
        track.src = bind(&track.src);
    }
    media
}

#[component]
pub(super) fn MediaResponse(
    media: Signal<MediaBlock>,
    project_path: Signal<Option<String>>,
) -> impl IntoView {
    let current = RwSignal::new(media.get_untracked());
    let bindings = StoredValue::new(BTreeMap::<String, String>::new());
    Effect::new(move |_| {
        let next = media.get();
        let same = bindings.with_value(|bindings| {
            bound_media(&current.get_untracked(), bindings) == bound_media(&next, bindings)
        });
        if !same {
            current.set(next);
        }
    });
    let on_captured = Callback::new(move |next: BTreeMap<String, String>| {
        bindings.update_value(|bindings| bindings.extend(next))
    });
    view! { {move || view! { <InlineMedia media=current.get() project_path on_captured /> }} }
}

fn kind_label(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Pdf => "PDF",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Sources {
    local: MediaBlock,
    shell: MediaBlock,
}

fn asset_sources(media: &MediaBlock) -> Vec<String> {
    std::iter::once(media.src.clone())
        .chain(media.poster.iter().cloned())
        .chain(media.tracks.iter().map(|track| track.src.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn resolved(
    media: &MediaBlock,
    project: Option<&str>,
    captured: &BTreeMap<String, client::Captured>,
) -> Sources {
    let transform = |source: &str, shell: bool| {
        if let Some(asset) = captured.get(source) {
            if shell {
                format!("{BASE_URL}{}", asset.signed_url)
            } else {
                media_url(BASE_URL, &asset.src, None)
            }
        } else {
            media_url(BASE_URL, source, project)
        }
    };
    let map = |shell| {
        let mut mapped = media.clone();
        mapped.src = transform(&media.src, shell);
        mapped.poster = media
            .poster
            .as_deref()
            .map(|source| transform(source, shell));
        for track in &mut mapped.tracks {
            track.src = transform(&track.src, shell);
        }
        mapped
    };
    Sources {
        local: map(false),
        shell: map(true),
    }
}

#[component]
pub(super) fn InlineMedia(
    media: MediaBlock,
    project_path: Signal<Option<String>>,
    #[prop(optional)] on_captured: Option<Callback<BTreeMap<String, String>>>,
) -> impl IntoView {
    let external: Vec<_> = asset_sources(&media)
        .into_iter()
        .filter(|source| media_is_remote(source))
        .collect();
    let remote = !external.is_empty();
    let loaded = RwSignal::new(!remote);
    let failed = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let retry = RwSignal::new(0_u64);
    let zoom = RwSignal::new(false);
    let download_busy = RwSignal::new(false);
    let kind = media.kind;
    let caption = if media.title.is_empty() {
        format!("Response {}", kind_label(kind))
    } else {
        media.title.clone()
    };
    let media = StoredValue::new(media);
    let caption = StoredValue::new(caption);
    let figure = NodeRef::<Figure>::new();
    let interaction = use_context::<ChatInteraction>();
    #[cfg(target_arch = "wasm32")]
    let sources = RwSignal::new(None::<Sources>);
    #[cfg(not(target_arch = "wasm32"))]
    let sources = RwSignal::new(Some(resolved(
        &media.get_value(),
        project_path.get_untracked().as_deref(),
        &BTreeMap::new(),
    )));
    let generation = StoredValue::new(0_u64);
    let captured_assets = RwSignal::new(BTreeMap::<String, client::Captured>::new());
    let load = Callback::new(move |()| {
        let media = media.get_value();
        if media.tracks.is_empty() {
            loaded.set(true);
            return;
        }
        sources.set(None);
        loaded.set(true);
        error.set(None);
        let external = asset_sources(&media)
            .into_iter()
            .filter(|source| media_is_remote(source))
            .collect::<Vec<_>>();
        let mut captured = captured_assets.get_untracked();
        let project = project_path.get_untracked();
        leptos::task::spawn_local(async move {
            let requests = external.into_iter().map(|source| async move {
                client::capture_remote(&source)
                    .await
                    .map(|asset| (source, asset))
            });
            match futures_util::future::try_join_all(requests).await {
                Ok(assets) => {
                    captured.extend(assets);
                    if let Some(callback) = on_captured {
                        let _ = callback.try_run(
                            captured
                                .iter()
                                .map(|(source, asset)| (source.clone(), asset.src.clone()))
                                .collect(),
                        );
                    }
                    let _ = captured_assets.try_set(captured.clone());
                    let _ = sources.try_set(Some(resolved(&media, project.as_deref(), &captured)));
                }
                Err(reason) => {
                    let _ = error.try_set(Some(format!(
                        "Could not load media with captions: {reason}"
                    )));
                }
            }
        });
    });
    Effect::new(move |_| {
        let project = project_path.get();
        retry.track();
        generation.update_value(|value| *value = value.wrapping_add(1));
        let mine = generation.get_value();
        let media = media.get_value();
        let local: Vec<_> = asset_sources(&media)
            .into_iter()
            .filter(|source| !media_is_remote(source))
            .collect();
        if local.is_empty() {
            sources.set(Some(resolved(&media, project.as_deref(), &BTreeMap::new())));
            return;
        }
        sources.set(None);
        leptos::task::spawn_local(async move {
            let requests = local.into_iter().map(|source| {
                let project = project.clone();
                async move {
                    client::capture_media(&source, project.as_deref())
                        .await
                        .map(|captured| (source, captured))
                }
            });
            let result = futures_util::future::try_join_all(requests).await;
            if generation.try_get_value() != Some(mine) {
                return;
            }
            match result {
                Ok(captured) => {
                    let captured: BTreeMap<String, client::Captured> =
                        captured.into_iter().collect();
                    if let Some(callback) = on_captured {
                        let _ = callback.try_run(
                            captured
                                .iter()
                                .map(|(source, asset)| (source.clone(), asset.src.clone()))
                                .collect(),
                        );
                    }
                    let _ = captured_assets.try_set(captured.clone());
                    let _ = sources.try_set(Some(resolved(&media, project.as_deref(), &captured)));
                }
                Err(reason) => {
                    let _ = error.try_set(Some(format!(
                        "Could not keep this media available: {reason}"
                    )));
                }
            }
        });
    });
    let external = StoredValue::new(external);
    view! {
        <figure node_ref=figure on:pointerdown=move |_| {if let Some(interaction)=interaction {interaction.0.run(());}} class="my-2 min-w-0 [&:fullscreen]:overflow-auto [&:fullscreen]:bg-longan-950 [&:fullscreen]:p-3" data-response-media=kind_label(kind)>
            <figcaption class="mb-1 flex flex-wrap items-center gap-1 text-label-sm text-rambutan-400">
                <span class="min-w-0 flex-1 break-words">{caption.get_value()}</span>
                <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") disabled=move || !loaded.get() || sources.get().is_none() on:click=move |_| { if let Some(node) = figure.get_untracked() { bridge::fullscreen((*node).clone(), error); } }>"Fullscreen"</button>
                {(kind == MediaKind::Image).then(|| view! { <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") disabled=move || !loaded.get() || sources.get().is_none() on:click=move |_| zoom.set(true)>"Zoom image"</button> })}
                {move || if media_is_remote(&media.get_value().src) {
                    view! { <button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") disabled=move || download_busy.get() on:click=move |_| {
                        download_busy.set(true); let source = media.get_value().src;
                        leptos::task::spawn_local(async move { if let Err(reason)=client::download_remote(&source).await { let _=error.try_set(Some(reason)); } let _=download_busy.try_set(false); });
                    }>{move || if download_busy.get() { "Downloading…" } else { "Download" }}</button> }.into_any()
                } else { sources.get().map(|sources| view! { <a class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "") href=format!("{}&download=true", sources.local.src)>"Download"</a> }).into_any() }}
            </figcaption>
            <Show when=move || loaded.get() fallback=move || view! {
                <div class="flex min-w-0 flex-col items-start gap-2 rounded-grape bg-longan-900/60 p-3 inset-ring inset-ring-longan-800">
                    <div class="max-h-28 w-full overflow-auto">{external.get_value().into_iter().map(|source| view! { <p class="break-all text-label-sm text-rambutan-400">{source}</p> }).collect_view()}</div>
                    <button type="button" disabled=move ||sources.get().is_none() class=button_class(ButtonVariant::Outline, ButtonSize::Small, "") on:click=move |_| load.run(())>{format!("Load {}", kind_label(kind))}</button>
                </div>
            }>
                {move || match sources.get() {
                    None => view! { <p class="py-2 text-label-sm text-rambutan-500" role="status">"Keeping media available…"</p> }.into_any(),
                    Some(sources) if remote => view! { <MediaFrame media=sources.shell title=caption.get_value() /> }.into_any(),
                    Some(sources) => view! { <NativeMedia media=sources.local title=caption.get_value() failed /> }.into_any(),
                }}
            </Show>
            <Show when=move || failed.get() || error.get().is_some()>
                <div class="mt-2 text-label-sm text-rambutan-400" role="status"><p>{move || error.get().unwrap_or_else(|| format!("Could not load this {}. Check the source or format.", kind_label(kind)))}</p><code class="break-all">{media.get_value().src}</code><button type="button" class=button_class(ButtonVariant::Ghost, ButtonSize::Small, "ms-2") on:click=move |_| { error.set(None); failed.set(false); loaded.set(!remote); retry.update(|value| *value=value.wrapping_add(1)); }>"Retry"</button></div>
            </Show>
            {media.get_value().transcript.map(|text| view! { <details class="mt-2 text-body-sm text-rambutan-300"><summary class="bruhs-control min-h-[28px] cursor-pointer py-1 text-label-sm">"Transcript"</summary><div class="max-h-80 overflow-auto whitespace-pre-wrap break-words py-2">{text}</div></details> })}
            <Show when=move || zoom.get()>
                {move || sources.get().map(|sources| view! { <ImageZoom media=if remote {sources.shell} else {sources.local} remote open=zoom title=caption.get_value() /> })}
            </Show>
        </figure>
    }
}

#[component]
fn MediaFrame(media: MediaBlock, title: String) -> impl IntoView {
    let frame = NodeRef::<Iframe>::new();
    let interaction = use_context::<ChatInteraction>();
    Effect::new(move |_| {
        if let Some(node) = frame.get() {
            bridge::watch_interaction(
                (*node).clone().into(),
                interaction.map(|interaction| interaction.0),
            );
        }
    });
    let height = if media.kind == MediaKind::Audio {
        72
    } else {
        320
    };
    view! { <iframe node_ref=frame src=media_frame_url(BASE_URL,&media) title=title sandbox="allow-scripts" allow="camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'; display-capture 'none'; fullscreen *" allowfullscreen referrerpolicy="no-referrer" class="w-full rounded-grape border-0" style=format!("height:{height}px") data-response-media-frame=""></iframe> }
}

#[component]
fn NativeMedia(media: MediaBlock, title: String, failed: RwSignal<bool>) -> impl IntoView {
    let tracks = media.tracks;
    let captions = !tracks.is_empty();
    match media.kind {
        MediaKind::Image => view! { <img src=media.src alt=title referrerpolicy="no-referrer" class="max-h-[36rem] max-w-full rounded-grape object-contain" on:error=move |_| failed.set(true) on:load=move |_| failed.set(false) /> }.into_any(),
        MediaKind::Video => view! { <video controls playsinline preload="none" src=media.src poster=media.poster crossorigin=captions.then_some("anonymous") aria-label=title class="max-h-[36rem] max-w-full rounded-grape" on:error=move |_| failed.set(true) on:loadedmetadata=move |_| failed.set(false)>{tracks.into_iter().enumerate().map(|(index,track)|view! { <track src=track.src kind=track.kind srclang=track.language label=track.label default=index==0 /> }).collect_view()}"Your browser cannot play this video."</video> }.into_any(),
        MediaKind::Audio => view! { <audio controls preload="none" src=media.src crossorigin=captions.then_some("anonymous") aria-label=title class="w-full" on:error=move |_| failed.set(true) on:loadedmetadata=move |_| failed.set(false)>{tracks.into_iter().enumerate().map(|(index,track)|view! { <track src=track.src kind=track.kind srclang=track.language label=track.label default=index==0 /> }).collect_view()}"Your browser cannot play this audio."</audio> }.into_any(),
        MediaKind::Pdf => view! { <div><iframe src=media.src.clone() title=title sandbox="" referrerpolicy="no-referrer" class="h-80 w-full resize-y rounded-grape bg-longan-900" on:error=move |_| failed.set(true)></iframe><a href=media.src target="_blank" rel="noopener noreferrer" class="bruhs-control inline-flex min-h-[28px] items-center py-1 text-label-sm text-blueberry-300 underline">"Open PDF"</a></div> }.into_any(),
    }
}

#[component]
fn ImageZoom(
    media: MediaBlock,
    remote: bool,
    open: RwSignal<bool>,
    title: String,
) -> impl IntoView {
    let dialog = NodeRef::<Dialog>::new();
    let scale = RwSignal::new(1.0_f64);
    let error = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        if let Some(node) = dialog.get() {
            if bridge::open_dialog(&(*node).clone().into()).is_err() {
                error.set(Some("Image zoom is unavailable in this browser.".into()));
            }
        }
    });
    view! {
        <dialog node_ref=dialog aria-label="Image zoom" class="fixed inset-0 m-0 h-dvh w-dvw max-h-none max-w-none border-0 bg-longan-950 p-0 text-rambutan-200 backdrop:bg-longan-950/90" on:cancel=move |event: web_sys::Event| { event.prevent_default(); open.set(false); } on:close=move |_| open.set(false)>
            <div class="flex h-full flex-col">
                <div class="flex min-h-[48px] flex-wrap items-center gap-2 px-3"><span class="min-w-0 flex-1 truncate text-label-sm">{title.clone()}</span><crate::ui::Slider min=1.0 max=4.0 step=0.25 value=scale label="Image zoom" class="max-w-40" on_input=Callback::new(move |next|scale.set(next)) /><button type="button" class=button_class(ButtonVariant::Ghost,ButtonSize::Small,"") on:click=move |_|scale.set(1.0)>"Fit"</button><button type="button" class=button_class(ButtonVariant::Outline,ButtonSize::Small,"") autofocus on:click=move |_|open.set(false)>"Close image"</button></div>
                <Show when=move ||error.get().is_some()><p class="p-3 text-label-sm text-lychee-300" role="status">{move ||error.get()}</p></Show>
                <div class="min-h-0 flex-1 overflow-auto"><div style=move ||format!("width:{}%;height:{}%;min-height:100%",scale.get()*100.0,scale.get()*100.0)>
                    {if remote { view! { <iframe src=media_frame_url(BASE_URL,&media) title=title sandbox="allow-scripts" referrerpolicy="no-referrer" class="h-full w-full border-0"></iframe> }.into_any() } else { view! { <img src=media.src alt=title referrerpolicy="no-referrer" class="h-full w-full object-contain" /> }.into_any() }}
                </div></div>
            </div>
        </dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_captured_asset_does_not_replace_a_playing_media_component() {
        let source = MediaBlock {
            kind: MediaKind::Video,
            src: "clip.webm".into(),
            poster: Some("poster.png".into()),
            ..Default::default()
        };
        let bindings = BTreeMap::from([
            ("clip.webm".into(), "/immutable/clip.webm".into()),
            ("poster.png".into(), "/immutable/poster.png".into()),
        ]);
        let rewritten = bound_media(&source, &bindings);
        assert_eq!(
            bound_media(&source, &bindings),
            bound_media(&rewritten, &bindings)
        );
        let changed = MediaBlock {
            src: "/immutable/new-content.webm".into(),
            ..rewritten
        };
        assert_ne!(
            bound_media(&source, &bindings),
            bound_media(&changed, &bindings)
        );
    }
}
