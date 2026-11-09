//! Inline notice when system-audio capture is unavailable (permission
//! not granted, non-macOS, etc.). Hidden when granted or while the
//! permission is "unsupported" (Linux/Windows — nothing the user
//! can do about it).

use leptos::prelude::*;

use bruhs_models::api::{CaptureStatus, SystemAudioPermission};

use crate::state::CaptureStatusSignal;

#[component]
pub fn CaptureStatusBanner() -> impl IntoView {
    let CaptureStatusSignal(status) = expect_context::<CaptureStatusSignal>();
    // The daemon's own words for the two states the user can fix.
    let permission = move || match &status.get() {
        CaptureStatus::Running(running) => match running.system_audio_permission {
            SystemAudioPermission::Denied => Some("denied"),
            SystemAudioPermission::NotDetermined => Some("not_determined"),
            _ => None,
        },
        CaptureStatus::NotRunning => None,
    };

    view! {
        <Show when=move || permission().is_some()
              fallback=|| view! { <></> }>
            // Lychee = the system's alert color; mangosteen is reserved for
            // decorative accents (design.bruhs.dev/llms.txt §color roles).
            <div
                role="status"
                class="bg-lychee-900/40 inset-ring inset-ring-lychee-700 rounded-seed px-3 py-2 text-label text-rambutan-300"
            >
                "System audio capture is " {move || permission()}
                ". Grant Screen Recording permission to capture the other side of meetings — open "
                <strong class="text-rambutan-200">"System Settings → Privacy & Security → Screen Recording"</strong>
                " and enable bruhs-capture."
            </div>
        </Show>
    }
}
