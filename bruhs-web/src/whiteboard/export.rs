use super::canvas::BoardCanvas;
use bruhs_models::{num::ToIntSaturating as _, whiteboard::*};
use std::rc::Rc;
use wasm_bindgen::{JsCast as _, JsValue};
pub fn download(name: &str, mime: &str, bytes: &[u8]) -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("Document unavailable"))?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let result = (|| {
        let link = document
            .create_element("a")?
            .dyn_into::<web_sys::HtmlAnchorElement>()?;
        link.set_href(&url);
        link.set_download(name);
        document
            .body()
            .ok_or_else(|| JsValue::from_str("Body unavailable"))?
            .append_child(&link)?;
        link.click();
        link.remove();
        Ok(())
    })();
    web_sys::Url::revoke_object_url(&url)?;
    result
}
pub async fn png(portable: PortableBoard) -> Result<Vec<u8>, JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("Document unavailable"))?;
    wasm_bindgen_futures::JsFuture::from(document.fonts().load("18px MonaspaceArgonVariable"))
        .await?;
    wasm_bindgen_futures::JsFuture::from(document.fonts().ready()?).await?;
    let editor =
        BoardEditor::new(portable.board.clone()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let area = editor
        .bounds()
        .unwrap_or(Rect::new(0.0, 0.0, 640.0, 480.0))
        .expanded(20.0);
    let scale = (bruhs_models::num::ToFloatLossy::to_f64_lossy(
        bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES,
    ) / (area.w * area.h * 4.0))
        .sqrt()
        .min(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(JsValue::from_str(
            "The board bounds cannot be rasterized. Export SVG instead.",
        ));
    }
    let width = (area.w * scale).ceil().max(1.0);
    let height = (area.h * scale).ceil().max(1.0);
    let element = document
        .create_element("canvas")?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    let mut painter = BoardCanvas::new(element, width, height, 1.0)?;
    painter.install_assets(&portable.board, &portable.payloads, Rc::new(|| {}))?;
    painter.wait_images().await?;
    let mut editor = editor;
    editor.camera = Camera {
        x: area.x,
        y: area.y,
        zoom: scale,
    };
    painter.draw(&editor)?;
    if painter.element.width() != width.to_u32_saturating()
        || painter.element.height() != height.to_u32_saturating()
    {
        return Err(JsValue::from_str(
            "The browser could not allocate the export canvas. Export SVG instead.",
        ));
    }
    let mut kept_callback = None;
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let encoding_error = reject.clone();
        let callback = wasm_bindgen::closure::Closure::once(move |blob: Option<web_sys::Blob>| {
            if let Some(blob) = blob {
                let _ = resolve.call1(&JsValue::NULL, &blob);
            } else {
                let _ = reject.call1(
                    &JsValue::NULL,
                    &JsValue::from_str("PNG export failed. Export SVG instead."),
                );
            }
        });
        if let Err(error) = painter
            .element
            .to_blob_with_type(callback.as_ref().unchecked_ref(), "image/png")
        {
            let _ = encoding_error.call1(&JsValue::NULL, &error);
        }
        kept_callback = Some(callback);
    });
    let blob = wasm_bindgen_futures::JsFuture::from(promise)
        .await?
        .dyn_into::<web_sys::Blob>()?;
    drop(kept_callback);
    let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer()).await?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
