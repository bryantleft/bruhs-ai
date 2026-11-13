use super::{context_usage, ContextMeter, PlanToggle};
use bruhs_models::agent::{provider_for_model_id, AgentProvider};
use bruhs_models::logic::context_chip::join_drafts;
use bruhs_models::logic::follow_up::provider_can_steer;
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use bruhs_models::PermissionMode;
use leptos::html::{Button, Div};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::format::{context_percentage, context_window_for};
use crate::stash::stash_preview;
use crate::state::{
    ChipRetargetGuard, ClaudeEffortSignal, CompactFullSignal, ComposerChips, CurrentModel,
    EffortMenuOpen, ModelSelectOpen, PermissionModeSignal, PromptStashSignal, SetModelSelectOpen,
    SetPermissionModeSignal, StashMenuOpen, ThreadNotice, UsageInfo,
};
use crate::ui::dialog::focusable_descendants;
use crate::ui::Icon;

const ACTION: &str = "bruhs-control inline-flex min-h-8 items-center justify-center gap-2 rounded-seed px-2 text-label-sm text-rambutan-300 hover:bg-longan-800/60 hover:text-rambutan-100 disabled:opacity-50 disabled:cursor-not-allowed";
const ROW: &str = "bruhs-control flex min-h-8 w-full items-center justify-between gap-2 rounded-seed px-2 text-left text-label-sm text-rambutan-300 hover:bg-longan-800/60 hover:text-rambutan-100 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-transparent disabled:hover:text-rambutan-300";

#[component]
pub(super) fn ComposerOptions(
    value: ReadSignal<String>,
    set_value: WriteSignal<String>,
    on_slash: Callback<&'static str>,
    last_usage: Signal<Option<(UsageInfo, String)>>,
    return_compact_available: Signal<bool>,
) -> impl IntoView {
    let EffortMenuOpen(open) = expect_context::<EffortMenuOpen>();
    let StashMenuOpen(stash_open) = expect_context::<StashMenuOpen>();
    let PromptStashSignal(stash) = expect_context::<PromptStashSignal>();
    let PermissionModeSignal(access) = expect_context::<PermissionModeSignal>();
    let SetPermissionModeSignal(set_access) = expect_context::<SetPermissionModeSignal>();
    let CurrentModel(model) = expect_context::<CurrentModel>();
    let ModelSelectOpen(model_open) = expect_context::<ModelSelectOpen>();
    let SetModelSelectOpen(set_model_open) = expect_context::<SetModelSelectOpen>();
    let ClaudeEffortSignal(effort) = expect_context::<ClaudeEffortSignal>();
    let CompactFullSignal(full_draft) = expect_context::<CompactFullSignal>();
    let ComposerChips(chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(chip_guard) = expect_context::<ChipRetargetGuard>();
    let notice = expect_context::<ThreadNotice>();
    let anchor = NodeRef::<Button>::new();
    let preferred = RwSignal::new("[data-effort-default]");
    let plan_toggle = StoredValue::new(PlanToggle::new(access.get_untracked()));

    Effect::new(move |_| {
        if open.get() {
            stash_open.set(false);
            set_model_open.set(false);
        } else {
            preferred.set("[data-effort-default]");
        }
    });
    Effect::new(move |_| {
        if stash_open.get() {
            open.set(false);
            set_model_open.set(false);
        }
    });
    Effect::new(move |_| {
        if model_open.get() {
            open.set(false);
            stash_open.set(false);
        }
    });

    let restore = move |id: String| {
        let entry = stash.try_update(|items| items.take(&id)).flatten();
        if let Some(entry) = entry {
            let (next, kept) = join_drafts(
                &value.get_untracked(),
                &chips.get_untracked(),
                &entry.text,
                &entry.chips,
                true,
            );
            chip_guard.set_value(true);
            chips.set(kept);
            set_value.set(next);
            stash_open.set(false);
            if let Some(input) = document()
                .get_element_by_id("chat-input")
                .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = input.focus();
            }
        }
    };

    view! {
        <div class="relative inline-flex shrink-0">
            <button node_ref=anchor type="button" data-composer-options-trigger=""
                class=move || if access.get() == PermissionMode::BypassPermissions {
                    "composer-action bruhs-control inline-flex size-8 shrink-0 items-center justify-center rounded-grape text-lychee-300 hover:bg-lychee-500/10"
                } else {
                    "composer-action bruhs-control inline-flex size-8 shrink-0 items-center justify-center rounded-grape text-rambutan-400 hover:bg-longan-800/50 hover:text-rambutan-100"
                }
                aria-label="Composer options" aria-haspopup="dialog" aria-expanded=move || open.get().to_string()
                title=move || format!("Composer options. Access: {}. Mode: {}. Effort: {}. Saved prompts: {}.",
                    if access.get() == PermissionMode::Plan { "Read only" } else { access.get().label() },
                    if access.get() == PermissionMode::Plan { "Plan" } else { "Build" },
                    effort.get().unwrap_or_else(|| "Default".into()), stash.with(|items| items.len()))
                on:click=move |_| {
                    preferred.set("[data-option-access]");
                    open.update(|state| *state = !*state);
                }>
                <Icon name="settings" size=14 />
            </button>
            <ComposerPopup open=open anchor=anchor label="Composer options" preferred=Signal::derive(move || preferred.get())>
                <div class="flex flex-col gap-4">
                    <section aria-label="Access">
                        <h3 class="mb-1 px-2 text-label-sm text-rambutan-500">"Access"</h3>
                        {move || (access.get() == PermissionMode::Plan).then(|| view! {
                            <p class="mb-1 px-2 text-label-sm text-rambutan-400">"Plan is read only"</p>
                        })}
                        <div role="radiogroup" aria-label="Access mode" class="flex flex-col gap-1">
                            {[PermissionMode::Default, PermissionMode::AcceptEdits, PermissionMode::BypassPermissions]
                                .into_iter().map(|mode| view! {
                                    <button type="button" role="radio" data-option-access=if mode == PermissionMode::Default { Some("") } else { None }
                                        aria-checked=move || (access.get() == mode).to_string()
                                        title=move || mode.description(Some(provider_for_model_id(&model.get())))
                                        class=move || format!("{ROW} {}", if access.get() == mode { "bg-longan-800 text-rambutan-100" } else { "" })
                                        on:click=move |_| { set_access.set(mode); plan_toggle.set_value(PlanToggle::new(mode)); }>
                                        <span>{mode.label()}</span>
                                        {move || (access.get() == mode).then(|| view! { <Icon name="check" size=14 /> })}
                                    </button>
                                }).collect_view()}
                        </div>
                    </section>
                    <section aria-label="Mode">
                        <h3 class="mb-1 px-2 text-label-sm text-rambutan-500">"Mode"</h3>
                        <div role="radiogroup" aria-label="Mode" class="flex gap-1">
                            {[false, true].into_iter().map(|plan| view! {
                                <button type="button" role="radio" aria-checked=move || ((access.get() == PermissionMode::Plan) == plan).to_string()
                                    class=move || format!("{ACTION} flex-1 {}", if (access.get() == PermissionMode::Plan) == plan { "bg-longan-800 text-rambutan-100" } else { "" })
                                    on:click=move |_| {
                                        let current = access.get_untracked();
                                        if (current == PermissionMode::Plan) != plan {
                                            let next = plan_toggle.try_update_value(|toggle| toggle.toggle(current)).unwrap_or(current);
                                            set_access.set(next);
                                        }
                                    }>{if plan { "Plan" } else { "Build" }}</button>
                            }).collect_view()}
                        </div>
                    </section>
                    {move || (provider_for_model_id(&model.get()) == AgentProvider::Claude).then(|| view! {
                        <section aria-label="Effort">
                            <h3 class="mb-1 px-2 text-label-sm text-rambutan-500">"Effort"</h3>
                            <div role="radiogroup" aria-label="Effort" class="grid grid-cols-3 gap-1">
                                {std::iter::once(None).chain(CLAUDE_EFFORTS.into_iter().map(Some)).map(|level| {
                                    let label = level.unwrap_or("Default");
                                    view! {
                                        <button type="button" role="radio" data-effort-default=level.is_none().then_some("")
                                            aria-checked=move || (effort.get().as_deref() == level).to_string()
                                            class=move || format!("{ACTION} {}", if effort.get().as_deref() == level { "bg-longan-800 text-rambutan-100" } else { "" })
                                            on:click=move |_| effort.set(level.map(str::to_string))>{label}</button>
                                    }
                                }).collect_view()}
                            </div>
                        </section>
                    })}
                    <section aria-label="Prompts">
                        <h3 class="mb-1 px-2 text-label-sm text-rambutan-500">"Prompts"</h3>
                        <button type="button" class=ROW disabled=move || value.get().trim().is_empty()
                            on:click=move |_| {
                                let text = value.get_untracked();
                                if text.trim().is_empty() { return; }
                                stash.update(|items| { items.push_with_chips(&text, &chips.get_untracked(), js_sys::Date::now()); });
                                chip_guard.set_value(true); chips.set(Vec::new()); set_value.set(String::new());
                                notice.flash("Prompt stashed");
                            }><span>"Stash draft"</span><span class="text-rambutan-500">"⌘S"</span></button>
                        <button type="button" class=ROW on:click=move |_| { open.set(false); stash_open.set(true); }>
                            <span>"Saved prompts"</span><span data-stash-count=move || stash.with(|items| items.len()) class="tabular-nums text-rambutan-500">{move || stash.with(|items| items.len())}</span>
                        </button>
                    </section>
                    {move || (last_usage.get().is_some() || return_compact_available.get()).then(|| view! {
                        <section aria-label="Context">
                            <h3 class="mb-1 px-2 text-label-sm text-rambutan-500">"Context"</h3>
                            {move || last_usage.get().map(|(usage, used_model)| {
                                let percentage = context_percentage(context_usage(&usage), context_window_for(&used_model));
                                view! { <div class="mb-1 flex items-center gap-2 px-2 text-label-sm text-rambutan-400"><ContextMeter usage=usage model=used_model /><span>{format!("{percentage:.0}% used")}</span></div> }
                            })}
                            <button type="button" class=ROW disabled=move || !provider_can_steer(provider_for_model_id(&model.get()))
                                on:click=move |_| { open.set(false); on_slash.run("/compact"); }><span>"Compact thread"</span></button>
                            {move || return_compact_available.get().then(|| view! {
                                <button type="button" class=ROW aria-pressed=move || full_draft.get().to_string()
                                    on:click=move |_| full_draft.update(|full| *full = !*full)>
                                    <span>"Send full draft"</span>
                                    {move || full_draft.get().then(|| view! { <Icon name="check" size=14 /> })}
                                </button>
                            })}
                        </section>
                    })}
                </div>
            </ComposerPopup>
            <ComposerPopup open=stash_open anchor=anchor label="Saved prompts" preferred=Signal::derive(|| "[data-stash-entry]")>
                {move || {
                    let entries = stash.with(|items| items.entries.clone());
                    if entries.is_empty() {
                        return view! { <p class="px-2 py-3 text-label-sm text-rambutan-500">"No saved prompts"</p> }.into_any();
                    }
                    let now = js_sys::Date::now();
                    view! {
                        <div role="list" class="flex flex-col gap-1">
                            {entries.into_iter().map(|entry| {
                                let restore_id = entry.id.clone(); let delete_id = entry.id;
                                let preview = stash_preview(&entry.text, 80);
                                let age = crate::format::relative_time(entry.at_ms, now);
                                view! {
                                    <div role="listitem" class="flex items-center gap-1">
                                        <button type="button" data-stash-entry="" class="bruhs-control min-h-8 min-w-0 flex-1 rounded-seed px-2 py-1 text-left text-label-sm text-rambutan-300 hover:bg-longan-800/60" title=entry.text on:click=move |_| restore(restore_id.clone())>
                                            <span class="block truncate">{preview}</span><span class="block text-rambutan-500">{age}</span>
                                        </button>
                                        <button type="button" aria-label="Delete saved prompt" class="bruhs-control inline-flex size-8 shrink-0 items-center justify-center rounded-seed text-rambutan-500 hover:bg-lychee-500/10 hover:text-lychee-300" on:click=move |_| { stash.update(|items| { items.remove(&delete_id); }); }><Icon name="x" size=14 /></button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }}
            </ComposerPopup>
        </div>
    }
}

#[component]
fn ComposerPopup(
    open: RwSignal<bool>,
    anchor: NodeRef<Button>,
    label: &'static str,
    preferred: Signal<&'static str>,
    children: ChildrenFn,
) -> impl IntoView {
    view! {
        <Show when=move || open.get()>
            <ComposerPopupSurface open=open anchor=anchor label=label preferred=preferred content=children.clone() />
        </Show>
    }
}

#[component]
fn ComposerPopupSurface(
    open: RwSignal<bool>,
    anchor: NodeRef<Button>,
    label: &'static str,
    preferred: Signal<&'static str>,
    content: ChildrenFn,
) -> impl IntoView {
    let menu = NodeRef::<Div>::new();
    let position = RwSignal::new("visibility:hidden".to_string());
    let place = move || {
        let (Some(Some(node)), Some(Some(button)), Some(window)) = (
            menu.try_get_untracked(),
            anchor.try_get_untracked(),
            web_sys::window(),
        ) else {
            return;
        };
        let (Some(width), Some(height)) = (
            window.inner_width().ok().and_then(|value| value.as_f64()),
            window.inner_height().ok().and_then(|value| value.as_f64()),
        ) else {
            return;
        };
        let canvas_top = document()
            .query_selector("nav[aria-label=\"Workspace mode\"]")
            .ok()
            .flatten()
            .map_or(0.0, |nav| nav.get_bounding_client_rect().bottom());
        let inset = 8.0;
        let top_bound = canvas_top + inset;
        let available = (height - top_bound - inset).max(0.0);
        let bounds = node.get_bounding_client_rect();
        let trigger = button.get_bounding_client_rect();
        let visible_height = bounds.height().min(available);
        let left = trigger
            .left()
            .clamp(inset, (width - bounds.width() - inset).max(inset));
        let top = (trigger.top() - inset - visible_height)
            .clamp(top_bound, (height - visible_height - inset).max(top_bound));
        position.set(format!(
            "left:{left}px;top:{top}px;max-height:{available}px;"
        ));
    };
    Effect::new(move |_| {
        if menu.get().is_some() {
            request_animation_frame(move || {
                place();
                request_animation_frame(move || {
                    let Some(Some(node)) = menu.try_get_untracked() else {
                        return;
                    };
                    let target = node
                        .query_selector(preferred.get_untracked())
                        .ok()
                        .flatten()
                        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
                    let html: web_sys::HtmlElement = (*node).clone();
                    if let Some(target) =
                        target.or_else(|| focusable_descendants(&html).into_iter().next())
                    {
                        let _ = target.focus();
                    }
                });
            });
        }
    });
    let resize = window_event_listener(leptos::ev::resize, move |_| place());
    on_cleanup(move || {
        resize.remove();
        let active = document().active_element();
        let closing = menu.get_untracked();
        let should_return = active.as_ref().is_none_or(|active| {
            matches!(active.tag_name().as_str(), "BODY" | "MAIN")
                || closing
                    .as_ref()
                    .is_some_and(|node| node.contains(Some(active)))
        });
        if should_return {
            if let Some(Some(button)) = anchor.try_get_untracked() {
                let _ = button.focus();
            }
        }
    });
    let on_key = move |event: web_sys::KeyboardEvent| {
        if event.key() == "Escape" {
            event.prevent_default();
            event.stop_propagation();
            open.set(false);
            return;
        }
        if event.key() != "Tab" {
            return;
        }
        let Some(node) = menu.get_untracked() else {
            return;
        };
        let html: web_sys::HtmlElement = (*node).clone();
        let stops = focusable_descendants(&html);
        let Some(active) = document().active_element() else {
            return;
        };
        if !node.contains(Some(&active)) {
            event.prevent_default();
            if let Some(first) = stops.first() {
                let _ = first.focus();
            }
            return;
        }
        if event.shift_key() {
            if stops
                .first()
                .is_some_and(|first| first.is_same_node(Some(&active)))
            {
                event.prevent_default();
                if let Some(last) = stops.last() {
                    let _ = last.focus();
                }
            }
        } else if stops
            .last()
            .is_some_and(|last| last.is_same_node(Some(&active)))
        {
            event.prevent_default();
            if let Some(first) = stops.first() {
                let _ = first.focus();
            }
        }
    };
    // A native overlay scrollbar can move focus to the document. Keep Escape
    // and the tab cycle available after dragging a handle as well.
    let outside_key = window_event_listener(leptos::ev::keydown, move |event| {
        let outside = menu.get_untracked().is_some_and(|node| {
            document()
                .active_element()
                .is_none_or(|active| !node.contains(Some(&active)))
        });
        if outside && matches!(event.key().as_str(), "Escape" | "Tab") {
            on_key(event);
        }
    });
    on_cleanup(move || outside_key.remove());
    view! {
        <div class="fixed inset-0 z-40" aria-hidden="true" on:click=move |_| open.set(false) />
        <div node_ref=menu role="dialog" aria-label=label style=move || position.get() on:keydown=on_key
            class="fixed z-50 flex w-80 max-w-[calc(100vw-1rem)] flex-col overflow-hidden rounded-grape bg-longan-700 inset-ring inset-ring-white/15">
            <div class="flex shrink-0 items-center justify-between gap-2 border-b border-white/10 px-3 py-2">
                <h2 class="text-label text-rambutan-100">{label}</h2>
                <button type="button" aria-label=format!("Close {label}") class="bruhs-control inline-flex size-8 items-center justify-center rounded-seed text-rambutan-400 hover:bg-longan-800/60 hover:text-rambutan-100" on:click=move |_| open.set(false)><Icon name="x" size=14 /></button>
            </div>
            <div data-composer-options-scroll="" class="min-h-0 overflow-y-auto p-2">{content()}</div>
        </div>
    }
}
