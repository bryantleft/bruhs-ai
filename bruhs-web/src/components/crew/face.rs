//! A bruh's face: the dithered Bruh drawn for its look and status. A working
//! bruh sweeps its gaze, but only while it is on screen, the tab is visible
//! and the person has not asked for reduced motion.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use bruhs_models::crew::{face_view, BruhLook, BruhStatus};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::ui::BrailleBruh;

/// Fewest milliseconds between two redraws of a sweep (about 30 frames a second).
const MIN_FRAME_MS: f64 = 33.0;

/// Whether the sweep should be redrawing right now.
fn sweep_runs(animates: bool, intersecting: bool, document_visible: bool) -> bool {
    animates && intersecting && document_visible
}

/// Whether enough time has passed since the last redraw to draw another.
fn frame_due(last_ms: f64, now_ms: f64) -> bool {
    now_ms - last_ms >= MIN_FRAME_MS
}

fn document_visible() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .is_none_or(|document| !document.hidden())
}

type FrameCallback = Closure<dyn FnMut(f64)>;

/// What `run_sweep` shares with the component: when to stop, and the pending
/// animation frame to cancel.
#[derive(Clone, Default)]
struct SweepControl {
    running: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    frame: Arc<AtomicI32>,
}

/// Redraws `now` on animation frames while `animates` holds, then stops.
fn run_sweep(
    now: RwSignal<i64>,
    intersecting: RwSignal<bool>,
    animates: Memo<bool>,
    control: &SweepControl,
    slot: StoredValue<Option<Rc<RefCell<Option<FrameCallback>>>>, LocalStorage>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    control.running.store(true, Ordering::Release);
    let cell: Rc<RefCell<Option<FrameCallback>>> = Rc::default();
    let again: Weak<RefCell<Option<FrameCallback>>> = Rc::downgrade(&cell);
    let frame_id = control.frame.clone();
    let control = control.clone();
    let mut last = f64::NEG_INFINITY;
    let win = window.clone();
    *cell.borrow_mut() = Some(Closure::new(move |frame_ms: f64| {
        if control.stopped.load(Ordering::Acquire) || !animates.try_get_untracked().unwrap_or(false)
        {
            control.running.store(false, Ordering::Release);
            return;
        }
        let on_screen = intersecting.try_get_untracked().unwrap_or(false);
        if sweep_runs(true, on_screen, document_visible()) && frame_due(last, frame_ms) {
            last = frame_ms;
            let _ = now.try_set(crate::clock::now_ms());
        }
        if let Some(cell) = again.upgrade() {
            if let Some(callback) = cell.borrow().as_ref() {
                if let Ok(id) = win.request_animation_frame(callback.as_ref().unchecked_ref()) {
                    control.frame.store(id, Ordering::Release);
                }
            }
        }
    }));
    if let Some(callback) = cell.borrow().as_ref() {
        if let Ok(id) = window.request_animation_frame(callback.as_ref().unchecked_ref()) {
            frame_id.store(id, Ordering::Release);
        }
    }
    slot.set_value(Some(cell));
}

type MediaWatch = (web_sys::MediaQueryList, Closure<dyn FnMut(web_sys::Event)>);
type Watch = (
    web_sys::IntersectionObserver,
    Closure<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)>,
);

/// A bruh's face `size` CSS pixels square, for its look and status. Decorative.
#[component]
pub fn BruhFace(
    size: f64,
    #[prop(into)] look: Signal<BruhLook>,
    #[prop(into)] status: Signal<BruhStatus>,
) -> impl IntoView {
    let now = RwSignal::new(crate::clock::now_ms());
    let reduced = RwSignal::new(false);
    let intersecting = RwSignal::new(true);
    let view = Memo::new(move |_| face_view(&look.get(), status.get(), now.get(), reduced.get()));
    let animates = Memo::new(move |_| view.get().animates);

    // The reduced-motion preference, kept current.
    let media = StoredValue::new_local(None::<MediaWatch>);
    if let Some(list) = web_sys::window()
        .and_then(|window| window.match_media("(prefers-reduced-motion: reduce)").ok())
        .flatten()
    {
        reduced.set(list.matches());
        let watched = list.clone();
        let on_change = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let _ = reduced.try_set(watched.matches());
        });
        let _ = list.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
        media.set_value(Some((list, on_change)));
    }

    // Whether the face is on screen.
    let wrapper = NodeRef::<html::Span>::new();
    let observer = StoredValue::new_local(None::<Watch>);
    Effect::new(move |_| {
        let Some(element) = wrapper.get() else {
            return;
        };
        if observer.with_value(Option::is_some) {
            return;
        }
        let on_entries =
            Closure::<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)>::new(
                move |entries: js_sys::Array, _| {
                    if let Some(entry) = entries.iter().last().and_then(|entry| {
                        entry.dyn_into::<web_sys::IntersectionObserverEntry>().ok()
                    }) {
                        let _ = intersecting.try_set(entry.is_intersecting());
                    }
                },
            );
        if let Ok(watch) = web_sys::IntersectionObserver::new(on_entries.as_ref().unchecked_ref()) {
            watch.observe(&element);
            observer.set_value(Some((watch, on_entries)));
        }
    });

    // The sweep: one frame loop while the face animates.
    let control = SweepControl::default();
    let slot = StoredValue::new_local(None::<Rc<RefCell<Option<FrameCallback>>>>);
    let loop_control = control.clone();
    Effect::new(move |_| {
        if animates.get() && !loop_control.running.load(Ordering::Acquire) {
            run_sweep(now, intersecting, animates, &loop_control, slot);
        }
    });

    on_cleanup(move || {
        control.stopped.store(true, Ordering::Release);
        if let Some(window) = web_sys::window() {
            let _ = window.cancel_animation_frame(control.frame.load(Ordering::Acquire));
        }
        media.update_value(|watch| {
            if let Some((list, callback)) = watch.take() {
                let _ = list.remove_event_listener_with_callback(
                    "change",
                    callback.as_ref().unchecked_ref(),
                );
            }
        });
        observer.update_value(|watch| {
            if let Some((watch, _callback)) = watch.take() {
                watch.disconnect();
            }
        });
    });

    view! {
        <span node_ref=wrapper aria-hidden="true" class="inline-flex shrink-0">
            <BrailleBruh size=size view=Signal::from(view) />
        </span>
    }
}

// Nothing renders a face until the crew rail lands; naming the component here
// keeps it (and what it uses) from being dead code until then. The rail task
// deletes this line.
const _: fn() = || {
    let _ = BruhFace;
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_runs_only_for_visible_animated_faces() {
        assert!(sweep_runs(true, true, true));
        assert!(
            !sweep_runs(true, false, true)
                && !sweep_runs(true, true, false)
                && !sweep_runs(false, true, true)
        );
        assert!(!frame_due(1000.0, 1020.0) && frame_due(1000.0, 1034.0));
    }
}
