//! Model picker popover anchored to the composer's model chip:
//! a search field, provider-grouped rows, ↑/↓ + Enter, and a one-line
//! auth status for the provider under the cursor.

use leptos::html::Input;
use leptos::prelude::*;

use bruhs_models::agent::{provider_for_model_id, AgentProvider, ModelInfo, MODEL_CATALOG};

use crate::auth::AuthStatus;
use crate::components::provider::{provider_label, provider_logo_src, PROVIDER_LOGO_CLASS};
use crate::state::{CodexAuthSignal, ModelSelectOpen, SetModelSelectOpen};
use crate::ui::{Icon, Kbd};

/// Models whose label, id or provider contains every word of `query`.
pub fn filter_models<'a>(models: &'a [ModelInfo], query: &str) -> Vec<&'a ModelInfo> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    models
        .iter()
        .filter(|m| {
            let hay = format!(
                "{} {} {}",
                m.label.to_lowercase(),
                m.id.to_lowercase(),
                provider_label(m.provider).to_lowercase()
            );
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Group in first-seen provider order (Anthropic first in the default list).
pub fn group_models<'a>(models: &[&'a ModelInfo]) -> Vec<(AgentProvider, Vec<&'a ModelInfo>)> {
    let mut groups: Vec<(AgentProvider, Vec<&'a ModelInfo>)> = Vec::new();
    for &m in models {
        match groups.iter_mut().find(|(p, _)| *p == m.provider) {
            Some((_, list)) => list.push(m),
            None => groups.push((m.provider, vec![m])),
        }
    }
    groups
}

/// Short auth line for a provider.
pub fn auth_line(
    provider: AgentProvider,
    claude: &AuthStatus,
    codex_authenticated: bool,
) -> String {
    match provider {
        AgentProvider::Codex => {
            if codex_authenticated {
                "Codex CLI signed in".to_string()
            } else {
                "Codex CLI not signed in — run `codex login`".to_string()
            }
        }
        AgentProvider::Claude => {
            if claude.authenticated {
                match &claude.user {
                    Some(u) => format!("Claude Code signed in as {u}"),
                    None => "Claude Code signed in".to_string(),
                }
            } else {
                "Claude Code not signed in — run `claude login`".to_string()
            }
        }
    }
}

#[component]
pub fn ModelPicker() -> impl IntoView {
    let ModelSelectOpen(open) = expect_context::<ModelSelectOpen>();
    let SetModelSelectOpen(set_open) = expect_context::<SetModelSelectOpen>();
    let current_model = expect_context::<crate::state::CurrentModel>().0;
    let set_current_model = expect_context::<crate::state::SetCurrentModel>().0;
    let crate::state::DefaultModel(default_model) = expect_context::<crate::state::DefaultModel>();
    let auth_status = expect_context::<crate::state::ClaudeAuthSignal>().0;
    let CodexAuthSignal(codex_auth) = expect_context::<CodexAuthSignal>();

    let (query, set_query) = signal(String::new());
    let (highlight, set_highlight) = signal(0usize);
    let input_ref = NodeRef::<Input>::new();

    let flat = Memo::new(move |_| filter_models(MODEL_CATALOG, &query.get()));
    let groups = Memo::new(move |_| group_models(&flat.get()));

    // Opening resets the search and parks the cursor on the current model.
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        set_query.set(String::new());
        let cur = current_model.get_untracked();
        let idx = MODEL_CATALOG.iter().position(|m| m.id == cur).unwrap_or(0);
        set_highlight.set(idx);
        gloo_timers::callback::Timeout::new(0, move || {
            if let Some(Some(el)) = input_ref.try_get_untracked() {
                let _ = el.focus();
            }
        })
        .forget();
    });
    // Typing re-anchors the cursor to the first match (only on a real
    // change, so opening keeps the current model highlighted).
    Effect::new(move |prev: Option<String>| {
        let q = query.get();
        if prev.is_some_and(|p| p != q) {
            set_highlight.set(0);
        }
        q
    });

    // The pick is this thread's model, and the one new threads start on.
    let choose = move |id: String| {
        default_model.set(id.clone());
        set_current_model.set(id);
        set_open.set(false);
    };
    let on_key = move |ev: leptos::ev::KeyboardEvent| {
        let n = flat.with_untracked(|f| f.len());
        // ⌘1–9 pick the Nth row.
        if (ev.meta_key() || ev.ctrl_key()) && !ev.shift_key() && !ev.alt_key() {
            if let Some(k) = ev
                .code()
                .strip_prefix("Digit")
                .and_then(|d| d.parse::<usize>().ok())
                .filter(|k| (1..=9).contains(k))
            {
                ev.prevent_default();
                if let Some(m) = flat.with_untracked(|f| f.get(k - 1).copied()) {
                    choose(m.id.to_string());
                }
                return;
            }
        }
        match ev.key().as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                set_highlight.update(|h| *h = (*h + 1).min(n.saturating_sub(1)));
            }
            "ArrowUp" => {
                ev.prevent_default();
                set_highlight.update(|h| *h = h.saturating_sub(1));
            }
            "Enter" => {
                ev.prevent_default();
                if let Some(m) = flat.with_untracked(|f| f.get(highlight.get_untracked()).copied())
                {
                    choose(m.id.to_string());
                }
            }
            "Escape" => {
                ev.prevent_default();
                set_open.set(false);
            }
            _ => {}
        }
    };
    let status_line = move || {
        let provider = flat
            .with(|f| f.get(highlight.get()).map(|m| m.provider))
            .unwrap_or_else(|| provider_for_model_id(&current_model.get()));
        auth_line(provider, &auth_status.get(), codex_auth.get().authenticated)
    };

    view! {
        {move || open.get().then(|| view! {
            <div class="fixed inset-0 z-40" aria-hidden="true" on:click=move |_| set_open.set(false)></div>
            <div
                role="dialog"
                aria-label="Choose model"
                class="absolute bottom-full left-0 z-50 mb-2 flex w-[22rem] max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-grape bg-longan-700 inset-ring inset-ring-white/15"
            >
                <label class="flex items-center gap-2 border-b border-white/10 px-3 py-2">
                    <Icon name="search" size=14 class="shrink-0 text-rambutan-400" />
                    <input
                        node_ref=input_ref
                        type="text"
                        placeholder="Search models…"
                        aria-label="Search models"
                        class="min-w-0 flex-1 bg-transparent text-body-sm text-rambutan-100 placeholder:text-rambutan-500 focus:outline-none"
                        prop:value=move || query.get()
                        on:input=move |ev| set_query.set(event_target_value(&ev))
                        on:keydown=on_key
                    />
                    <Kbd class="text-rambutan-400">"esc"</Kbd>
                </label>
                <div role="listbox" aria-label="Models" class="max-h-72 overflow-y-auto p-1">
                    {move || {
                        let gs = groups.get();
                        if gs.is_empty() {
                            return view! {
                                <p class="px-3 py-6 text-center text-label-sm text-rambutan-400">"No models match."</p>
                            }.into_any();
                        }
                        let mut index = 0usize;
                        gs.into_iter().map(|(provider, models)| {
                            let header = provider_label(provider);
                            let logo = provider_logo_src(provider);
                            let rows = models.into_iter().map(|m| {
                                let i = index;
                                index += 1;
                                let id_choose = m.id;
                                let id_selected = m.id;
                                let is_selected = Signal::derive(move || current_model.get() == id_selected);
                                let is_hot = Signal::derive(move || highlight.get() == i);
                                let label = m.label;
                                let id_text = m.id;
                                view! {
                                    <button
                                        type="button"
                                        role="option"
                                        aria-selected=move || is_selected.get().to_string()
                                        class=move || format!(
                                            "flex w-full items-center gap-3 rounded-grape px-2 py-2 text-left transition-colors {}",
                                            if is_hot.get() { "bg-white/[0.06]" } else { "" }
                                        )
                                        on:mouseenter=move |_| set_highlight.set(i)
                                        on:click=move |_| choose(id_choose.to_string())
                                    >
                                        <div class="min-w-0 flex-1">
                                            <div class="truncate text-label text-rambutan-100">{label}</div>
                                            <div class="mt-0.5 flex items-center gap-1.5 text-label-sm text-rambutan-400">
                                                <img src=logo alt="" aria-hidden="true" class=PROVIDER_LOGO_CLASS />
                                                <span class="truncate">{id_text}</span>
                                            </div>
                                        </div>
                                        {(i < 9).then(|| view! {
                                            <Kbd class="text-rambutan-400">{format!("⌘{}", i + 1)}</Kbd>
                                        })}
                                        {move || is_selected.get().then(|| view! {
                                            <Icon name="check" size=14 class="shrink-0 text-rambutan-100" />
                                        })}
                                    </button>
                                }
                            }).collect_view();
                            view! {
                                <div>
                                    <div class="px-2 pb-1 pt-2 text-label-sm uppercase tracking-widest text-rambutan-400">{header}</div>
                                    {rows}
                                </div>
                            }
                        }).collect_view().into_any()
                    }}
                </div>
                <div class="border-t border-white/10 px-3 py-2 text-label-sm text-rambutan-400" role="status">
                    {status_line}
                </div>
            </div>
        })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &'static str, label: &'static str, provider: AgentProvider) -> ModelInfo {
        ModelInfo {
            id,
            label,
            provider,
        }
    }

    #[test]
    fn filter_matches_label_id_and_provider_words() {
        let all = vec![
            m(
                "claude-sonnet-4-6",
                "Claude Sonnet 4.6",
                AgentProvider::Claude,
            ),
            m("gpt-5.5", "GPT-5.5", AgentProvider::Codex),
        ];
        assert_eq!(filter_models(&all, "sonnet").len(), 1);
        assert_eq!(filter_models(&all, "openai").len(), 1);
        assert_eq!(filter_models(&all, "gpt 5").len(), 1);
        assert_eq!(filter_models(&all, "").len(), 2);
        assert!(filter_models(&all, "gemini").is_empty());
    }

    #[test]
    fn groups_keep_first_seen_provider_order() {
        let all = [
            m("a", "A", AgentProvider::Claude),
            m("b", "B", AgentProvider::Codex),
            m("c", "C", AgentProvider::Claude),
        ];
        let g = group_models(&all.iter().collect::<Vec<_>>());
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].0, AgentProvider::Claude);
        assert_eq!(g[0].1.len(), 2);
    }

    #[test]
    fn auth_lines_name_the_fix() {
        let signed_out = AuthStatus::default();
        assert!(auth_line(AgentProvider::Codex, &signed_out, false).contains("codex login"));
        assert!(auth_line(AgentProvider::Claude, &signed_out, false).contains("claude login"));
        assert_eq!(
            auth_line(AgentProvider::Codex, &signed_out, true),
            "Codex CLI signed in"
        );
    }
}
