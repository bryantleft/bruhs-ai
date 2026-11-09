//! Modal that appears when the auto-record watcher matches an event
//! but settings.prompt_before_starting is on. Actions:
//!   - Record: trigger auto-start now
//!   - Skip once: in-memory session suppression
//!   - Always skip this event: persistent suppression
//!   - Pause/Resume: freezes the countdown (WCAG 2.2.1 — the user must be
//!     able to extend a time limit before it auto-acts)
//!
//! Built on the `Dialog` primitive so it gets role="dialog", aria-modal,
//! the focus trap, Escape, and scrim dismissal (both map to "skip once").
//!
//! Phase 4c additions:
//! - 30-second countdown: if the user doesn't act, auto-records by default
//!   (since they may be in the meeting, not at the screen).
//! - `AudioPromptCooldownSignal` is updated on every dismissal (Record /
//!   Skip / Always-skip) so that repeated audio-driven prompts are gated
//!   by the 5-minute cooldown.

use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::state::{
    AudioPromptCooldownSignal, PendingAutoRecordSignal, SessionSkippedEventsSignal,
    SetPendingAutoRecordSignal, SetSessionSkippedEventsSignal, SetSuppressedEventsSignal,
};
use crate::ui::{Button, ButtonSize, ButtonVariant, Dialog};

/// Countdown duration in seconds before the modal auto-records.
const COUNTDOWN_SECS: i32 = 30;

#[component]
pub fn AutoRecordPromptModal() -> impl IntoView {
    let PendingAutoRecordSignal(pending) = expect_context::<PendingAutoRecordSignal>();
    let SetPendingAutoRecordSignal(set_pending) = expect_context::<SetPendingAutoRecordSignal>();
    let targets = crate::auto_record::RecordingTargets::from_context();
    let SetSuppressedEventsSignal(set_suppressed) = expect_context::<SetSuppressedEventsSignal>();
    let SessionSkippedEventsSignal(session_skipped) =
        expect_context::<SessionSkippedEventsSignal>();
    let SetSessionSkippedEventsSignal(set_session_skipped) =
        expect_context::<SetSessionSkippedEventsSignal>();
    let audio_cooldown = use_context::<AudioPromptCooldownSignal>().map(|s| s.0);
    let capture = crate::auto_record::CaptureSettings::from_context();
    let capture_options = move || capture.current();

    // Countdown signal — counts from COUNTDOWN_SECS down to 0.
    let (countdown, set_countdown) = signal(COUNTDOWN_SECS);
    // User-controlled freeze for the countdown (WCAG 2.2.1).
    let paused = RwSignal::new(false);
    // The live Interval handle; stored in a StoredValue (LocalStorage so
    // Interval's non-Send inner Rc is acceptable) so it is accessible
    // from both the Effect (which starts it) and the button handlers
    // (which cancel it). We use Option so we can take() it when cancelling.
    let interval_handle: StoredValue<Option<Interval>, LocalStorage> = StoredValue::new_local(None);

    // Helper: bump the cooldown timestamp. Called on every dismissal action
    // to prevent audio-driven re-prompting within the cooldown window.
    let bump_cooldown = move || {
        if let Some(cd) = audio_cooldown {
            cd.set(js_sys::Date::now());
        }
    };

    // Helper: cancel the running interval.
    let cancel_interval = move || {
        interval_handle.update_value(|h: &mut Option<Interval>| {
            if let Some(iv) = h.take() {
                iv.cancel();
            }
        });
    };

    // Single idempotent effect that manages the full countdown lifecycle.
    //
    // Tracks both `pending` and `countdown` so it re-runs on every tick AND
    // on every pending change. On each run:
    //   - If no pending event → cancel any existing interval and reset counter.
    //   - If pending and countdown still positive → ensure an interval is
    //     running (cancel-then-start to avoid double-intervals on rapid
    //     Some→None→Some transitions).
    //   - If pending and countdown reached 0 → fire auto-record.
    Effect::new(move |_| {
        let is_pending = pending.get().is_some();

        if !is_pending {
            // Modal dismissed or never shown — kill any running interval and
            // reset the counter. Critically, do NOT read `countdown` reactively
            // in this branch: the effect also *writes* it here, so subscribing
            // would self-retrigger into an infinite loop (this branch runs on
            // every load, since `pending` starts None). Read untracked + guard
            // the write so it neither subscribes nor fires a redundant set.
            cancel_interval();
            if countdown.get_untracked() != COUNTDOWN_SECS {
                set_countdown.set(COUNTDOWN_SECS);
            }
            // Same untracked-read + guard discipline as `countdown` above.
            if paused.get_untracked() {
                paused.set(false);
            }
            return;
        }

        // User paused the countdown — freeze it (no interval) until resumed.
        // Tracking `paused` here is what re-runs the effect on Resume.
        if paused.get() {
            cancel_interval();
            return;
        }

        // Pending: now it's safe to track `countdown` — the per-second interval
        // owns the writes, so reacting to them is the intended countdown tick.
        let ticks_left = countdown.get();

        if ticks_left <= 0 {
            // Countdown expired — fire auto-record.
            cancel_interval();
            if let Some(event) = pending.get_untracked() {
                let key = event
                    .google_event_id
                    .clone()
                    .unwrap_or_else(|| event.id.clone());
                let mut current = session_skipped.get_untracked();
                current.insert(key);
                set_session_skipped.set(current);
                set_pending.set(None);
                bump_cooldown();
                crate::auto_record::start_recording_for_event(event, capture_options(), targets);
            }
            return;
        }

        // Pending and still counting down — (re)start the interval idempotently.
        // Cancel first so rapid Some→None→Some cycles don't stack intervals.
        cancel_interval();
        let iv = Interval::new(1_000, move || {
            set_countdown.update(|c| {
                *c -= 1;
            });
        });
        interval_handle.set_value(Some(iv));
    });

    let on_record = move |_| {
        cancel_interval();
        if let Some(event) = pending.get_untracked() {
            let key = event
                .google_event_id
                .clone()
                .unwrap_or_else(|| event.id.clone());
            let mut current = session_skipped.get_untracked();
            current.insert(key);
            set_session_skipped.set(current);
            set_pending.set(None);
            bump_cooldown();
            crate::auto_record::start_recording_for_event(event, capture_options(), targets);
        }
    };

    // Shared by the "Skip once" button and the Dialog's Escape/scrim dismiss.
    let skip_once_action = move || {
        cancel_interval();
        if let Some(event) = pending.get_untracked() {
            let key = event.google_event_id.clone().unwrap_or(event.id);
            let mut current = session_skipped.get_untracked();
            current.insert(key);
            set_session_skipped.set(current);
            set_pending.set(None);
            bump_cooldown();
        }
    };
    let on_skip_once = move |_: web_sys::MouseEvent| skip_once_action();

    let on_always_skip = move |_| {
        cancel_interval();
        if let Some(event) = pending.get_untracked() {
            let key = event.google_event_id.clone().unwrap_or(event.id);
            set_suppressed.update(|s| {
                s.insert(key);
            });
            set_pending.set(None);
            bump_cooldown();
        }
    };

    view! {
        <Dialog
            open=Signal::derive(move || pending.get().is_some())
            on_close=Callback::new(move |_| skip_once_action())
            max_width="max-w-md".to_string()
            label="Record this meeting?"
        >
            <div class="p-4 space-y-3">
                <h2 class="text-h4 text-rambutan-100">"Record this meeting?"</h2>
                <p class="text-body-sm text-rambutan-300 leading-snug">
                    {move || pending.get().map(|e| e.title).unwrap_or_default()}
                </p>
                // role="timer" (deliberately NOT aria-live: announcing every
                // tick would spam screen readers). Pause is the WCAG 2.2.1
                // control for the time limit.
                <div role="timer" class="flex items-center gap-2 text-label text-rambutan-400">
                    <span class="tabular-nums">
                        {move || if paused.get() {
                            "Auto-record paused".to_string()
                        } else {
                            format!("Auto-record in {}s", countdown.get().max(0))
                        }}
                    </span>
                    <button
                        type="button"
                        on:click=move |_| paused.update(|p| *p = !*p)
                        class="rounded-seed px-1.5 py-0.5 text-label-sm text-rambutan-300 hover:bg-longan-700/50 hover:text-rambutan-100 transition-colors cursor-pointer"
                    >
                        {move || if paused.get() { "Resume" } else { "Pause" }}
                    </button>
                </div>
                <div class="flex flex-wrap gap-2 pt-1">
                    <Button
                        variant=ButtonVariant::Primary
                        size=ButtonSize::Small
                        on_click=Callback::new(on_record)
                    >
                        "Record"
                    </Button>
                    <Button
                        variant=ButtonVariant::Outline
                        size=ButtonSize::Small
                        on_click=Callback::new(on_skip_once)
                    >
                        "Skip once"
                    </Button>
                    <Button
                        variant=ButtonVariant::Ghost
                        size=ButtonSize::Small
                        on_click=Callback::new(on_always_skip)
                    >
                        "Always skip this event"
                    </Button>
                </div>
            </div>
        </Dialog>
    }
}
