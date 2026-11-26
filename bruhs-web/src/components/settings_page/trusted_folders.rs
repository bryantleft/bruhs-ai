use leptos::{prelude::*, task::spawn_local};

use crate::api::{list_trusted_folders, revoke_trusted_folder};
use crate::ui::{button_class, ButtonSize, ButtonVariant};

#[component]
pub(super) fn TrustedFolders() -> impl IntoView {
    let paths = RwSignal::new(Vec::<String>::new());
    let busy = RwSignal::new(false);
    let notice = RwSignal::new("");

    let update = move |revoke: Option<String>| {
        if busy.get_untracked() {
            return;
        }
        let revoking = revoke.is_some();
        busy.set(true);
        notice.set(if revoking {
            "Revoking trust…"
        } else {
            "Loading trusted folders…"
        });
        spawn_local(async move {
            let result = match revoke {
                Some(path) => revoke_trusted_folder(path).await,
                None => list_trusted_folders().await,
            };
            match result {
                Ok(remaining) => {
                    let message = match (revoking, remaining.is_empty()) {
                        (true, true) => "Trust revoked. No trusted folders.",
                        (true, false) => "Trust revoked.",
                        (false, true) => "No trusted folders.",
                        (false, false) => "",
                    };
                    paths.try_set(remaining);
                    notice.try_set(message);
                }
                Err(_) => {
                    notice.try_set(if revoking {
                        "Could not revoke trust. Try again."
                    } else {
                        "Could not load trusted folders. Refresh to try again."
                    });
                }
            }
            busy.try_set(false);
        });
    };
    Effect::new(move |_| update(None));

    view! {
        <div class="flex items-center justify-end gap-3 px-4 py-2.5">
            <p role="status" class="grow text-label-sm text-rambutan-400">{move || notice.get()}</p>
            <button
                type="button"
                class=button_class(ButtonVariant::Outline, ButtonSize::Small, "shrink-0")
                aria-label="Refresh trusted folders"
                disabled=move || busy.get()
                on:click=move |_| update(None)
            >"Refresh"</button>
        </div>
        <ul aria-label="Trusted folder paths" class="divide-y divide-longan-800/70 empty:hidden">
            <For
                each=move || paths.get()
                key=String::clone
                children=move |path| {
                    let label = format!("Revoke trust for {path}");
                    let entry = path.clone();
                    view! {
                        <li class="flex flex-wrap items-center justify-between gap-3 px-4 py-2.5">
                            <span class="min-w-0 flex-1 text-label text-rambutan-100 [overflow-wrap:anywhere]">{path}</span>
                            <button
                                type="button"
                                class=button_class(ButtonVariant::Outline, ButtonSize::Small, "shrink-0")
                                aria-label=label
                                disabled=move || busy.get()
                                on:click=move |_| update(Some(entry.clone()))
                            >"Revoke trust"</button>
                        </li>
                    }
                }
            />
        </ul>
    }
}
