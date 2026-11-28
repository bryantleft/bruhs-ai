//! Usage dashboard: live provider rate-limit pools, model & project token
//! spend breakdown, and daily usage history, backed by `bruhs-server`'s
//! `/api/agent-extensions/usage*` endpoints.

use bruhs_models::agent_extensions::{
    QuotaWindow, UsageLimitAccount, UsageLimitSnapshot, UsageSnapshot, UsageTotal,
};
use bruhs_models::num::ToFloatLossy;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::ui::{Badge, BadgeTone, Button, ButtonSize, ButtonVariant, Icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageTab {
    Limits,
    Breakdown,
    History,
}

impl UsageTab {
    const ALL: [Self; 3] = [Self::Limits, Self::Breakdown, Self::History];

    fn label(self) -> &'static str {
        match self {
            Self::Limits => "Limits & Pools",
            Self::Breakdown => "Models & Projects",
            Self::History => "Daily History",
        }
    }

    fn test_id(self) -> &'static str {
        match self {
            Self::Limits => "usage-tab-limits",
            Self::Breakdown => "usage-tab-breakdown",
            Self::History => "usage-tab-history",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct UsageSummary {
    turns: u64,
    unpriced_turns: u64,
    total_tokens: u64,
    estimated_usd: f64,
    has_priced: bool,
    cache_savings_usd: f64,
}

fn summarize_snapshot(snapshot: &UsageSnapshot) -> UsageSummary {
    let mut summary = UsageSummary {
        cache_savings_usd: snapshot.breakdown.cache_savings.estimated_usd,
        ..Default::default()
    };
    for total in snapshot.by_model.values() {
        summary.turns = summary.turns.saturating_add(total.turns);
        summary.unpriced_turns = summary.unpriced_turns.saturating_add(total.unpriced_turns);
        summary.total_tokens = summary.total_tokens.saturating_add(total_tokens(total));
        if let Some(usd) = total.priced_subtotal() {
            summary.estimated_usd += usd;
            summary.has_priced = true;
        }
    }
    summary
}

fn total_tokens(total: &UsageTotal) -> u64 {
    total
        .input
        .saturating_add(total.output)
        .saturating_add(total.cache_read)
        .saturating_add(total.cache_write)
}

fn format_tokens(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.2}B", count.to_f64_lossy() / 1_000_000_000.0)
    } else if count >= 1_000_000 {
        format!("{:.2}M", count.to_f64_lossy() / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count.to_f64_lossy() / 1_000.0)
    } else {
        count.to_string()
    }
}

fn format_usd(amount: f64) -> String {
    if !amount.is_finite() || amount <= 0.0 {
        "$0.00".to_string()
    } else if amount < 0.01 {
        format!("${amount:.4}")
    } else {
        format!("${amount:.2}")
    }
}

fn format_reset(resets_at: Option<i64>, now_secs: i64) -> Option<String> {
    let at = resets_at?;
    let delta = at.saturating_sub(now_secs);
    if delta <= 0 {
        return Some("Resets soon".to_string());
    }
    let hours = delta / 3600;
    let mins = (delta % 3600) / 60;
    if hours >= 24 {
        let days = hours / 24;
        let rem_hours = hours % 24;
        Some(format!("Resets in {days}d {rem_hours}h"))
    } else if hours > 0 {
        Some(format!("Resets in {hours}h {mins}m"))
    } else {
        Some(format!("Resets in {}m", mins.max(1)))
    }
}

fn quota_bar_color(used_percent: f64) -> &'static str {
    if used_percent >= 85.0 {
        "bg-lychee-400"
    } else if used_percent >= 65.0 {
        "bg-persimmon-400"
    } else {
        "bg-pandan-400"
    }
}

fn quota_text_color(used_percent: f64) -> &'static str {
    if used_percent >= 85.0 {
        "text-lychee-300"
    } else if used_percent >= 65.0 {
        "text-persimmon-300"
    } else {
        "text-pandan-300"
    }
}

fn sorted_totals(
    map: &std::collections::BTreeMap<String, UsageTotal>,
) -> Vec<(String, UsageTotal)> {
    let mut rows: Vec<(String, UsageTotal)> =
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    rows.sort_by(|a, b| {
        let a_cost = a.1.priced_subtotal().unwrap_or(-1.0);
        let b_cost = b.1.priced_subtotal().unwrap_or(-1.0);
        b_cost
            .total_cmp(&a_cost)
            .then_with(|| total_tokens(&b.1).cmp(&total_tokens(&a.1)))
            .then_with(|| a.0.cmp(&b.0))
    });
    rows
}

fn now_unix_secs() -> i64 {
    bruhs_models::EpochMs::saturating_from_f64(js_sys::Date::now()).as_millis() / 1000
}

fn render_quota_window(window: QuotaWindow, now_secs: i64) -> impl IntoView {
    let pct = window.used_percent.clamp(0.0, 100.0);
    let width_style = format!("width:{pct:.1}%");
    let reset = format_reset(window.resets_at, now_secs);
    let bar_color = quota_bar_color(pct);
    let text_color = quota_text_color(pct);
    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex items-center justify-between gap-2 text-label-sm">
                <span class="min-w-0 truncate text-rambutan-200">{window.name}</span>
                <span class="flex shrink-0 items-center gap-2">
                    {reset.map(|label| view! {
                        <span class="text-rambutan-500">{label}</span>
                    })}
                    <span class=format!("font-mono font-medium {text_color}")>
                        {format!("{pct:.0}%")}
                    </span>
                </span>
            </div>
            <div class="h-1.5 w-full overflow-hidden rounded-orb bg-longan-800">
                <div
                    class=format!("h-full rounded-orb transition-all {bar_color}")
                    style=width_style
                ></div>
            </div>
        </div>
    }
}

#[component]
pub fn UsagePage() -> impl IntoView {
    let active_tab = RwSignal::new(UsageTab::Limits);
    let snapshot = RwSignal::new(None::<UsageSnapshot>);
    let pools = RwSignal::new(None::<UsageLimitSnapshot>);
    let loading = RwSignal::new(true);
    let refreshing = RwSignal::new(false);
    let error_banner = RwSignal::new(None::<String>);
    let credit_notice = RwSignal::new(None::<String>);
    let redeeming_handle = RwSignal::new(None::<uuid::Uuid>);
    let selected_env = RwSignal::new(String::new());
    let selected_provider = RwSignal::new(String::new());

    let fetch_all = move |force_refresh: bool| {
        if force_refresh {
            refreshing.set(true);
        } else {
            loading.set(true);
        }
        error_banner.set(None);
        let env_filter = selected_env.get_untracked();
        let prov_filter = selected_provider.get_untracked();
        spawn_local(async move {
            let usage_res = if force_refresh {
                api::refresh_usage().await
            } else {
                api::usage_snapshot().await
            };
            let pools_res = api::usage_pools(
                (!env_filter.is_empty()).then_some(env_filter.as_str()),
                (!prov_filter.is_empty()).then_some(prov_filter.as_str()),
            )
            .await;

            match usage_res {
                Ok(data) => snapshot.set(Some(data)),
                Err(err) => {
                    error_banner.set(Some(format!(
                        "Could not reach bruhs-server for usage data ({err})."
                    )));
                }
            }
            if let Ok(pool_data) = pools_res {
                pools.set(Some(pool_data));
            }
            loading.set(false);
            refreshing.set(false);
        });
    };

    // Initial load.
    Effect::new(move |_| {
        fetch_all(false);
    });

    let reload_pools = move || {
        let env_filter = selected_env.get_untracked();
        let prov_filter = selected_provider.get_untracked();
        spawn_local(async move {
            if let Ok(pool_data) = api::usage_pools(
                (!env_filter.is_empty()).then_some(env_filter.as_str()),
                (!prov_filter.is_empty()).then_some(prov_filter.as_str()),
            )
            .await
            {
                pools.set(Some(pool_data));
            }
        });
    };

    let summary = Memo::new(move |_| {
        snapshot.with(|snap| snap.as_ref().map(summarize_snapshot).unwrap_or_default())
    });

    let account_count =
        Memo::new(move |_| pools.with(|p| p.as_ref().map_or(0, |snap| snap.accounts.len())));
    let provider_count =
        Memo::new(move |_| pools.with(|p| p.as_ref().map_or(0, |snap| snap.providers.len())));

    let redeem_credit = move |handle: uuid::Uuid| {
        redeeming_handle.set(Some(handle));
        credit_notice.set(None);
        spawn_local(async move {
            match api::redeem_usage_credit(handle).await {
                Ok(msg) => {
                    credit_notice.set(Some(msg));
                    let env_filter = selected_env.get_untracked();
                    let prov_filter = selected_provider.get_untracked();
                    if let Ok(pool_data) = api::usage_pools(
                        (!env_filter.is_empty()).then_some(env_filter.as_str()),
                        (!prov_filter.is_empty()).then_some(prov_filter.as_str()),
                    )
                    .await
                    {
                        pools.set(Some(pool_data));
                    }
                }
                Err(err) => {
                    credit_notice.set(Some(format!("Could not redeem credit: {err}")));
                }
            }
            redeeming_handle.set(None);
        });
    };

    view! {
        <div
            data-testid="usage-page"
            class="flex min-h-0 flex-1 flex-col overflow-y-auto bg-longan-950 px-6 py-6"
        >
            <div class="mx-auto flex w-full max-w-5xl flex-col gap-5">
                // Page header
                <header class="flex flex-wrap items-center justify-between gap-3">
                    <div>
                        <h1 class="text-h3 font-semibold text-rambutan-100">"Usage"</h1>
                        <p class="mt-0.5 text-label-sm text-rambutan-500">
                            "Live provider rate limits, account pools, token spend, and daily history."
                        </p>
                    </div>
                    <Button
                        variant=ButtonVariant::Outline
                        size=ButtonSize::Small
                        disabled=Signal::derive(move || refreshing.get() || loading.get())
                        on_click=Callback::new(move |_| fetch_all(true))
                    >
                        <span class=move || {
                            if refreshing.get() {
                                "inline-flex motion-safe:animate-spin"
                            } else {
                                "inline-flex"
                            }
                        }>
                            <Icon name="refresh-cw" size=14 />
                        </span>
                        <span>{move || if refreshing.get() { "Refreshing…" } else { "Refresh" }}</span>
                    </Button>
                </header>

                // Offline / error banner
                <Show when=move || error_banner.get().is_some()>
                    <div
                        role="status"
                        class="flex items-center gap-2.5 rounded-lychee bg-longan-900 px-4 py-3 text-label-sm text-rambutan-300 inset-ring inset-ring-persimmon-500/25"
                    >
                        <Icon name="alert-triangle" size=15 class="shrink-0 text-persimmon-400" />
                        <span>{move || error_banner.get().unwrap_or_default()}</span>
                    </div>
                </Show>

                // Credit redemption notice
                <Show when=move || credit_notice.get().is_some()>
                    <div
                        role="status"
                        class="flex items-center justify-between gap-2.5 rounded-lychee bg-longan-900 px-4 py-2.5 text-label-sm text-rambutan-200 inset-ring inset-ring-white/10"
                    >
                        <span>{move || credit_notice.get().unwrap_or_default()}</span>
                        <button
                            type="button"
                            aria-label="Dismiss notice"
                            class="inline-flex size-6 items-center justify-center rounded-seed text-rambutan-500 hover:bg-longan-800 hover:text-rambutan-100 cursor-pointer"
                            on:click=move |_| credit_notice.set(None)
                        >
                            <Icon name="x" size=14 />
                        </button>
                    </div>
                </Show>

                // Summary KPI strip
                <section
                    aria-label="Usage summary"
                    class="grid grid-cols-2 gap-3 sm:grid-cols-4"
                >
                    <div class="rounded-lychee bg-longan-900 px-4 py-3.5 inset-ring inset-ring-white/5">
                        <div class="text-label-sm text-rambutan-500">"Estimated spend"</div>
                        <div class="mt-1 font-mono text-h3 font-semibold text-rambutan-100">
                            {move || {
                                let s = summary.get();
                                if s.has_priced { format_usd(s.estimated_usd) } else { "—".to_string() }
                            }}
                        </div>
                        <div class="mt-0.5 text-[11px] text-pandan-400">
                            {move || {
                                let saved = summary.get().cache_savings_usd;
                                if saved >= 0.01 {
                                    format!("{} saved via cache", format_usd(saved))
                                } else {
                                    "Priced model turns".to_string()
                                }
                            }}
                        </div>
                    </div>

                    <div class="rounded-lychee bg-longan-900 px-4 py-3.5 inset-ring inset-ring-white/5">
                        <div class="text-label-sm text-rambutan-500">"Total tokens"</div>
                        <div class="mt-1 font-mono text-h3 font-semibold text-rambutan-100">
                            {move || format_tokens(summary.get().total_tokens)}
                        </div>
                        <div class="mt-0.5 text-[11px] text-rambutan-500">
                            "Input, output & cache"
                        </div>
                    </div>

                    <div class="rounded-lychee bg-longan-900 px-4 py-3.5 inset-ring inset-ring-white/5">
                        <div class="text-label-sm text-rambutan-500">"Turns recorded"</div>
                        <div class="mt-1 font-mono text-h3 font-semibold text-rambutan-100">
                            {move || summary.get().turns.to_string()}
                        </div>
                        <div class="mt-0.5 text-[11px] text-rambutan-500">
                            {move || {
                                let unpriced = summary.get().unpriced_turns;
                                if unpriced > 0 {
                                    format!("{unpriced} unpriced")
                                } else {
                                    "All turns priced".to_string()
                                }
                            }}
                        </div>
                    </div>

                    <div class="rounded-lychee bg-longan-900 px-4 py-3.5 inset-ring inset-ring-white/5">
                        <div class="text-label-sm text-rambutan-500">"Connected accounts"</div>
                        <div class="mt-1 font-mono text-h3 font-semibold text-rambutan-100">
                            {move || account_count.get().to_string()}
                        </div>
                        <div class="mt-0.5 text-[11px] text-rambutan-500">
                            {move || {
                                let p = provider_count.get();
                                match p {
                                    0 => "No providers polled".to_string(),
                                    1 => "1 provider".to_string(),
                                    n => format!("{n} providers"),
                                }
                            }}
                        </div>
                    </div>
                </section>

                // Tabs
                <div
                    role="tablist"
                    aria-label="Usage views"
                    class="flex items-center gap-1 border-b border-longan-800/80 pb-2"
                >
                    {UsageTab::ALL.into_iter().map(|tab| {
                        let is_active = Signal::derive(move || active_tab.get() == tab);
                        view! {
                            <button
                                type="button"
                                role="tab"
                                data-testid=tab.test_id()
                                aria-selected=move || is_active.get().to_string()
                                class=move || format!(
                                    "rounded-grape px-3 py-1.5 text-label-sm font-medium transition-colors cursor-pointer focus-visible:outline-1 focus-visible:outline-persimmon-400 {}",
                                    if is_active.get() {
                                        "bg-longan-800/80 text-rambutan-100"
                                    } else {
                                        "text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-200"
                                    }
                                )
                                on:click=move |_| active_tab.set(tab)
                            >
                                {tab.label()}
                            </button>
                        }
                    }).collect_view()}
                </div>

                // Tab body
                {move || match active_tab.get() {
                    UsageTab::Limits => {
                        let now_secs = now_unix_secs();
                        let pool_snap = pools.get();
                        let fallback_limits = snapshot
                            .get()
                            .map(|s| s.limits)
                            .unwrap_or_default();
                        let envs = pool_snap
                            .as_ref()
                            .map(|p| p.environments.clone())
                            .unwrap_or_default();
                        let provs = pool_snap
                            .as_ref()
                            .map(|p| p.providers.clone())
                            .unwrap_or_default();
                        let accounts: Vec<UsageLimitAccount> = pool_snap
                            .as_ref()
                            .map(|p| p.accounts.clone())
                            .unwrap_or_default();
                        let notices = pool_snap
                            .as_ref()
                            .map(|p| p.notices.clone())
                            .unwrap_or_default();
                        let external = pool_snap
                            .as_ref()
                            .map(|p| p.external.clone())
                            .unwrap_or_default();
                        view! {
                            <div class="flex flex-col gap-4">
                                // Filters when multiple environments or providers exist
                                {((envs.len() > 1) || (provs.len() > 1)).then(|| view! {
                                    <div class="flex flex-wrap items-center gap-2">
                                        {(envs.len() > 1).then(|| {
                                            let envs_clone = envs.clone();
                                            view! {
                                                <div class="flex flex-wrap items-center gap-1">
                                                    <button
                                                        type="button"
                                                        class=move || format!(
                                                            "rounded-orb px-2.5 py-1 text-[11px] font-medium transition-colors cursor-pointer {}",
                                                            if selected_env.get().is_empty() {
                                                                "bg-longan-700 text-rambutan-100"
                                                            } else {
                                                                "bg-longan-900 text-rambutan-500 hover:text-rambutan-200"
                                                            }
                                                        )
                                                        on:click=move |_| {
                                                            selected_env.set(String::new());
                                                            reload_pools();
                                                        }
                                                    >
                                                        "All environments"
                                                    </button>
                                                    {envs_clone.into_iter().map(|env_name| {
                                                        let cmp = env_name.clone();
                                                        let set_val = env_name.clone();
                                                        view! {
                                                            <button
                                                                type="button"
                                                                class=move || format!(
                                                                    "rounded-orb px-2.5 py-1 text-[11px] font-medium transition-colors cursor-pointer {}",
                                                                    if selected_env.get() == cmp {
                                                                        "bg-longan-700 text-rambutan-100"
                                                                    } else {
                                                                        "bg-longan-900 text-rambutan-500 hover:text-rambutan-200"
                                                                    }
                                                                )
                                                                on:click=move |_| {
                                                                    selected_env.set(set_val.clone());
                                                                    reload_pools();
                                                                }
                                                            >
                                                                {env_name}
                                                            </button>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            }
                                        })}
                                        {(provs.len() > 1).then(|| {
                                            let provs_clone = provs.clone();
                                            view! {
                                                <div class="flex flex-wrap items-center gap-1">
                                                    <button
                                                        type="button"
                                                        class=move || format!(
                                                            "rounded-orb px-2.5 py-1 text-[11px] font-medium transition-colors cursor-pointer {}",
                                                            if selected_provider.get().is_empty() {
                                                                "bg-longan-700 text-rambutan-100"
                                                            } else {
                                                                "bg-longan-900 text-rambutan-500 hover:text-rambutan-200"
                                                            }
                                                        )
                                                        on:click=move |_| {
                                                            selected_provider.set(String::new());
                                                            reload_pools();
                                                        }
                                                    >
                                                        "All providers"
                                                    </button>
                                                    {provs_clone.into_iter().map(|prov_name| {
                                                        let cmp = prov_name.clone();
                                                        let set_val = prov_name.clone();
                                                        view! {
                                                            <button
                                                                type="button"
                                                                class=move || format!(
                                                                    "rounded-orb px-2.5 py-1 text-[11px] font-medium transition-colors cursor-pointer {}",
                                                                    if selected_provider.get() == cmp {
                                                                        "bg-longan-700 text-rambutan-100"
                                                                    } else {
                                                                        "bg-longan-900 text-rambutan-500 hover:text-rambutan-200"
                                                                    }
                                                                )
                                                                on:click=move |_| {
                                                                    selected_provider.set(set_val.clone());
                                                                    reload_pools();
                                                                }
                                                            >
                                                                {prov_name}
                                                            </button>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            }
                                        })}
                                    </div>
                                })}

                                // Account pool cards
                                {if !accounts.is_empty() {
                                    view! {
                                        <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
                                            {accounts.into_iter().map(|account| {
                                                let reset_choice = account.reset.clone();
                                                let subtitle = account
                                                    .email
                                                    .clone()
                                                    .or_else(|| account.workspace.clone());
                                                let sources_line = if !account.sources.is_empty() {
                                                    Some(account.sources.join(" · "))
                                                } else if account.on_device {
                                                    Some("This computer".to_string())
                                                } else {
                                                    None
                                                };
                                                view! {
                                                    <article class="flex flex-col gap-3 rounded-lychee bg-longan-900 p-4 inset-ring inset-ring-white/5">
                                                        <div class="flex items-start justify-between gap-2">
                                                            <div class="min-w-0">
                                                                <div class="flex flex-wrap items-center gap-1.5">
                                                                    <h2 class="truncate text-label font-semibold text-rambutan-100">
                                                                        {account.label}
                                                                    </h2>
                                                                    <Badge tone=BadgeTone::Neutral>{account.provider}</Badge>
                                                                    {account.plan.map(|plan| view! {
                                                                        <Badge tone=BadgeTone::Accent>{plan}</Badge>
                                                                    })}
                                                                    {account.zero_spend_cap.then(|| view! {
                                                                        <Badge tone=BadgeTone::Warning>"$0 cap"</Badge>
                                                                    })}
                                                                </div>
                                                                {subtitle.map(|text| view! {
                                                                    <p class="mt-0.5 truncate text-[11px] text-rambutan-500">{text}</p>
                                                                })}
                                                                {sources_line.map(|src| view! {
                                                                    <p class="mt-0.5 truncate text-[11px] text-rambutan-500">{src}</p>
                                                                })}
                                                            </div>
                                                            {reset_choice.map(|choice| {
                                                                let handle = choice.handle;
                                                                let hub_label = choice.hub;
                                                                view! {
                                                                    <Button
                                                                        variant=ButtonVariant::Outline
                                                                        size=ButtonSize::Small
                                                                        disabled=Signal::derive(move || redeeming_handle.get() == Some(handle))
                                                                        on_click=Callback::new(move |_| redeem_credit(handle))
                                                                    >
                                                                        {move || if redeeming_handle.get() == Some(handle) {
                                                                            "Redeeming…".to_string()
                                                                        } else {
                                                                            format!("Reset ({hub_label})")
                                                                        }}
                                                                    </Button>
                                                                }
                                                            })}
                                                        </div>
                                                        {if account.windows.is_empty() {
                                                            view! {
                                                                <p class="text-label-sm text-rambutan-500">
                                                                    "No active quota windows reported for this account."
                                                                </p>
                                                            }.into_any()
                                                        } else {
                                                            view! {
                                                                <div class="flex flex-col gap-2.5">
                                                                    {account.windows.into_iter().map(|w| render_quota_window(w, now_secs)).collect_view()}
                                                                </div>
                                                            }.into_any()
                                                        }}
                                                    </article>
                                                }
                                            }).collect_view()}
                                        </div>
                                    }.into_any()
                                } else if !fallback_limits.is_empty() {
                                    view! {
                                        <div class="flex flex-col gap-3 rounded-lychee bg-longan-900 p-4 inset-ring inset-ring-white/5">
                                            <h2 class="text-label font-semibold text-rambutan-100">"Provider quota windows"</h2>
                                            {fallback_limits.into_iter().map(|w| render_quota_window(w, now_secs)).collect_view()}
                                        </div>
                                    }.into_any()
                                } else if loading.get() {
                                    view! {
                                        <div class="rounded-lychee bg-longan-900 px-5 py-8 text-center text-label-sm text-rambutan-500 inset-ring inset-ring-white/5">
                                            "Loading provider limits and usage history…"
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div class="flex flex-col items-center gap-2 rounded-lychee bg-longan-900 px-5 py-10 text-center inset-ring inset-ring-white/5">
                                            <Icon name="activity" size=20 class="text-rambutan-500" />
                                            <p class="text-label text-rambutan-200">"No live rate-limit windows yet"</p>
                                            <p class="max-w-md text-label-sm text-rambutan-500">
                                                "Run a session with Claude, Codex, or another connected provider — or link a usage hub — and hit Refresh to inspect live windows."
                                            </p>
                                        </div>
                                    }.into_any()
                                }}

                                // Notices & external usage management links
                                {(!notices.is_empty()).then(|| view! {
                                    <div class="flex flex-col gap-1.5 rounded-lychee bg-longan-900/70 px-4 py-3 text-label-sm text-rambutan-400 inset-ring inset-ring-white/5">
                                        {notices.into_iter().map(|n| view! { <p>{n}</p> }).collect_view()}
                                    </div>
                                })}
                                {(!external.is_empty()).then(|| view! {
                                    <div class="flex flex-wrap items-center gap-2">
                                        {external.into_iter().map(|page| view! {
                                            <a
                                                href=page.url
                                                target="_blank"
                                                rel="noopener noreferrer"
                                                class="inline-flex items-center gap-1.5 rounded-grape bg-longan-900 px-3 py-1.5 text-label-sm text-rambutan-200 inset-ring inset-ring-white/10 hover:bg-longan-800"
                                            >
                                                <span>{page.label}</span>
                                                <Icon name="external-link" size=12 class="text-rambutan-500" />
                                            </a>
                                        }).collect_view()}
                                    </div>
                                })}
                            </div>
                        }.into_any()
                    }
                    UsageTab::Breakdown => {
                        let snap = snapshot.get().unwrap_or_default();
                        let models = sorted_totals(&snap.by_model);
                        let projects = sorted_totals(&snap.by_project);
                        let max_model_tokens = models.iter().map(|(_, v)| total_tokens(v)).max().unwrap_or(1).max(1);
                        let max_project_tokens = projects.iter().map(|(_, v)| total_tokens(v)).max().unwrap_or(1).max(1);
                        view! {
                            <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
                                // Models table
                                <section class="flex flex-col gap-3 rounded-lychee bg-longan-900 p-4 inset-ring inset-ring-white/5">
                                    <div class="flex items-center justify-between">
                                        <h2 class="text-label font-semibold text-rambutan-100">"By model"</h2>
                                        <span class="text-label-sm text-rambutan-500">
                                            {format!("{} models", models.len())}
                                        </span>
                                    </div>
                                    {if models.is_empty() {
                                        view! {
                                            <p class="py-6 text-center text-label-sm text-rambutan-500">
                                                "No model usage recorded yet."
                                            </p>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div class="flex flex-col gap-2.5">
                                                {models.into_iter().map(|(model, total)| {
                                                    let tokens = total_tokens(&total);
                                                    let pct = (tokens.to_f64_lossy() / max_model_tokens.to_f64_lossy() * 100.0).clamp(2.0, 100.0);
                                                    let cost = total.priced_subtotal().map_or_else(|| "—".to_string(), format_usd);
                                                    view! {
                                                        <div class="flex flex-col gap-1">
                                                            <div class="flex items-center justify-between gap-2 text-label-sm">
                                                                <span class="min-w-0 truncate font-mono text-rambutan-200">{model}</span>
                                                                <span class="flex shrink-0 items-center gap-3 font-mono text-[11px] text-rambutan-400">
                                                                    <span>{format!("{}t", total.turns)}</span>
                                                                    <span>{format_tokens(tokens)}</span>
                                                                    <span class="text-rambutan-100">{cost}</span>
                                                                </span>
                                                            </div>
                                                            <div class="h-1.5 w-full overflow-hidden rounded-orb bg-longan-800">
                                                                <div class="h-full rounded-orb bg-persimmon-400/80" style=format!("width:{pct:.1}%")></div>
                                                            </div>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </section>

                                // Projects table
                                <section class="flex flex-col gap-3 rounded-lychee bg-longan-900 p-4 inset-ring inset-ring-white/5">
                                    <div class="flex items-center justify-between">
                                        <h2 class="text-label font-semibold text-rambutan-100">"By project"</h2>
                                        <span class="text-label-sm text-rambutan-500">
                                            {format!("{} projects", projects.len())}
                                        </span>
                                    </div>
                                    {if projects.is_empty() {
                                        view! {
                                            <p class="py-6 text-center text-label-sm text-rambutan-500">
                                                "No project usage recorded yet."
                                            </p>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div class="flex flex-col gap-2.5">
                                                {projects.into_iter().map(|(project, total)| {
                                                    let tokens = total_tokens(&total);
                                                    let pct = (tokens.to_f64_lossy() / max_project_tokens.to_f64_lossy() * 100.0).clamp(2.0, 100.0);
                                                    let cost = total.priced_subtotal().map_or_else(|| "—".to_string(), format_usd);
                                                    view! {
                                                        <div class="flex flex-col gap-1">
                                                            <div class="flex items-center justify-between gap-2 text-label-sm">
                                                                <span class="min-w-0 truncate text-rambutan-200">{project}</span>
                                                                <span class="flex shrink-0 items-center gap-3 font-mono text-[11px] text-rambutan-400">
                                                                    <span>{format!("{}t", total.turns)}</span>
                                                                    <span>{format_tokens(tokens)}</span>
                                                                    <span class="text-rambutan-100">{cost}</span>
                                                                </span>
                                                            </div>
                                                            <div class="h-1.5 w-full overflow-hidden rounded-orb bg-longan-800">
                                                                <div class="h-full rounded-orb bg-blueberry-400/80" style=format!("width:{pct:.1}%")></div>
                                                            </div>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </section>
                            </div>
                        }.into_any()
                    }
                    UsageTab::History => {
                        let snap = snapshot.get().unwrap_or_default();
                        let mut days: Vec<(String, UsageTotal)> = snap
                            .by_day
                            .into_iter()
                            .collect();
                        // Most recent 14 days first.
                        days.reverse();
                        days.truncate(14);
                        let max_day_tokens = days.iter().map(|(_, v)| total_tokens(v)).max().unwrap_or(1).max(1);
                        let cost_by_type = snap.breakdown.cost_by_type;
                        view! {
                            <div class="flex flex-col gap-4">
                                <section class="flex flex-col gap-3 rounded-lychee bg-longan-900 p-4 inset-ring inset-ring-white/5">
                                    <div class="flex items-center justify-between">
                                        <h2 class="text-label font-semibold text-rambutan-100">"Recent daily activity"</h2>
                                        <span class="text-label-sm text-rambutan-500">"Last 14 active days"</span>
                                    </div>
                                    {if days.is_empty() {
                                        view! {
                                            <p class="py-6 text-center text-label-sm text-rambutan-500">
                                                "No daily history recorded yet."
                                            </p>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div class="flex flex-col gap-2">
                                                {days.into_iter().map(|(day, total)| {
                                                    let tokens = total_tokens(&total);
                                                    let pct = (tokens.to_f64_lossy() / max_day_tokens.to_f64_lossy() * 100.0).clamp(2.0, 100.0);
                                                    let cost = total.priced_subtotal().map_or_else(|| "—".to_string(), format_usd);
                                                    view! {
                                                        <div class="grid grid-cols-[96px_1fr_auto] items-center gap-3 text-label-sm">
                                                            <span class="font-mono text-[11px] text-rambutan-400">{day}</span>
                                                            <div class="h-2 w-full overflow-hidden rounded-orb bg-longan-800">
                                                                <div class="h-full rounded-orb bg-persimmon-400/80" style=format!("width:{pct:.1}%")></div>
                                                            </div>
                                                            <span class="flex items-center gap-3 font-mono text-[11px] text-rambutan-300">
                                                                <span class="text-rambutan-500">{format!("{}t", total.turns)}</span>
                                                                <span>{format_tokens(tokens)}</span>
                                                                <span class="w-16 text-right text-rambutan-100">{cost}</span>
                                                            </span>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </section>

                                <section class="grid grid-cols-2 gap-3 sm:grid-cols-4">
                                    <div class="rounded-lychee bg-longan-900 px-4 py-3 inset-ring inset-ring-white/5">
                                        <div class="text-[11px] text-rambutan-500">"Input cost"</div>
                                        <div class="mt-0.5 font-mono text-label font-semibold text-rambutan-100">
                                            {format_usd(cost_by_type.input)}
                                        </div>
                                    </div>
                                    <div class="rounded-lychee bg-longan-900 px-4 py-3 inset-ring inset-ring-white/5">
                                        <div class="text-[11px] text-rambutan-500">"Output cost"</div>
                                        <div class="mt-0.5 font-mono text-label font-semibold text-rambutan-100">
                                            {format_usd(cost_by_type.output)}
                                        </div>
                                    </div>
                                    <div class="rounded-lychee bg-longan-900 px-4 py-3 inset-ring inset-ring-white/5">
                                        <div class="text-[11px] text-rambutan-500">"Cache read"</div>
                                        <div class="mt-0.5 font-mono text-label font-semibold text-rambutan-100">
                                            {format_usd(cost_by_type.cache_read)}
                                        </div>
                                    </div>
                                    <div class="rounded-lychee bg-longan-900 px-4 py-3 inset-ring inset-ring-white/5">
                                        <div class="text-[11px] text-rambutan-500">"Cache write"</div>
                                        <div class="mt-0.5 font-mono text-label font-semibold text-rambutan-100">
                                            {format_usd(cost_by_type.cache_write)}
                                        </div>
                                    </div>
                                </section>
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tokens_usd_and_reset_countdowns() {
        assert_eq!(format_tokens(420), "420");
        assert_eq!(format_tokens(12_500), "12.5K");
        assert_eq!(format_tokens(3_450_000), "3.45M");
        assert_eq!(format_usd(0.0), "$0.00");
        assert_eq!(format_usd(0.0025), "$0.0025");
        assert_eq!(format_usd(14.728), "$14.73");
        assert_eq!(format_reset(Some(1000), 1000), Some("Resets soon".into()));
        assert_eq!(
            format_reset(Some(1000 + 3 * 3600 + 15 * 60), 1000),
            Some("Resets in 3h 15m".into())
        );
        assert_eq!(
            format_reset(Some(1000 + 2 * 86400 + 5 * 3600), 1000),
            Some("Resets in 2d 5h".into())
        );
    }

    #[test]
    fn summarizes_usage_snapshot_totals() {
        let mut snap = UsageSnapshot::default();
        snap.by_model.insert(
            "claude-sonnet-4-6".into(),
            UsageTotal {
                turns: 4,
                unpriced_turns: 0,
                input: 10_000,
                output: 2_000,
                cache_read: 50_000,
                cache_write: 1_000,
                estimated_usd: Some(0.42),
                priced_usd: Some(0.42),
            },
        );
        snap.by_model.insert(
            "gpt-5.4".into(),
            UsageTotal {
                turns: 2,
                unpriced_turns: 1,
                input: 5_000,
                output: 1_000,
                cache_read: 0,
                cache_write: 0,
                estimated_usd: None,
                priced_usd: Some(0.18),
            },
        );
        snap.breakdown.cache_savings.estimated_usd = 0.15;

        let summary = summarize_snapshot(&snap);
        assert_eq!(summary.turns, 6);
        assert_eq!(summary.unpriced_turns, 1);
        assert_eq!(summary.total_tokens, 69_000);
        assert!(summary.has_priced);
        assert!((summary.estimated_usd - 0.60).abs() < 1e-6);
        assert!((summary.cache_savings_usd - 0.15).abs() < 1e-6);
    }
}
