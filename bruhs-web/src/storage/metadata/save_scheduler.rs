use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/storage/metadata/save_scheduler.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = yieldBeforeMetadataSave)]
    fn yield_before_metadata_save() -> Result<js_sys::Promise, JsValue>;
}

pub async fn yield_before_save() {
    // Scheduling is an optimization, never a reason to lose a pending save.
    // The actual persistence errors still propagate through metadata::save.
    if let Ok(promise) = yield_before_metadata_save() {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}
