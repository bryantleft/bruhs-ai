use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Move keyboard focus after route rendering, without an animated scroll delay.
pub(super) fn focus_record(id: String) {
    leptos::prelude::request_animation_frame(move || {
        if let Some(element) = document().get_element_by_id(&id) {
            if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
                let _ = element.focus();
                let options = web_sys::ScrollIntoViewOptions::new();
                options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                options.set_inline(web_sys::ScrollLogicalPosition::Nearest);
                element.scroll_into_view_with_scroll_into_view_options(&options);
            }
        }
    });
}
