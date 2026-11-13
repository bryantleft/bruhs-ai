//! Batch the WASM/JS crossing for ordinary metadata without changing its schema.

use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;

use super::json_safe::JsonSafe;
use crate::storage::{from_js, to_js};

#[wasm_bindgen(module = "/src/storage/metadata/json_bridge.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = stringifyMetadata)]
    fn stringify_metadata(value: &JsValue) -> Result<JsValue, JsValue>;
}

pub(in crate::storage) fn json_string<T: Serialize>(value: &T) -> Option<String> {
    value.serialize(JsonSafe).ok()?;
    serde_json::to_string(value).ok()
}

pub(in crate::storage) fn encode<T: Serialize>(value: &T) -> Result<JsValue, idb::Error> {
    if let Some(json) = json_string(value) {
        if let Ok(encoded) = js_sys::JSON::parse(&json) {
            return Ok(encoded);
        }
    }
    // Big integers, non-finite floats and other special serde representations
    // keep the established structured encoding (including its error behavior).
    to_js(value)
}

pub(super) fn decode<T: DeserializeOwned>(value: JsValue) -> Result<T, idb::Error> {
    if let Ok(json) = stringify_metadata(&value) {
        if let Some(json) = json.as_string() {
            if let Ok(decoded) = serde_json::from_str(&json) {
                return Ok(decoded);
            }
        }
    }
    from_js(value)
}
