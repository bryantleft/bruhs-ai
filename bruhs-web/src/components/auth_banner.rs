//! Auth banner — shown above the chat when the CLI for the *currently
//! selected model's provider* isn't authenticated.
//!
//! Clicking "Log in with <Provider>" POSTs `/api/agent/login/<provider>`
//! (which runs `claude auth login` / `codex login` server-side) and then
//! polls `/api/agent/auth/<provider>` every 2s for up to 60s, updating the
//! matching global auth signal as soon as the user completes the flow.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::auth::{fetch_auth_status_for, trigger_login_for, AuthStatus};
use bruhs_models::agent::{provider_for_model_id, AgentProvider};

use crate::state::{CodexAuthSignal, SetCodexAuthSignal};
use crate::ui::{Button, ButtonSize, ButtonVariant, Icon};

const POLL_INTERVAL_MS: u32 = 2000;
const POLL_ATTEMPTS: u32 = 30; // 30 × 2s = 60s

/// Human-friendly provider name for banner copy.
fn provider_name(provider: &str) -> &'static str {
    match provider {
        "codex" => "Codex",
        "claude" => "Claude",
        _ => "Agent",
    }
}

fn offers_in_app_login(model: &str) -> bool {
    matches!(
        provider_for_model_id(model),
        AgentProvider::Claude | AgentProvider::Codex
    )
}

#[component]
pub fn AuthBanner() -> impl IntoView {
    // Claude auth is a raw signal; Codex is newtyped (see state.rs).
    let claude_auth = expect_context::<crate::state::ClaudeAuthSignal>().0;
    let set_claude_auth = expect_context::<crate::state::SetClaudeAuthSignal>().0;
    let CodexAuthSignal(codex_auth) = expect_context::<CodexAuthSignal>();
    let SetCodexAuthSignal(set_codex_auth) = expect_context::<SetCodexAuthSignal>();
    let current_model = expect_context::<crate::state::CurrentModel>().0;

    let (polling, set_polling) = signal(false);
    let (message, set_message) = signal::<Option<String>>(None);

    // The provider of the currently-selected model (reactive).
    let current_provider = move || {
        provider_for_model_id(&current_model.get())
            .as_str()
            .to_string()
    };

    // The auth status for the current provider (reactive).
    let current_status = move || match current_provider().as_str() {
        "codex" => codex_auth.get(),
        "claude" => claude_auth.get(),
        _ => AuthStatus::default(),
    };

    // Write a freshly-fetched status into the right provider's signal.
    let store_status = move |provider: &str, status: AuthStatus| match provider {
        "codex" => set_codex_auth.set(status),
        "claude" => set_claude_auth.set(status),
        _ => {}
    };

    let do_recheck = move || {
        if !offers_in_app_login(&current_model.get_untracked()) {
            return;
        }
        let provider = provider_for_model_id(&current_model.get_untracked())
            .as_str()
            .to_string();
        spawn_local(async move {
            let status = fetch_auth_status_for(&provider).await;
            store_status(&provider, status);
        });
    };

    let do_login = move || {
        if polling.get_untracked() || !offers_in_app_login(&current_model.get_untracked()) {
            return;
        }
        set_polling.set(true);
        let provider = provider_for_model_id(&current_model.get_untracked())
            .as_str()
            .to_string();
        set_message.set(Some(format!(
            "Waiting for {} login… complete the flow in the terminal where bruhs-server is running.",
            provider_name(&provider)
        )));
        spawn_local(async move {
            // Kick off login on the server (returns when the CLI exits) while
            // polling status in parallel.
            let login_future = trigger_login_for(&provider);
            let poll_future = async {
                for _ in 0..POLL_ATTEMPTS {
                    gloo_timers::future::TimeoutFuture::new(POLL_INTERVAL_MS).await;
                    let status = fetch_auth_status_for(&provider).await;
                    if status.authenticated {
                        store_status(&provider, status);
                        return true;
                    }
                }
                false
            };
            let (login_result, authed_early) = futures_join(login_future, poll_future).await;

            if !authed_early {
                let status = fetch_auth_status_for(&provider).await;
                if status.authenticated {
                    store_status(&provider, status);
                } else if let Err(e) = login_result {
                    set_message.set(Some(format!("Login failed: {e}")));
                } else {
                    set_message.set(Some(format!(
                        "Still not authenticated. Try running `{}` in a terminal.",
                        if provider == "codex" {
                            "codex login"
                        } else {
                            "claude auth login"
                        }
                    )));
                }
            }
            set_polling.set(false);
        });
    };

    view! {
        <Show
            when=move || offers_in_app_login(&current_model.get()) && !current_status().authenticated
            fallback=|| ()
        >
            <div class="mx-4 my-3 rounded-grape bg-durian-500/10 inset-ring inset-ring-durian-400/30 p-3 text-body-sm text-durian-100">
                <div class="flex items-start gap-3">
                    <span class="text-durian-300 pt-0.5">
                        <Icon name="alert-triangle" size=16 />
                    </span>
                    <div class="flex-1 min-w-0">
                        <div class="font-medium text-durian-100 mb-1">
                            {move || {
                                let status = current_status();
                                let name = provider_name(&current_provider());
                                bruhs_models::logic::cli_login::sign_in_banner_title(
                                    name,
                                    status.error.as_deref(),
                                )
                            }}
                        </div>
                        {move || message.get().map(|m| view! {
                            <div class="text-durian-200/80 mb-2">{m}</div>
                        })}
                        <div class="flex items-center gap-2 mt-1">
                            <Button
                                variant=ButtonVariant::Primary
                                size=ButtonSize::Small
                                disabled=Signal::derive(move || polling.get())
                                on_click=Callback::new(move |_| do_login())
                            >
                                {move || if polling.get() {
                                    "Waiting…".to_string()
                                } else {
                                    format!("Log in with {}", provider_name(&current_provider()))
                                }}
                            </Button>
                            <Button
                                variant=ButtonVariant::Outline
                                size=ButtonSize::Small
                                on_click=Callback::new(move |_| do_recheck())
                            >
                                "Check again"
                            </Button>
                        </div>
                    </div>
                </div>
            </div>
        </Show>
    }
}

/// Minimal `join` for two futures that returns both results. Local helper so
/// we don't pull in futures_util just for this.
async fn futures_join<A, B, FA, FB>(a: FA, b: FB) -> (A, B)
where
    FA: std::future::Future<Output = A>,
    FB: std::future::Future<Output = B>,
{
    use std::pin::pin;
    use std::task::Poll;

    let mut a = pin!(a);
    let mut b = pin!(b);
    let mut ra: Option<A> = None;
    let mut rb: Option<B> = None;
    std::future::poll_fn(move |cx| {
        if ra.is_none() {
            if let Poll::Ready(v) = a.as_mut().poll(cx) {
                ra = Some(v);
            }
        }
        if rb.is_none() {
            if let Poll::Ready(v) = b.as_mut().poll(cx) {
                rb = Some(v);
            }
        }
        match (ra.take(), rb.take()) {
            (Some(a), Some(b)) => Poll::Ready((a, b)),
            (a, b) => {
                ra = a;
                rb = b;
                Poll::Pending
            }
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_runtimes_never_inherit_a_claude_or_codex_login_banner() {
        assert!(offers_in_app_login("claude-sonnet-4-6"));
        assert!(offers_in_app_login("gpt-5.5"));
        for model in [
            "cursor/default",
            "antigravity/default",
            "grok/default",
            "opencode/local/model",
        ] {
            assert!(!offers_in_app_login(model), "{model}");
        }
    }
}
