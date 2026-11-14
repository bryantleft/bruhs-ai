use gloo_net::http::Response;
use js_sys::{Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{ReadableStreamDefaultReader, TextDecodeOptions, TextDecoder};

struct ErrorReader {
    reader: ReadableStreamDefaultReader,
    finished: bool,
}

impl ErrorReader {
    async fn cancel(&mut self) {
        let cancellation = self.reader.cancel();
        self.finished = true;
        let _ = JsFuture::from(cancellation).await;
    }
}

impl Drop for ErrorReader {
    fn drop(&mut self) {
        if !self.finished {
            let cancellation = self.reader.cancel();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(cancellation).await;
            });
        }
        self.reader.release_lock();
    }
}

pub(super) async fn read(response: &Response) -> Option<String> {
    let Some(body) = response.body() else {
        return Some(String::new());
    };
    let Ok(decoder) = TextDecoder::new() else {
        return response.text().await.ok();
    };
    let Ok(reader) = ReadableStreamDefaultReader::new(&body) else {
        return response.text().await.ok();
    };
    let mut reader = ErrorReader {
        reader,
        finished: false,
    };
    let options = TextDecodeOptions::new();
    options.set_stream(true);
    let mut text = String::new();
    loop {
        let part = JsFuture::from(reader.reader.read()).await.ok()?;
        if Reflect::get(&part, &JsValue::from_str("done"))
            .ok()?
            .as_bool()?
        {
            reader.finished = true;
            text.push_str(&decoder.decode().ok()?);
            return Some(text);
        }
        let bytes = Reflect::get(&part, &JsValue::from_str("value"))
            .ok()?
            .dyn_into::<Uint8Array>()
            .ok()?;
        let decoded = decoder
            .decode_with_js_u8_array_and_options(&bytes, &options)
            .ok()?;
        let decoded = if text.is_empty() {
            decoded.trim_start()
        } else {
            &decoded
        };
        // ApiError::http always rejects markup; its remaining body cannot
        // refine the known status. Other bodies still require complete reads.
        if text.is_empty() && decoded.starts_with('<') {
            reader.cancel().await;
            return Some(String::new());
        }
        text.push_str(decoded);
    }
}
