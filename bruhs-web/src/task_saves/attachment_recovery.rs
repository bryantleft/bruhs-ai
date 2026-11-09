//! Keep recovery text bounded without discarding large, unsaved binary files.
use std::sync::Arc;

use bruhs_models::KnowledgeAttachment;
use serde::{Serialize, Serializer};
use wasm_bindgen::JsCast;

use super::Record;

const INLINE_BYTES_LIMIT: usize = 64 * 1024;

pub(super) struct RecoveryRecord<'a> {
    pub record: &'a Record,
    pub role: &'static str,
}

#[derive(Clone)]
pub(super) struct RecoveryAttachment {
    pub key: String,
    pub download_name: String,
    pub attachment: Arc<KnowledgeAttachment>,
}

impl RecoveryAttachment {
    pub fn from_record(record: &Record, role: &'static str) -> Option<Self> {
        let Record::Attachment(attachment) = record else {
            return None;
        };
        (attachment.bytes.len() > INLINE_BYTES_LIMIT).then(|| Self {
            key: format!("{role}-{}", attachment.id),
            // The metadata retains the original name; the recovery file uses
            // a short identity to avoid filesystem name limits/path separators.
            download_name: format!("{role}-{}.bin", attachment.id),
            attachment: attachment.clone(),
        })
    }
}

impl Serialize for RecoveryRecord<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Some(file) = RecoveryAttachment::from_record(self.record, self.role) else {
            return self.record.serialize(serializer);
        };
        #[derive(Serialize)]
        struct BinaryReference<'a> {
            id: &'a str,
            document_id: &'a str,
            file_name: &'a str,
            mime_type: &'a str,
            created_at_ms: i64,
            bytes_file: &'a str,
            byte_length: usize,
        }
        let attachment = &file.attachment;
        BinaryReference {
            id: &attachment.id,
            document_id: &attachment.document_id,
            file_name: &attachment.file_name,
            mime_type: &attachment.mime_type,
            created_at_ms: attachment.created_at_ms,
            bytes_file: &file.download_name,
            byte_length: attachment.bytes.len(),
        }
        .serialize(serializer)
    }
}

pub(super) fn download(file: &RecoveryAttachment) -> Result<(), &'static str> {
    let bytes = js_sys::Uint8Array::from(file.attachment.bytes.as_slice());
    let parts = js_sys::Array::new();
    parts.push(&bytes.buffer());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)
        .map_err(|_| "Could not prepare the recovery file. Keep this tab open and try again.")?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| {
        "Could not prepare the recovery download. Keep this tab open and try again."
    })?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("The download window is unavailable. Keep this tab open.")?;
        let anchor = document
            .create_element("a")
            .ok()
            .and_then(|element| element.dyn_into::<web_sys::HtmlAnchorElement>().ok())
            .ok_or("Could not start the download. Keep this tab open and try again.")?;
        anchor.set_href(&url);
        anchor.set_download(&file.download_name);
        anchor.click();
        Ok(())
    })();
    let _ = web_sys::Url::revoke_object_url(&url);
    result
}
