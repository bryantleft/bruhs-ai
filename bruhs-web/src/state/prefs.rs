//! Small `localStorage` preferences.

/// Read a small string preference from `window.localStorage`.
/// Returns `None` on any browser/DOM error or when the key is missing.
pub fn load_pref(key: &str) -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|ls| ls.get_item(key).ok().flatten())
}

/// Write a small string preference to `window.localStorage`. Best-effort —
/// failures are silent (private mode, quota, etc.).
pub fn save_pref(key: &str, value: &str) {
    if let Some(ls) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = ls.set_item(key, value);
    }
}

/// Load a JSON-serialized preference from localStorage, deserializing into `T`.
/// Returns `None` if the key is missing, the value is not valid UTF-8, or
/// deserialization fails.
pub(crate) fn load_json_pref<T: for<'de> serde::Deserialize<'de>>(key: &str) -> Option<T> {
    load_pref(key).and_then(|s| serde_json::from_str::<T>(&s).ok())
}

/// Serialize `value` to JSON and write it to localStorage. Best-effort —
/// failures are silent.
pub(crate) fn save_json_pref<T: serde::Serialize>(key: &str, value: &T) {
    if let Ok(s) = serde_json::to_string(value) {
        save_pref(key, &s);
    }
}
