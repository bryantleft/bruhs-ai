//! Preview surface (⌘⇧J): a URL row over an iframe, with the
//! machine's local TCP listeners offered as one-click targets while no URL
//! is set. An iframe, not a webview: pages that forbid framing fall back to
//! "Open in browser".

use leptos::prelude::*;

use crate::api::{list_local_servers, LocalServer};
use crate::state::{
    load_pref, normalize_preview_url, preview_pref_key, save_pref, ActiveProjectSignal,
};
use crate::ui::{Icon, Input};

/// Sandbox for a page whose origin is `origin` (`None`: it could not be
/// determined). A page from another origin keeps its own origin, so its
/// storage and cookies work. A page that is — or might be — served from the
/// app's own origin or the API server's gets an opaque origin instead:
/// `allow-scripts` plus `allow-same-origin` there would let it reach into
/// this app (its IndexedDB, its DOM, the API as the app). Fails closed.
fn sandbox_for(origin: Option<&str>, app_origins: &[String]) -> &'static str {
    const OPAQUE: &str = "allow-scripts allow-forms allow-popups allow-modals";
    const OWN_ORIGIN: &str =
        "allow-scripts allow-same-origin allow-forms allow-popups allow-modals";
    match origin {
        Some(origin) if origin != "null" && !app_origins.iter().any(|app| app == origin) => {
            OWN_ORIGIN
        }
        _ => OPAQUE,
    }
}

fn preview_sandbox(url: &str) -> &'static str {
    sandbox_for(origin_of(url).as_deref(), &app_origins())
}

/// The origins this app runs with: the page's own and the API server's.
fn app_origins() -> Vec<String> {
    let page = web_sys::window().and_then(|w| w.location().origin().ok());
    page.into_iter()
        .chain(origin_of(crate::api::BASE_URL))
        .collect()
}

/// The origin of `url` as the browser's own URL parser serializes it
/// (backslashes, tabs, trailing dots, numeric IPv4 forms and credentials are
/// all normalized the way navigation would). `None` if it does not parse.
#[cfg(target_arch = "wasm32")]
fn origin_of(url: &str) -> Option<String> {
    web_sys::Url::new(url).ok().map(|url| url.origin())
}

/// Host-target stand-in for [`origin_of`] (unit tests have no browser). It
/// is not a URL parser: it answers `None` — which [`sandbox_for`] treats as
/// "maybe the app" — for anything a browser could normalize into a local
/// origin: `\`, `@`, whitespace or control characters anywhere, and hosts
/// that are loopback, `*.localhost`, end in a dot, are bracketed, or are
/// purely numeric/hex (IPv4 shorthands such as `0x7f.0.0.1`, `2130706433`).
#[cfg(not(target_arch = "wasm32"))]
fn origin_of(url: &str) -> Option<String> {
    if url
        .chars()
        .any(|c| c == '\\' || c == '@' || c.is_ascii_whitespace() || c.is_control())
    {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => "80",
        "https" => "443",
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority.as_str(), None),
    };
    let local = host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with('.')
        || host.starts_with('[')
        || host.starts_with("127.")
        || host
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '.' || c == 'x');
    if local {
        return None;
    }
    Some(match port {
        Some(port) if port.is_empty() || port == default_port => format!("{scheme}://{host}"),
        Some(port) if port.chars().all(|c| c.is_ascii_digit()) => {
            format!("{scheme}://{host}:{port}")
        }
        Some(_) => return None,
        None => format!("{scheme}://{host}"),
    })
}

#[component]
pub fn PreviewPanel() -> impl IntoView {
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let active_thread_id = expect_context::<crate::state::ActiveThreadId>().0;
    let ActiveProjectSignal(active_project) = expect_context::<ActiveProjectSignal>();

    let project_path = Signal::derive(move || {
        let from_thread = active_thread_id.get().and_then(|id| {
            threads.with(|ts| {
                ts.iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.project_path.clone())
            })
        });
        from_thread.or_else(|| active_project.get().map(|p| p.path))
    });
    let pref_key = move || preview_pref_key(project_path.get_untracked().as_deref());

    let url = RwSignal::new(None::<String>);
    let draft = RwSignal::new(String::new());
    let servers = RwSignal::new(Vec::<LocalServer>::new());
    let servers_loading = RwSignal::new(false);
    // Bumped to remount the iframe (a reload that works cross-origin).
    let nonce = RwSignal::new(0u32);

    let load_servers = move || {
        if servers_loading.get_untracked() {
            return;
        }
        servers_loading.set(true);
        wasm_bindgen_futures::spawn_local(async move {
            // Nothing to offer when the scan fails; the URL field still works.
            let list = list_local_servers().await.unwrap_or_else(|error| {
                log::warn!("preview: local server scan failed: {error}");
                Vec::new()
            });
            let _ = servers.try_set(list);
            let _ = servers_loading.try_set(false);
        });
    };

    // Restore the last URL for this project; offer local servers otherwise.
    Effect::new(move |_| {
        project_path.track();
        let saved = load_pref(&pref_key()).filter(|s| !s.is_empty());
        draft.set(saved.clone().unwrap_or_default());
        url.set(saved.clone());
        if saved.is_none() {
            load_servers();
        }
    });

    let go_to = move |raw: String| {
        let Some(normalized) = normalize_preview_url(&raw) else {
            return;
        };
        draft.set(normalized.clone());
        save_pref(&pref_key(), &normalized);
        url.set(Some(normalized));
        nonce.update(|n| *n += 1);
    };
    let open_external = move || {
        if let (Some(window), Some(u)) = (web_sys::window(), url.get_untracked()) {
            let _ = window.open_with_url_and_target(&u, "_blank");
        }
    };
    let clear = move || {
        url.set(None);
        draft.set(String::new());
        save_pref(&pref_key(), "");
        load_servers();
    };

    view! {
        <div class="flex h-full flex-col" data-preview-panel="">
            <form
                class="flex h-10 shrink-0 items-center gap-1.5 border-b border-longan-800/60 px-2"
                on:submit=move |ev| {
                    ev.prevent_default();
                    go_to(draft.get_untracked());
                }
            >
                <Input
                    value=Signal::derive(move || draft.get())
                    on_input=Callback::new(move |v: String| draft.set(v))
                    placeholder="http://localhost:3000"
                    class="h-7 !py-0 !text-code-sm"
                />
                <button
                    type="submit"
                    aria-label="Go"
                    title="Open URL"
                    class="inline-flex h-7 shrink-0 items-center rounded-grape px-2 text-label-sm text-rambutan-200 inset-ring inset-ring-white/10 transition-colors hover:bg-longan-800/60 cursor-pointer"
                >
                    "Go"
                </button>
                <button
                    type="button"
                    aria-label="Reload preview"
                    title="Reload"
                    class="inline-flex size-7 shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 disabled:opacity-40 cursor-pointer"
                    disabled=move || url.get().is_none()
                    on:click=move |_| nonce.update(|n| *n += 1)
                >
                    <Icon name="refresh-cw" size=13 />
                </button>
                <button
                    type="button"
                    aria-label="Open in browser"
                    title="Open in browser"
                    class="inline-flex h-7 shrink-0 items-center gap-1 rounded-grape px-2 text-label-sm text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 disabled:opacity-40 cursor-pointer"
                    disabled=move || url.get().is_none()
                    on:click=move |_| open_external()
                >
                    <Icon name="globe" size=13 />
                    <span aria-hidden="true">"↗"</span>
                </button>
                <button
                    type="button"
                    aria-label="Clear preview"
                    title="Clear"
                    class="inline-flex size-7 shrink-0 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 disabled:opacity-40 cursor-pointer"
                    disabled=move || url.get().is_none()
                    on:click=move |_| clear()
                >
                    <Icon name="x" size=13 />
                </button>
            </form>

            <div class="min-h-0 flex-1 overflow-hidden bg-longan-950">
                {move || match url.get() {
                    Some(u) => {
                        let _ = nonce.get();
                        let sandbox = preview_sandbox(&u);
                        view! {
                            <iframe
                                data-preview-frame=""
                                src=u
                                title="Preview"
                                class="h-full w-full border-0 bg-white"
                                sandbox=sandbox
                                referrerpolicy="no-referrer"
                            ></iframe>
                        }.into_any()
                    }
                    None => view! {
                        <div class="flex h-full flex-col items-center justify-center gap-4 p-6 text-center">
                            <Icon name="globe" size=18 class="text-rambutan-400" />
                            <div>
                                <h3 class="text-label text-rambutan-100">"Enter a local URL to preview"</h3>
                                <p class="mt-1 text-label-sm text-rambutan-500">
                                    "Pages that refuse framing (X-Frame-Options) open in the browser instead."
                                </p>
                            </div>
                            <div class="w-full max-w-xs text-left" aria-label="Local servers" role="group">
                                <div class="mb-1.5 flex items-center justify-between">
                                    <span class="text-label-sm uppercase tracking-wide text-rambutan-500">"Local servers"</span>
                                    <button
                                        type="button"
                                        aria-label="Rescan local servers"
                                        class="inline-flex size-6 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/50 hover:text-rambutan-100 cursor-pointer"
                                        on:click=move |_| load_servers()
                                    >
                                        <Icon name="refresh-cw" size=12 />
                                    </button>
                                </div>
                                {move || {
                                    let list = servers.get();
                                    if list.is_empty() {
                                        return view! {
                                            <p class="rounded-grape bg-longan-900 px-3 py-2 text-label-sm text-rambutan-500 inset-ring inset-ring-white/5">
                                                {if servers_loading.get() { "Scanning…" } else { "Nothing is listening on localhost." }}
                                            </p>
                                        }.into_any();
                                    }
                                    view! {
                                        <ul class="flex flex-col gap-1">
                                            {list.into_iter().map(|s| {
                                                let target = format!("http://localhost:{}", s.port);
                                                let label = target.clone();
                                                view! {
                                                    <li>
                                                        <button
                                                            type="button"
                                                            data-local-server=s.port
                                                            class="flex w-full items-center justify-between gap-3 rounded-grape bg-longan-900 px-3 py-1.5 text-left text-label-sm text-rambutan-200 inset-ring inset-ring-white/5 transition-colors hover:bg-longan-800/70 cursor-pointer"
                                                            on:click=move |_| go_to(target.clone())
                                                        >
                                                            <span class="text-code-sm">{label}</span>
                                                            <span class="truncate text-rambutan-500">{s.command}</span>
                                                        </button>
                                                    </li>
                                                }
                                            }).collect_view()}
                                        </ul>
                                    }.into_any()
                                }}
                            </div>
                        </div>
                    }.into_any(),
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPAQUE: &str = "allow-scripts allow-forms allow-popups allow-modals";

    fn app() -> Vec<String> {
        vec![
            "http://127.0.0.1:8080".to_string(),
            "http://127.0.0.1:3001".to_string(),
        ]
    }

    /// The decision itself, given origins as the browser serializes them.
    #[test]
    fn the_app_origin_is_framed_opaque_and_unknown_origins_fail_closed() {
        assert_eq!(sandbox_for(Some("http://127.0.0.1:3001"), &app()), OPAQUE);
        assert_eq!(sandbox_for(Some("http://127.0.0.1:8080"), &app()), OPAQUE);
        assert_eq!(sandbox_for(None, &app()), OPAQUE);
        assert_eq!(sandbox_for(Some("null"), &app()), OPAQUE);
        for other in [
            "http://localhost:8080",
            "http://127.0.0.1:5173",
            "https://example.com",
        ] {
            assert!(
                sandbox_for(Some(other), &app()).contains("allow-same-origin"),
                "{other}"
            );
        }
    }

    /// Spellings a browser normalizes to the app's origin must never keep
    /// `allow-same-origin` (the review's bypasses of the first version).
    #[test]
    fn spellings_of_a_local_origin_fail_closed() {
        for url in [
            "http://127.0.0.1:3001\\@evil.com/",
            "http://127.0.0.1.:3001/",
            "http://0x7f.0.0.1:3001/",
            "http://2130706433:3001/",
            "http://127.0.0.1:30\t01/",
            "http://127.0.0.1:3001/",
            "http://user@127.0.0.1:3001/",
            "http://localhost:3001/",
            "http://api.bruhs.localhost/",
            "http://[::1]:3001/",
            " http://127.0.0.1:3001/",
            "http://127.1:3001/",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert_eq!(origin_of(url), None, "{url:?}");
            assert_eq!(
                sandbox_for(origin_of(url).as_deref(), &app()),
                OPAQUE,
                "{url:?}"
            );
        }
    }

    #[test]
    fn plain_remote_origins_keep_their_own_origin() {
        assert_eq!(
            origin_of("HTTPS://Example.com:443/a?b#c").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            origin_of("http://example.com:8080/").as_deref(),
            Some("http://example.com:8080")
        );
        let origin = origin_of("https://example.com/");
        assert!(sandbox_for(origin.as_deref(), &app()).contains("allow-same-origin"));
    }
}
