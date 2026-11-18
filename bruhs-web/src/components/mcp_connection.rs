//! Agent workflows → MCP workspace connection (`crate::workspace_bridge`):
//! the notes, buttons and copied config desktop's panel has.

use bruhs_models::workspace_bridge::{COPIED_NOTE, SESSION_ACTIVE_NOTE, SESSION_OFF_NOTE};
use leptos::prelude::*;

use crate::ui::{button_class, ButtonSize, ButtonVariant};
use crate::workspace_bridge::{Connection, WorkspaceBridge, UNAVAILABLE};

#[component]
pub fn McpConnectionPanel() -> impl IntoView {
    let bridge = expect_context::<WorkspaceBridge>();
    let copied = RwSignal::new(false);
    let connected = move || matches!(bridge.connection.get(), Connection::On { .. });
    let busy = move || {
        matches!(
            bridge.connection.get(),
            Connection::On { .. } | Connection::Starting
        )
    };
    let class = button_class(ButtonVariant::Outline, ButtonSize::Small, "");
    let copy_class = class.clone();
    view! {
        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="mcp-connection-title">
            <h2 id="mcp-connection-title" class="text-h4 text-rambutan-100">"MCP workspace connection"</h2>
            <p class="mt-1 text-body-sm text-rambutan-500">
                {move || if connected() { SESSION_ACTIVE_NOTE } else { SESSION_OFF_NOTE }}
            </p>
            {move || (!bridge.available() && !busy()).then(|| view! {
                <p role="status" class="mt-2 text-body-sm text-durian-300">{UNAVAILABLE}</p>
            })}
            {move || match bridge.connection.get() {
                Connection::Failed(message) => Some(view! {
                    <p role="alert" class="mt-2 text-body-sm text-lychee-300">{message}</p>
                }),
                _ => None,
            }}
            {move || bridge.notice.get().map(|message| view! {
                <p role="status" class="mt-2 text-body-sm text-blueberry-300">{message}</p>
            })}
            <div class="mt-3 flex flex-wrap gap-2">
                <button
                    type="button"
                    class=class
                    data-testid="mcp-connection-toggle"
                    aria-label=move || if busy() { "Disable MCP connection" } else { "Enable MCP connection for this session" }
                    disabled=move || !busy() && !bridge.available()
                    on:click=move |_| {
                        copied.set(false);
                        if busy() {
                            bridge.disable();
                        } else {
                            bridge.enable();
                        }
                    }
                >
                    {move || if busy() { "Disable connection" } else { "Enable for this session" }}
                </button>
                <Show when=connected>
                    <button
                        type="button"
                        class=copy_class.clone()
                        data-testid="mcp-copy-config"
                        aria-label="Copy MCP client configuration"
                        on:click=move |_| copied.set(bridge.copy_config())
                    >
                        "Copy MCP config"
                    </button>
                </Show>
            </div>
            <Show when=move || copied.get() && connected()>
                <p role="status" class="mt-2 text-label-sm text-pandan-300">
                    {COPIED_NOTE}
                </p>
            </Show>
        </section>
    }
}
