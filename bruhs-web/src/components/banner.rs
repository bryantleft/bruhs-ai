//! Error/warning/info banner system.
//!
//! `<BannerStack />` renders up to 5 banners in a bounded rail that floats over
//! the active view, below its header row, so nothing moves when a banner comes
//! or goes. Banners auto-dismiss after `ttl_ms` milliseconds; users can also
//! dismiss manually via the × button.
//!
//! Usage: push banners via `BannerStackSignal` in the Leptos context.

use std::collections::VecDeque;

use leptos::prelude::*;

/// Classification used for color-coding.
#[derive(Debug, Clone, PartialEq)]
pub enum BannerKind {
    Info,
    Warning,
    Error,
}

/// A single banner entry.
#[derive(Debug, Clone)]
pub struct Banner {
    /// Unique id (UUID v4 string).
    pub id: String,
    pub kind: BannerKind,
    pub message: String,
    /// Auto-dismiss after this many ms. `None` = sticky until manually closed.
    pub ttl_ms: Option<u32>,
    /// One optional action button; choosing it also dismisses the banner.
    pub action: Option<BannerAction>,
}

/// A banner's action button, such as "Open".
#[derive(Debug, Clone, Copy)]
pub struct BannerAction {
    pub label: &'static str,
    pub run: Callback<()>,
}

/// Context newtype wrapping the banner queue signal.
#[derive(Copy, Clone)]
pub struct BannerStackSignal(pub RwSignal<VecDeque<Banner>>);

/// Maximum number of banners visible at once. Oldest are dropped when the cap
/// is exceeded.
const MAX_VISIBLE: usize = 5;

impl BannerStackSignal {
    pub fn push(self, kind: BannerKind, message: impl Into<String>, ttl_ms: Option<u32>) {
        self.push_banner(kind, message.into(), ttl_ms, None);
    }

    /// Like [`Self::push`], with an action button next to the message.
    pub fn push_with_action(
        self,
        kind: BannerKind,
        message: impl Into<String>,
        ttl_ms: Option<u32>,
        action: BannerAction,
    ) {
        self.push_banner(kind, message.into(), ttl_ms, Some(action));
    }

    fn push_banner(
        self,
        kind: BannerKind,
        message: String,
        ttl_ms: Option<u32>,
        action: Option<BannerAction>,
    ) {
        let banner = Banner {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            message,
            ttl_ms,
            action,
        };
        self.0.update(|q| {
            q.push_back(banner);
            // Cap at MAX_VISIBLE — drop oldest.
            while q.len() > MAX_VISIBLE {
                q.pop_front();
            }
        });
    }
}

/// Float over the active view without taking its height. Only the banners take
/// pointer input; long queues scroll within 30% of the viewport.
#[component]
pub fn BannerStack() -> impl IntoView {
    let Some(BannerStackSignal(banners)) = use_context::<BannerStackSignal>() else {
        return view! { <div /> }.into_any();
    };

    view! {
        // role="status": new banners are announced to screen readers without
        // interrupting (WCAG 4.1.3 Status Messages).
        <div
            role="status"
            aria-live="polite"
            aria-label="Notifications"
            data-banner-stack=""
            class="pointer-events-none absolute inset-x-0 top-[52px] z-30 flex max-h-[30dvh] flex-col items-center gap-2 overflow-y-auto px-3"
        >
            <For
                each=move || {
                    let q = banners.get();
                    q.into_iter().collect::<Vec<_>>()
                }
                key=|b| b.id.clone()
                let:banner
            >
                <BannerItem
                    banner=banner
                    on_dismiss={
                        move |id: String| {
                            banners.update(|q| q.retain(|b| b.id != id));
                        }
                    }
                />
            </For>
        </div>
    }
    .into_any()
}

#[component]
fn BannerItem(banner: Banner, on_dismiss: impl Fn(String) + 'static + Clone) -> impl IntoView {
    let id_for_dismiss = banner.id;
    let on_dismiss_btn = on_dismiss.clone();
    let on_dismiss_action = on_dismiss.clone();
    let id_for_action = id_for_dismiss.clone();
    let action = banner.action;

    // Auto-dismiss if ttl is set.
    if let Some(ttl) = banner.ttl_ms {
        let id_t = id_for_dismiss.clone();
        let timeout = gloo_timers::callback::Timeout::new(ttl, move || {
            on_dismiss(id_t);
        });
        // The item's owner drops this timer when the banner is dismissed or
        // evicted. No forgotten callbacks accumulate during reconnect bursts.
        let _dismiss_timeout = StoredValue::new_local(timeout);
    }

    let (bg_class, icon) = match banner.kind {
        BannerKind::Info => (
            "bg-rambutan-900/95 inset-ring inset-ring-rambutan-700 text-rambutan-100",
            "ℹ",
        ),
        BannerKind::Warning => (
            "bg-durian-900/95 inset-ring inset-ring-durian-700 text-durian-100",
            "⚠",
        ),
        // Lychee is the system's error color; mangosteen is reserved for
        // decorative accents (design.bruhs.dev/llms.txt §color roles).
        BannerKind::Error => (
            "bg-lychee-900/95 inset-ring inset-ring-lychee-700 text-lychee-100",
            "✕",
        ),
    };

    let message = banner.message;

    view! {
        <div class=format!(
            "pointer-events-auto flex w-full min-w-0 max-w-[480px] shrink-0 self-center items-center gap-2 rounded-seed px-3 py-2 text-body-sm {bg_class}"
        )>
            <span class="shrink-0 text-label" aria-hidden="true">{icon}</span>
            <span class="min-w-0 flex-1 [overflow-wrap:anywhere]">{message}</span>
            {action.map(|action| view! {
                <button
                    type="button"
                    class="bruhs-control -my-1 min-h-control shrink-0 rounded-seed px-2 text-label-sm underline-offset-2 hover:underline focus-visible:outline focus-visible:outline-persimmon-500 cursor-pointer"
                    on:click=move |_| {
                        action.run.run(());
                        on_dismiss_action(id_for_action.clone());
                    }
                >
                    {action.label}
                </button>
            })}
            <button
                type="button"
                aria-label="Dismiss notification"
                class="bruhs-control -mr-2 -my-1 min-h-control min-w-control shrink-0 rounded-seed opacity-60 hover:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-persimmon-500 transition-opacity cursor-pointer"
                on:click=move |_| on_dismiss_btn(id_for_dismiss.clone())
            >
                "×"
            </button>
        </div>
    }
}
