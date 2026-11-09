use bruhs_models::num::ToIntSaturating;
use leptos::{html, prelude::*};
use wasm_bindgen::{closure::Closure, JsCast};

/// A native scroll container, with resize observation owned by the sidebar.
#[derive(Clone, Copy)]
pub struct Viewport {
    pub node: NodeRef<html::Ul>,
    pub top: RwSignal<f64>,
    pub height: RwSignal<f64>,
}

impl Viewport {
    pub fn new() -> Self {
        let viewport = Self {
            node: NodeRef::new(),
            top: RwSignal::new(0.0),
            height: RwSignal::new(900.0),
        };
        let observer =
            StoredValue::new_local(None::<(web_sys::ResizeObserver, Closure<dyn FnMut()>)>);
        Effect::new(move |_| {
            let Some(node) = viewport.node.get() else {
                return;
            };
            viewport.measure();
            let callback = Closure::wrap(Box::new(move || viewport.measure()) as Box<dyn FnMut()>);
            match web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
                Ok(resize) => {
                    resize.observe(&node);
                    observer.set_value(Some((resize, callback)));
                }
                Err(error) => log::warn!("Thread list resize observation unavailable: {error:?}"),
            }
        });
        on_cleanup(move || {
            observer.update_value(|value| {
                if let Some((resize, _callback)) = value.take() {
                    resize.disconnect();
                }
            });
        });
        viewport
    }

    pub fn measure(self) {
        if let Some(node) = self.node.try_get_untracked().flatten() {
            let height = f64::from(node.client_height());
            if height > 0.0 && height != self.height.get_untracked() {
                self.height.set(height);
            }
            let top = f64::from(node.scroll_top());
            if top != self.top.get_untracked() {
                self.top.set(top);
            }
        }
    }

    pub fn scroll(self, top: f64) {
        let Some(node) = self.node.try_get_untracked().flatten() else {
            return;
        };
        // Publish the requested range before the browser's following scroll event.
        self.top.set(top);
        node.set_scroll_top(top.to_i32_saturating());
    }
}
