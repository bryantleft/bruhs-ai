use bruhs_models::logic::rich_response::WidgetState;
use gloo_net::http::Method;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::JsCast;

use crate::api::{self, ApiError};

thread_local! {
    static WIDGET_SNAPSHOTS: std::cell::RefCell<std::collections::BTreeMap<String,(String,WidgetState)>> = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

pub(super) fn remember_state(scope: &str, key: &str, state: &WidgetState) {
    WIDGET_SNAPSHOTS.with(|snapshots| {
        snapshots
            .borrow_mut()
            .insert(key.to_string(), (scope.to_string(), state.clone()))
    });
}

pub(super) fn forget_state(key: &str) {
    WIDGET_SNAPSHOTS.with(|snapshots| snapshots.borrow_mut().remove(key));
}

fn export_states(scope: &str) -> std::collections::BTreeMap<String, WidgetState> {
    let prefix = format!("{scope}/");
    WIDGET_SNAPSHOTS.with(|snapshots| {
        snapshots
            .borrow()
            .iter()
            .filter(|(_, (scope, _))| scope.starts_with(&prefix))
            .map(|(key, (_, state))| (key.clone(), state.clone()))
            .collect()
    })
}

fn export_state_keys(scope: &str, markdown: &str) -> Vec<String> {
    use bruhs_models::logic::rich_response::{response_blocks, widget_state_key, ResponseBlock};
    WIDGET_SNAPSHOTS.with(|snapshots| {
        let snapshots = snapshots.borrow();
        let mut keys = Vec::new();
        for (index, block) in response_blocks(markdown, false).into_iter().enumerate() {
            if !matches!(block, ResponseBlock::Html(_) | ResponseBlock::Artifact(_)) {
                continue;
            }
            let selected = format!("{scope}/{index}");
            if let Some((key, _)) = snapshots.iter().find(|(_, (scope, _))| scope == &selected) {
                keys.push(key.clone());
            } else if let ResponseBlock::Html(html) = block {
                keys.push(widget_state_key(&selected, &html.html));
            } else {
                return Vec::new();
            }
        }
        keys
    })
}

pub(super) async fn json<T: DeserializeOwned>(
    method: Method,
    route: &str,
    body: Option<&impl Serialize>,
) -> Result<T, ApiError> {
    let response = api::send(method, route, body).await?;
    let text = response.text().await?;
    serde_json::from_str(&text).map_err(|error| ApiError::Decode(error.to_string()))
}

pub(super) async fn load_state(key: &str) -> Result<WidgetState, ApiError> {
    api::get_json(&format!(
        "/api/agent/rich-response/state?key={}",
        api::segment(key)
    ))
    .await
}

pub(super) async fn save_state(key: &str, state: &WidgetState) -> Result<WidgetState, ApiError> {
    json(
        Method::PUT,
        &format!("/api/agent/rich-response/state?key={}", api::segment(key)),
        Some(state),
    )
    .await
}

fn state_cache_key(key: &str) -> String {
    format!("bruhs.widget.pending.{key}")
}

fn cache() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or("The browser window is unavailable.")?
        .local_storage()
        .map_err(|_| "Browser storage could not open the widget recovery cache.")?
        .ok_or_else(|| "Browser storage is unavailable for widget recovery.".into())
}

pub(super) fn cached_state(key: &str) -> Result<Option<WidgetState>, String> {
    let Some(json) = cache()?
        .get_item(&state_cache_key(key))
        .map_err(|_| "Could not read the widget recovery cache.")?
    else {
        return Ok(None);
    };
    let state: WidgetState = serde_json::from_str(&json).map_err(|_| {
        "The widget recovery cache could not be decoded. The server copy can still be loaded."
    })?;
    state.validate().map_err(str::to_string)?;
    Ok(Some(state))
}

pub(super) fn cache_state(key: &str, state: &WidgetState) -> Result<(), String> {
    state.validate().map_err(str::to_string)?;
    let json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    cache()?
        .set_item(&state_cache_key(key), &json)
        .map_err(|_| "Browser storage could not keep a local widget recovery copy.".into())
}

pub(super) fn clear_cached_state(key: &str, saved: &WidgetState) -> Result<(), String> {
    let cache = cache()?;
    let key = state_cache_key(key);
    let expected = serde_json::to_string(saved).map_err(|error| error.to_string())?;
    if cache
        .get_item(&key)
        .map_err(|_| "Could not read the widget recovery cache.")?
        .as_deref()
        == Some(expected.as_str())
    {
        cache
            .remove_item(&key)
            .map_err(|_| "Could not clear the saved widget recovery copy.")?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct Captured {
    pub src: String,
    pub signed_url: String,
}

pub(super) async fn capture_remote(url: &str) -> Result<Captured, ApiError> {
    json(
        Method::POST,
        "/api/agent/rich-response/remote-media",
        Some(&serde_json::json!({"url":url})),
    )
    .await
}

pub(super) async fn capture_media(src: &str, project: Option<&str>) -> Result<Captured, ApiError> {
    let reply: Captured = json(
        Method::POST,
        "/api/agent/rich-response/media",
        Some(&serde_json::json!({ "src": src, "project": project })),
    )
    .await?;
    Ok(reply)
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Artifact {
    pub id: String,
    pub title: String,
    pub html: String,
}

pub(super) async fn artifact(id: &str) -> Result<Artifact, ApiError> {
    api::get_json(&format!("/api/agent/artifacts/{}", api::segment(id))).await
}

pub(super) async fn diagnostics(id: &str, errors: &[String]) -> Result<(), ApiError> {
    api::send(
        Method::POST,
        &format!("/api/agent/artifacts/{}/diagnostics", api::segment(id)),
        Some(&serde_json::json!({
            "state": if errors.is_empty() { "ready" } else { "error" },
            "errors": errors,
        })),
    )
    .await?;
    Ok(())
}

pub(super) async fn export(
    markdown: &str,
    project: Option<&str>,
    scope: &str,
    include_remote: bool,
    include_state: bool,
) -> Result<(), String> {
    let widget_states = if include_state {
        export_states(scope)
    } else {
        std::collections::BTreeMap::new()
    };
    let state_keys = if include_state {
        export_state_keys(scope, markdown)
    } else {
        Vec::new()
    };
    let response = api::send(
        Method::POST,
        "/api/agent/rich-response/export",
        Some(&serde_json::json!({ "markdown": markdown, "project": project, "scope": scope, "include_remote": include_remote, "include_state": include_state, "widget_states":widget_states, "state_keys":state_keys })),
    )
    .await
    .map_err(|error| error.to_string())?;
    download_response(response, "reply.zip").await
}

pub(super) async fn download_remote(url: &str) -> Result<(), String> {
    let response = api::send(
        Method::POST,
        "/api/agent/rich-response/download",
        Some(&serde_json::json!({"url":url})),
    )
    .await
    .map_err(|error| error.to_string())?;
    download_response(response, "media").await
}

async fn download_response(
    response: gloo_net::http::Response,
    fallback_name: &str,
) -> Result<(), String> {
    let disposition = response
        .headers()
        .get("content-disposition")
        .unwrap_or_default();
    let name = disposition
        .split("filename=\"")
        .nth(1)
        .and_then(|value| value.split('"').next())
        .unwrap_or(fallback_name)
        .to_string();
    // Browser-owned blobs keep large recordings and bundles out of WASM memory.
    let response: web_sys::Response = response.into();
    let blob = wasm_bindgen_futures::JsFuture::from(
        response
            .blob()
            .map_err(|_| "The browser could not receive this download.")?,
    )
    .await
    .map_err(|_| "The media download did not finish.")?
    .dyn_into::<web_sys::Blob>()
    .map_err(|_| "The browser could not package this media.")?;
    download_blob(&blob, &name)
}

fn download_blob(blob: &web_sys::Blob, name: &str) -> Result<(), String> {
    let url = web_sys::Url::create_object_url_with_blob(blob)
        .map_err(|_| "The browser could not open this download.")?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("The browser document is unavailable.")?;
        let anchor = document
            .create_element("a")
            .map_err(|_| "The browser could not create a download link.")?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "The browser could not use the download link.")?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.click();
        Ok(())
    })();
    let _ = web_sys::Url::revoke_object_url(&url);
    result
}

pub(super) fn download(bytes: &[u8], mime: &str, name: &str) -> Result<(), String> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let properties = web_sys::BlobPropertyBag::new();
    properties.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &properties)
        .map_err(|_| "The browser could not package this download.")?;
    download_blob(&blob, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::logic::rich_response::{response_blocks, widget_state_key, ResponseBlock};

    #[test]
    fn state_exports_select_current_reply_snapshots_in_widget_order() {
        let scope = "thread/message/text";
        let markdown = "```bruhs-html\n<input value='original'>\n```\n\n```bruhs-artifact\n{\"id\":\"00000000-0000-4000-8000-000000000001\"}\n```";
        let mut expected = Vec::new();
        for (index, block) in response_blocks(markdown, false).into_iter().enumerate() {
            let html = match block {
                ResponseBlock::Html(html) => html.html,
                ResponseBlock::Artifact(_) => "<input value='artifact'>".into(),
                _ => continue,
            };
            let selected = format!("{scope}/{index}");
            let key = widget_state_key(&selected, &html);
            remember_state(&selected, &key, &WidgetState::default());
            expected.push(key);
        }
        assert_eq!(expected.len(), 2);
        let current = WidgetState {
            custom: serde_json::json!({ "typed": "pending PUT" }),
            ..Default::default()
        };
        let selected = WIDGET_SNAPSHOTS
            .with(|snapshots| snapshots.borrow().get(&expected[0]).unwrap().0.clone());
        remember_state(&selected, &expected[0], &current);
        remember_state("thread/another/text/0", "unrelated", &current);
        assert_eq!(export_state_keys(scope, markdown), expected);
        let snapshots = export_states(scope);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots.get(&expected[0]), Some(&current));
        for key in expected.iter().map(String::as_str).chain(["unrelated"]) {
            forget_state(key);
        }
    }
}
