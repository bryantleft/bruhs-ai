//! A bruh's Computer tab: its browser's status line, the owner's controls
//! and the live view.
//!
//! The live view is a WebSocket to the service while the browser runs.
//! Frames arrive one at a time as JPEG blobs, each shown through an object
//! URL and acknowledged once decoded, so a hidden tab never stalls the
//! stream. While the owner holds control, the view's pointer, wheel, keys
//! and paste go to the page.

mod input;
mod live;

use std::time::Duration;

use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::{
    computer_controls, computer_on, computer_status_line, ADDRESS_LABEL, BACK, CLOSE_BROWSER,
    HAND_BACK, LIVE_VIEW_HINT, LIVE_VIEW_LABEL, OPEN_BROWSER, RELOAD, TAKE_CONTROL,
    TAKE_CONTROL_FIRST, USE_THE_WINDOW,
};
use bruhs_models::local_tools::{
    wheel_input, ComputerInput, ComputerRequest, ComputerState, HeldKeys, KeyAction, MouseButton,
    MouseKind,
};
use leptos::ev;
use leptos::html;
use leptos::prelude::*;

use self::input::{
    click_count, modifier_bits, pointer_input, pressed_button, wheel_px, with_clicks, Press,
};
use self::live::{LiveEvents, LiveView};
use super::data::use_crew_snapshot;
use crate::components::shell::host_mod_is_command;
use crate::ui::{Button, ButtonVariant};

/// How often the tab asks for the state while no live view runs, so a
/// browser the bruh opens shows up.
const CLOSED_POLL_MS: u64 = 5_000;

const ADDRESS: &str = "block min-h-control w-full min-w-0 flex-1 basis-48 bruhs-control rounded-grape bg-white/5 px-2.5 py-[5px] text-body text-rambutan-100 inset-ring inset-ring-white/10 read-only:text-rambutan-400 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400";
const ERROR: &str = "rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200";
/// A request button while its request runs: still focusable, but dimmed.
const BUSY: &str = "aria-busy:cursor-progress aria-busy:opacity-60";

/// The live view's classes. Without a frame the `<img>` has no `src`, and
/// Chrome draws its silver broken-image border over the box's ring, so it
/// stays see-through until a frame comes, or while focused, so its outline
/// shows.
fn live_view_class(has_frame: bool, input: bool) -> String {
    let touch = if input {
        " touch-none cursor-default"
    } else {
        ""
    };
    let empty = if has_frame {
        ""
    } else {
        " opacity-0 focus:opacity-100"
    };
    format!("block h-full w-full select-none object-contain focus:outline-2 focus:-outline-offset-2 focus:outline-persimmon-400{touch}{empty}")
}

#[component]
pub fn BruhComputer(agent_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    let id = StoredValue::new(agent_id);
    let mac = host_mod_is_command();

    // The bruh as the snapshot has it: its name and whether its Computer
    // is on.
    let bruh = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            let snapshot = snapshot.as_ref()?;
            id.with_value(|id| {
                let bruh = snapshot.bruhs.iter().find(|bruh| bruh.agent_id == *id)?;
                let provider = snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.id == *id)
                    .map_or(AgentProvider::Claude, |agent| {
                        AgentProvider::for_model(&agent.model)
                    });
                Some((
                    bruh.name.clone(),
                    computer_on(bruh.computer, &bruh.access, provider),
                ))
            })
        })
    });
    let on = Memo::new(move |_| bruh.with(|bruh| bruh.as_ref().is_some_and(|bruh| bruh.1)));
    let ready = Memo::new(move |_| data.computer_ready());

    let state = RwSignal::new(None::<ComputerState>);
    let error = RwSignal::new(None::<String>);
    // The request on its way, if any.
    let pending = RwSignal::new(None::<ComputerRequest>);
    let focused = RwSignal::new(false);
    let alive = RwSignal::new(true);
    let img = NodeRef::<html::Img>::new();
    let address_field = NodeRef::<html::Input>::new();
    let live = LiveView::new(img);

    let controls = Memo::new(move |_| {
        state.with(|state| computer_controls(state.as_ref(), on.get(), ready.get(), false))
    });
    let session = Memo::new(move |_| {
        state.with(|state| state.as_ref().and_then(|state| state.session_id.clone()))
    });
    let url = Memo::new(move |_| {
        state.with(|state| {
            state
                .as_ref()
                .and_then(|state| state.url.clone())
                .unwrap_or_default()
        })
    });
    let viewport = Memo::new(move |_| {
        state.with(|state| {
            state
                .as_ref()
                .map(|state| state.viewport)
                .unwrap_or_default()
        })
    });
    let status = Memo::new(move |_| {
        if !ready.get() {
            return data.computer_reason().unwrap_or_default().to_string();
        }
        let name = bruh.with(|bruh| bruh.as_ref().map(|bruh| bruh.0.clone()).unwrap_or_default());
        state.with(|state| computer_status_line(state.as_ref(), on.get(), &name))
    });
    let hint = Memo::new(move |_| {
        let controls = controls.get();
        if controls.input {
            focused.get().then_some(LIVE_VIEW_HINT)
        } else if state.with(|state| state.as_ref().is_some_and(|state| state.headed)) {
            Some(USE_THE_WINDOW)
        } else if controls.take {
            Some(TAKE_CONTROL_FIRST)
        } else {
            None
        }
    });

    // The state from the service. A newer answer, or a state message from
    // the live view, makes an older request's answer stale.
    let revision = StoredValue::new(0u64);
    let fetch_failed = StoredValue::new(false);
    let load = move || {
        revision.update_value(|value| *value = value.wrapping_add(1));
        let requested = revision.get_value();
        let agent = id.get_value();
        async move {
            let result = crate::api::computer_state(&agent).await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_value() != Some(requested)
            {
                return;
            }
            match result {
                Ok(next) => {
                    if fetch_failed.get_value() {
                        fetch_failed.set_value(false);
                        error.set(None);
                    }
                    state.set(Some(next));
                }
                Err(err) => {
                    fetch_failed.set_value(true);
                    error.set(Some(err.to_string()));
                }
            }
        }
    };
    Effect::new(move |_| {
        if ready.get() && on.get() {
            wasm_bindgen_futures::spawn_local(load());
        }
    });

    let request = move |request: ComputerRequest| {
        if pending.get_untracked().is_some() {
            return;
        }
        pending.set(Some(request));
        error.set(None);
        fetch_failed.set_value(false);
        live.forget_error();
        revision.update_value(|value| *value = value.wrapping_add(1));
        let agent = id.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::computer_request(&agent, &request).await;
            if alive.try_get_untracked() != Some(true) {
                return;
            }
            pending.set(None);
            match result {
                Ok(next) => state.set(Some(next)),
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    };

    // The live view, while the browser runs on a service that streams it.
    let events = LiveEvents {
        state: Callback::new(move |next| {
            revision.update_value(|value| *value = value.wrapping_add(1));
            state.set(Some(next));
        }),
        error: Callback::new(move |message| error.set(message)),
        // Ask for the state again, then connect to what it says.
        closed: Callback::new(move |token| {
            wasm_bindgen_futures::spawn_local(async move {
                load().await;
                live.retry(token);
            });
        }),
    };
    Effect::new(move |_| {
        live.reconnect.track();
        let session = session.get();
        let streams = ready.get() && on.get();
        match session.filter(|_| streams) {
            Some(session) => live.connect(&session, events),
            None => {
                live.disconnect();
                live.clear_frame();
            }
        }
    });

    // While no live view runs, look for a browser the bruh opens. A request
    // on its way answers for itself.
    let poll = set_interval_with_handle(
        move || {
            if ready.try_get_untracked() == Some(true)
                && on.try_get_untracked() == Some(true)
                && pending.try_with_untracked(Option::is_none) == Some(true)
                && !live.running()
            {
                wasm_bindgen_futures::spawn_local(load());
            }
        },
        Duration::from_millis(CLOSED_POLL_MS),
    )
    .ok();
    on_cleanup(move || {
        alive.set(false);
        if let Some(poll) = poll {
            poll.clear();
        }
        live.disconnect();
        live.clear_frame();
    });

    // Owner input, only while the owner holds control.
    let send_now = move |input: ComputerInput| {
        if controls
            .try_get_untracked()
            .is_some_and(|controls| controls.input)
        {
            live.send(input);
        }
    };
    // Moves and wheel turns wait for the next frame, newest move and summed
    // turns winning; anything else sends them first, so order holds.
    let pending_move = StoredValue::new(None::<ComputerInput>);
    let pending_wheel = StoredValue::new(None::<ComputerInput>);
    let frame_asked = StoredValue::new(false);
    let flush = move || {
        let _ = frame_asked.try_set_value(false);
        for slot in [pending_move, pending_wheel] {
            if let Some(Some(input)) = slot.try_update_value(Option::take) {
                send_now(input);
            }
        }
    };
    let send_next_frame = move || {
        if frame_asked.try_get_value() == Some(false) {
            frame_asked.set_value(true);
            request_animation_frame(flush);
        }
    };
    let send = move |input: ComputerInput| {
        flush();
        send_now(input);
    };

    // Where a pointer event lands on the live view, and the view's size.
    let point = move |event: &web_sys::MouseEvent| {
        let img = img.get_untracked()?;
        let rect = img.get_bounding_client_rect();
        Some((
            f64::from(event.client_x()) - rect.left(),
            f64::from(event.client_y()) - rect.top(),
            (rect.width(), rect.height()),
        ))
    };
    let modifiers = |event: &web_sys::MouseEvent| {
        modifier_bits(
            event.alt_key(),
            event.ctrl_key(),
            event.meta_key(),
            event.shift_key(),
        )
    };
    let last_press = StoredValue::new(None::<Press>);
    // The press the next release ends: its DOM button, click count and the
    // page point it last moved to.
    let held = StoredValue::new(None::<(i16, u8, f64, f64)>);
    let held_keys = StoredValue::new(HeldKeys::default());
    // A press whose release the view won't see (focus or capture lost) is
    // released where it last was, and so is every key still held.
    let release_pointer = move || {
        let Some(Some((button, clicks, x, y))) = held.try_update_value(Option::take) else {
            return;
        };
        let button = pressed_button(button);
        if button != MouseButton::None {
            send(ComputerInput::Mouse {
                kind: MouseKind::Up,
                x,
                y,
                button,
                clicks,
                modifiers: 0,
            });
        }
    };
    let release_held = move || {
        release_pointer();
        for input in held_keys
            .try_update_value(HeldKeys::release_all)
            .unwrap_or_default()
        {
            send(input);
        }
    };

    let on_pointer_down = move |event: web_sys::PointerEvent| {
        if !controls.get_untracked().input || !event.is_primary() {
            return;
        }
        event.prevent_default();
        let Some(img) = img.get_untracked() else {
            return;
        };
        // Measured before focusing, and focused without scrolling, so the
        // view never moves under the press.
        let measured = point(&event);
        let options = web_sys::FocusOptions::new();
        options.set_prevent_scroll(true);
        let _ = img.focus_with_options(&options);
        let Some((x, y, shown)) = measured else {
            return;
        };
        let Some(input) = pointer_input(
            MouseKind::Down,
            x,
            y,
            shown,
            event.buttons(),
            event.button(),
            modifiers(&event),
            viewport.get_untracked(),
        ) else {
            return;
        };
        let _ = img.set_pointer_capture(event.pointer_id());
        let ComputerInput::Mouse {
            x: page_x,
            y: page_y,
            button,
            ..
        } = input
        else {
            return;
        };
        let at_ms = event.time_stamp();
        let clicks = click_count(last_press.get_value(), at_ms, page_x, page_y, button);
        last_press.set_value(Some(Press {
            at_ms,
            x: page_x,
            y: page_y,
            button,
            clicks,
        }));
        held.set_value(Some((event.button(), clicks, page_x, page_y)));
        send(with_clicks(input, clicks));
    };
    let release = move |event: &web_sys::PointerEvent, button: i16, clicks: u8| {
        let Some((x, y, shown)) = point(event) else {
            return;
        };
        if let Some(input) = pointer_input(
            MouseKind::Up,
            x,
            y,
            shown,
            event.buttons(),
            button,
            modifiers(event),
            viewport.get_untracked(),
        ) {
            send(with_clicks(input, clicks));
        }
    };
    let on_pointer_up = move |event: web_sys::PointerEvent| {
        if !event.is_primary() {
            return;
        }
        let Some((_, clicks, _, _)) = held.get_value() else {
            return;
        };
        held.set_value(None);
        release(&event, event.button(), clicks);
    };
    // A cancelled pointer (a touch the browser took over) ends its press.
    let on_pointer_cancel = move |event: web_sys::PointerEvent| {
        if let Some((button, clicks, _, _)) = held.get_value() {
            held.set_value(None);
            release(&event, button, clicks);
        }
    };
    let on_pointer_move = move |event: web_sys::PointerEvent| {
        if !controls.get_untracked().input || !event.is_primary() {
            return;
        }
        let Some((x, y, shown)) = point(&event) else {
            return;
        };
        let input = pointer_input(
            MouseKind::Move,
            x,
            y,
            shown,
            event.buttons(),
            event.button(),
            modifiers(&event),
            viewport.get_untracked(),
        );
        if let Some(input) = input {
            if let ComputerInput::Mouse { x, y, .. } = input {
                held.update_value(|held| {
                    if let Some(held) = held {
                        (held.2, held.3) = (x, y);
                    }
                });
            }
            pending_move.set_value(Some(input));
            send_next_frame();
        }
    };
    let on_wheel = move |event: web_sys::WheelEvent| {
        if !controls.get_untracked().input {
            return;
        }
        event.prevent_default();
        let Some((x, y, shown)) = point(&event) else {
            return;
        };
        let viewport = viewport.get_untracked();
        let mode = event.delta_mode();
        let delta = (
            wheel_px(event.delta_x(), mode, viewport),
            wheel_px(event.delta_y(), mode, viewport),
        );
        let next = wheel_input(pending_wheel.get_value(), x, y, shown, delta, viewport);
        if next.is_some() {
            pending_wheel.set_value(next);
            send_next_frame();
        }
    };
    let on_key = move |event: web_sys::KeyboardEvent, down: bool| {
        if !controls.get_untracked().input || event.is_composing() {
            return;
        }
        let modifiers = modifier_bits(
            event.alt_key(),
            event.ctrl_key(),
            event.meta_key(),
            event.shift_key(),
        );
        let action = held_keys
            .try_update_value(|held| held.route(&event.key(), &event.code(), modifiers, mac, down));
        match action.unwrap_or(KeyAction::App) {
            KeyAction::Send(input) => {
                event.prevent_default();
                event.stop_propagation();
                send(input);
            }
            // The browser's paste event follows, with the clipboard text.
            KeyAction::Paste => event.stop_propagation(),
            KeyAction::Leave => {
                event.prevent_default();
                event.stop_propagation();
                if down {
                    if let Some(field) = address_field.get_untracked() {
                        let _ = field.focus();
                    }
                }
            }
            KeyAction::App => {}
        }
    };
    let on_paste = move |event: web_sys::ClipboardEvent| {
        if !controls.get_untracked().input {
            return;
        }
        event.prevent_default();
        let text = event
            .clipboard_data()
            .and_then(|clipboard| clipboard.get_data("text/plain").ok())
            .unwrap_or_default();
        if !text.is_empty() {
            send(ComputerInput::Text { text });
        }
    };

    // The address shows the page's URL, unless the owner is typing one.
    let address = RwSignal::new(String::new());
    Effect::new(move |_| {
        let url = url.get();
        controls.track();
        let typing = address_field.get_untracked().is_some_and(|field| {
            let field: &web_sys::Element = &field;
            web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element())
                .is_some_and(|active| &active == field)
        });
        if !(typing && controls.get_untracked().navigate) {
            address.set(url);
        }
    });
    let on_address_key = move |event: web_sys::KeyboardEvent| {
        if event.key() == "Enter" && controls.get_untracked().navigate {
            event.prevent_default();
            let url = address.get_untracked().trim().to_string();
            if !url.is_empty() {
                send(ComputerInput::Navigate { url });
            }
        }
    };

    let mut wheel = ev::Custom::<web_sys::WheelEvent>::new("wheel");
    wheel.options_mut().set_passive(false);
    let input_on = move || controls.get().input;
    let live_view = view! {
        <img
            node_ref=img
            data-computer-live=""
            alt=move || if live.has_frame.get() { LIVE_VIEW_LABEL } else { "" }
            draggable="false"
            tabindex=move || input_on().then_some("0")
            aria-label=move || input_on().then_some(LIVE_VIEW_LABEL)
            aria-describedby=move || input_on().then_some("bruh-computer-hint")
            class=move || live_view_class(live.has_frame.get(), input_on())
            on:focus=move |_| focused.set(true)
            on:blur=move |_| {
                focused.set(false);
                release_held();
            }
            on:lostpointercapture=move |_| release_pointer()
            on:pointerdown=on_pointer_down
            on:pointermove=on_pointer_move
            on:pointerup=on_pointer_up
            on:pointercancel=on_pointer_cancel
            on:contextmenu=move |event| {
                if controls.get_untracked().input {
                    event.prevent_default();
                }
            }
            on:keydown=move |event| on_key(event, true)
            on:keyup=move |event| on_key(event, false)
            on:paste=on_paste
        />
    }
    .into_inner()
    .on(wheel, on_wheel);

    // One button each for control and the browser, whose label follows the
    // state, so focus stays on it when Take control becomes Hand back. They
    // stay enabled while a request runs (a disabled button drops focus),
    // marked busy and dimmed; `request` ignores presses until it answers.
    let control_shown = Memo::new(move |_| {
        let controls = controls.get();
        controls.take || controls.hand_back
    });
    let browser_shown = Memo::new(move |_| {
        let controls = controls.get();
        controls.open || controls.close
    });
    let control_button = view! {
        <Show when=move || control_shown.get()>
            <Button
                variant=ButtonVariant::Outline
                class=BUSY
                attr:aria-busy=move || {
                    matches!(pending.get(), Some(ComputerRequest::TakeControl | ComputerRequest::HandBack))
                        .then_some("true")
                }
                on_click=Callback::new(move |_| {
                    request(if controls.get_untracked().hand_back {
                        ComputerRequest::HandBack
                    } else {
                        ComputerRequest::TakeControl
                    })
                })
            >
                <span data-computer-control="">{move || if controls.get().hand_back { HAND_BACK } else { TAKE_CONTROL }}</span>
            </Button>
        </Show>
    };
    let browser_button = view! {
        <Show when=move || browser_shown.get()>
            <Button
                variant=ButtonVariant::Outline
                class=BUSY
                attr:aria-busy=move || {
                    matches!(pending.get(), Some(ComputerRequest::Open | ComputerRequest::Close))
                        .then_some("true")
                }
                on_click=Callback::new(move |_| {
                    request(if controls.get_untracked().close {
                        ComputerRequest::Close
                    } else {
                        ComputerRequest::Open
                    })
                })
            >
                <span data-computer-browser="">{move || if controls.get().close { CLOSE_BROWSER } else { OPEN_BROWSER }}</span>
            </Button>
        </Show>
    };

    view! {
        <div class="mx-auto w-full min-w-0 max-w-5xl space-y-3 p-4 sm:px-6" data-bruh-computer="">
            <p role="status" data-computer-status="" class="text-body-sm text-rambutan-300">{move || status.get()}</p>
            <div class="flex min-w-0 flex-wrap items-center gap-2">
                <input
                    node_ref=address_field
                    type="url"
                    data-computer-address=""
                    aria-label=ADDRESS_LABEL
                    autocomplete="off"
                    spellcheck="false"
                    class=ADDRESS
                    readonly=move || !controls.get().navigate
                    prop:value=move || address.get()
                    on:input=move |event| address.set(event_target_value(&event))
                    on:keydown=on_address_key
                />
                <Show when=move || controls.get().navigate>
                    <Button
                        variant=ButtonVariant::Outline
                        disabled=Signal::derive(move || !live.connected.get())
                        on_click=Callback::new(move |_| send(ComputerInput::Back))
                    >
                        <span data-computer-back="">{BACK}</span>
                    </Button>
                    <Button
                        variant=ButtonVariant::Outline
                        disabled=Signal::derive(move || !live.connected.get())
                        on_click=Callback::new(move |_| send(ComputerInput::Reload))
                    >
                        <span data-computer-reload="">{RELOAD}</span>
                    </Button>
                </Show>
                {control_button}
                {browser_button}
            </div>
            {move || error.get().map(|message| view! {
                <p role="alert" data-computer-error="" class=ERROR>{message}</p>
            })}
            // Always mounted, so the frame element outlives a hidden view. As
            // wide as fits, but no taller than the window leaves room for.
            <div
                class="mx-auto aspect-[16/10] w-full max-w-[min(100%,max(20rem,calc((100dvh_-_20rem)*1.6)))] overflow-hidden rounded-grape bg-longan-900 inset-ring inset-ring-longan-700"
                class:hidden=move || !(on.get() && ready.get())
            >
                {live_view}
            </div>
            <p id="bruh-computer-hint" data-computer-hint="" class="min-h-[1lh] text-label-sm text-rambutan-500">{move || hint.get()}</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::live_view_class;

    #[test]
    fn the_empty_live_view_hides_chromes_broken_image_box() {
        // No frame yet: an `<img>` without `src` draws Chrome's silver
        // broken-image border over the box's ring, which the desktop
        // doesn't have. It stays hidden until a frame, or focus, shows it.
        let empty = live_view_class(false, false);
        assert!(empty.split_whitespace().any(|class| class == "opacity-0"));
        assert!(empty
            .split_whitespace()
            .any(|class| class == "focus:opacity-100"));
        let framed = live_view_class(true, false);
        assert!(!framed
            .split_whitespace()
            .any(|class| class.contains("opacity")));
        let held = live_view_class(true, true);
        assert!(held.split_whitespace().any(|class| class == "touch-none"));
        assert!(!held
            .split_whitespace()
            .any(|class| class.contains("opacity")));
    }
}
