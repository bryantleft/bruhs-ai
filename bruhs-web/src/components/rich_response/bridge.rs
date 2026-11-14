use bruhs_models::logic::rich_response::{
    WidgetEvent, WidgetMessage, WidgetState, WIDGET_PROTOCOL,
};
use leptos::prelude::*;
use serde::Serialize;
use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen(module = "/src/components/rich_response/bridge.js")]
extern "C" {
    #[wasm_bindgen(js_name = connectWidget)]
    fn connect_widget(
        frame: &web_sys::Element,
        nonce: &str,
        callback: &js_sys::Function,
    ) -> JsValue;
    #[wasm_bindgen(js_name = initializeWidget)]
    fn initialize_widget(handle: &JsValue, state: &JsValue);
    #[wasm_bindgen(js_name = disposeWidget)]
    fn dispose_widget(handle: &JsValue);
    #[wasm_bindgen(catch, js_name = fullscreen)]
    async fn request_fullscreen(element: &web_sys::Element) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = openDialog)]
    pub(super) fn open_dialog(element: &web_sys::Element) -> Result<(), JsValue>;
    #[wasm_bindgen(js_name = widgetTheme)]
    fn widget_theme() -> JsValue;
    #[wasm_bindgen(js_name = watchFrameInteraction)]
    fn watch_frame_interaction(element: &web_sys::Element, callback: &js_sys::Function) -> JsValue;
    #[wasm_bindgen(js_name = disposeWatcher)]
    fn dispose_watcher(handle: &JsValue);
}

pub(super) fn watch_interaction(element: web_sys::Element, on_interaction: Option<Callback<()>>) {
    let Some(on_interaction) = on_interaction else {
        return;
    };
    let callback = Closure::<dyn FnMut()>::new(move || on_interaction.run(()));
    let handle = StoredValue::new_local(watch_frame_interaction(
        &element,
        callback.as_ref().unchecked_ref(),
    ));
    let callback = StoredValue::new_local(Some(callback));
    on_cleanup(move || {
        if let Some(handle) = handle.try_get_value() {
            dispose_watcher(&handle);
        }
        callback.try_update_value(|callback| {
            callback.take();
        });
    });
}

pub(super) fn theme() -> Result<std::collections::BTreeMap<String, String>, String> {
    serde_wasm_bindgen::from_value(widget_theme())
        .map_err(|error| format!("Could not read the widget theme: {error}"))
}

pub(super) fn install(
    element: web_sys::Element,
    nonce: String,
    state: WidgetState,
    on_event: Callback<WidgetEvent>,
) {
    let expected_nonce = nonce.clone();
    let callback = Closure::<dyn FnMut(String)>::new(move |json: String| {
        let Ok(packet) = serde_json::from_str::<WidgetMessage>(&json) else {
            return;
        };
        if packet.bruhs != WIDGET_PROTOCOL || packet.nonce != expected_nonce {
            return;
        }
        if let WidgetEvent::State { state } = &packet.event {
            if let Err(message) = state.validate() {
                on_event.run(WidgetEvent::Error {
                    message: message.into(),
                });
                return;
            }
        }
        on_event.run(packet.event);
    });
    let handle = connect_widget(&element, &nonce, callback.as_ref().unchecked_ref());
    if let Ok(state) = state.serialize(&serde_wasm_bindgen::Serializer::json_compatible()) {
        initialize_widget(&handle, &state);
    }
    let handle = StoredValue::new_local(handle);
    let callback = StoredValue::new_local(Some(callback));
    on_cleanup(move || {
        if let Some(handle) = handle.try_get_value() {
            dispose_widget(&handle);
        }
        callback.try_update_value(|callback| {
            callback.take();
        });
    });
}

pub(super) fn fullscreen(element: web_sys::Element, error: RwSignal<Option<String>>) {
    leptos::task::spawn_local(async move {
        if let Err(reason) = request_fullscreen(&element).await {
            let message = reason
                .as_string()
                .unwrap_or_else(|| "Fullscreen could not open. Check browser permissions.".into());
            let _ = error.try_set(Some(message));
        }
    });
}
