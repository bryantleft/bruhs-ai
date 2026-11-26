//! Workspace Trust banner — shown above the composer when the active project
//! folder is untrusted and contains executable repository configs (hooks,
//! `.claude/settings.json`, `.mcp.json`, `.gitmodules`, etc.).

use bruhs_models::workspace_trust::{banner_summary, WorkspaceTrustStatus};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{set_workspace_trust, workspace_trust_status};
use crate::ui::{Banner, BannerTone, Button, ButtonSize, ButtonVariant};

#[component]
pub fn WorkspaceTrustBanner(#[prop(into)] project_path: Signal<Option<String>>) -> impl IntoView {
    let (status, set_status) = signal::<Option<WorkspaceTrustStatus>>(None);
    let (saving, set_saving) = signal(false);
    let (error, set_error) = signal::<Option<String>>(None);
    let ticket = StoredValue::new(0u32);

    Effect::new(move |_| {
        let path = project_path.get();
        let current = ticket.with_value(|n| n.wrapping_add(1));
        ticket.set_value(current);
        set_error.set(None);
        let Some(path) = path.filter(|p| !p.trim().is_empty()) else {
            set_status.set(None);
            return;
        };
        spawn_local(async move {
            let result = workspace_trust_status(&path).await;
            if ticket.with_value(|n| *n) != current {
                return;
            }
            match result {
                Ok(found) => set_status.set(Some(found)),
                Err(_) => set_status.set(None),
            }
        });
    });

    let trust_now = move || {
        if saving.get_untracked() {
            return;
        }
        let Some(path) = project_path
            .get_untracked()
            .filter(|p| !p.trim().is_empty())
        else {
            return;
        };
        set_saving.set(true);
        set_error.set(None);
        spawn_local(async move {
            match set_workspace_trust(&path, true).await {
                Ok(updated) => {
                    set_status.set(Some(updated));
                    set_error.set(None);
                }
                Err(err) => set_error.set(Some(format!("Could not trust folder: {err}"))),
            }
            set_saving.set(false);
        });
    };

    view! {
        <Show
            when=move || status.get().is_some_and(|s| s.should_warn())
            fallback=|| ()
        >
            <div class="mx-auto mb-2 w-full max-w-3xl" data-workspace-trust-banner>
                <Banner
                    tone=BannerTone::Warning
                    actions=Box::new(move || view! {
                        <Button
                            variant=ButtonVariant::Primary
                            size=ButtonSize::Small
                            disabled=Signal::derive(move || saving.get())
                            on_click=Callback::new(move |_| trust_now())
                        >
                            {move || if saving.get() { "Trusting…".to_string() } else { "Trust folder".to_string() }}
                        </Button>
                    }.into_any())
                >
                    <p class="font-medium text-rambutan-100">
                        {move || {
                            status
                                .get()
                                .map(|s| banner_summary(&s.risks))
                                .unwrap_or_default()
                        }}
                    </p>
                    {move || error.get().map(|msg| view! {
                        <p class="mt-0.5 text-body-sm text-lychee-300">{msg}</p>
                    })}
                </Banner>
            </div>
        </Show>
    }
}
