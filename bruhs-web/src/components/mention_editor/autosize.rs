use leptos::{html, prelude::*};
use wasm_bindgen::{closure::Closure, JsCast, JsValue};

use super::{grow_height, EDITOR_MAX_HEIGHT_PX, EDITOR_MIN_HEIGHT_PX};

type WidthObserver = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

/// Publish the value before measuring, including when a retained draft remounts.
pub(super) fn bind_value(node: NodeRef<html::Textarea>, value: ReadSignal<String>, grow: bool) {
    Effect::new(move |_| {
        value.track();
        let Some(textarea) = node.get() else {
            return;
        };
        value.with_untracked(|text| {
            // Reassigning an unchanged native value can disturb selection/IME.
            if textarea.value() != *text {
                textarea.set_value(text);
                let end = u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX);
                let _ = textarea.set_selection_start(Some(end));
                let _ = textarea.set_selection_end(Some(end));
            }
        });
        if grow {
            measure(&textarea);
        }
    });
    if grow {
        observe_width(node);
    }
}

fn measure(textarea: &web_sys::HtmlTextAreaElement) {
    if let Err(error) = resize(textarea) {
        log::warn!("Composer sizing unavailable: {error:?}");
    }
}

fn resize(textarea: &web_sys::HtmlTextAreaElement) -> Result<(), JsValue> {
    let style = web_sys::HtmlElement::style(textarea);
    let scroll_top = textarea.scroll_top();
    style.set_property("height", "auto")?;
    let (height, scroll) = grow_height(
        textarea.scroll_height(),
        EDITOR_MIN_HEIGHT_PX,
        EDITOR_MAX_HEIGHT_PX,
    );
    style.set_property("height", &format!("{height}px"))?;
    style.set_property("overflow-y", if scroll { "auto" } else { "hidden" })?;
    textarea.set_scroll_top(scroll_top);
    Ok(())
}

fn observe_width(node: NodeRef<html::Textarea>) {
    let observer = StoredValue::new_local(None::<WidthObserver>);
    Effect::new(move |_| {
        let Some(textarea) = node.get() else {
            return;
        };
        observer.update_value(disconnect);
        let mut last_width = 0;
        let callback = Closure::wrap(Box::new(move || {
            let Some(textarea) = node.try_get_untracked().flatten() else {
                return;
            };
            let width = textarea.client_width();
            // Our own height writes also notify ResizeObserver. Ignore those.
            if width > 0 && width != last_width {
                last_width = width;
                measure(&textarea);
            }
        }) as Box<dyn FnMut()>);
        match web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            Ok(resize) => {
                resize.observe(&textarea);
                observer.set_value(Some((resize, callback)));
            }
            Err(error) => log::warn!("Composer width observation unavailable: {error:?}"),
        }
    });
    on_cleanup(move || observer.update_value(disconnect));
}

fn disconnect(value: &mut Option<WidthObserver>) {
    if let Some((resize, _callback)) = value.take() {
        resize.disconnect();
    }
}
