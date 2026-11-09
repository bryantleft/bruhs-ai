use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, Write};

// Leave room for the response envelope, record ID and section inside 512 KiB.
const PAGE_BYTES: usize = 511 * 1024;
const RECORD_LIMIT: usize = 50;

struct BoundedJson {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other(
                "Workspace record exceeds the page byte limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn read_page<T: Serialize>(
    records: impl Iterator<Item = T>,
    offset: usize,
) -> Result<Value, String> {
    if offset > 100_000 {
        return Err("Offset is too large.".into());
    }
    let mut values = Vec::new();
    let mut remaining = PAGE_BYTES;
    let mut next_offset = None;
    for record in records.skip(offset) {
        if values.len() == RECORD_LIMIT {
            next_offset = Some(offset + values.len());
            break;
        }
        let mut encoded = BoundedJson {
            bytes: Vec::new(),
            limit: remaining,
        };
        if let Err(error) = serde_json::to_writer(&mut encoded, &record) {
            if error.is_io() && !values.is_empty() {
                next_offset = Some(offset + values.len());
                break;
            }
            return Err("A workspace record cannot fit in one response or could not be encoded. No records were changed.".into());
        }
        remaining = remaining.saturating_sub(encoded.bytes.len() + 1);
        values.push(
            serde_json::from_slice::<Value>(&encoded.bytes)
                .map_err(|_| "A workspace record could not be decoded.")?,
        );
    }
    Ok(json!({"records": values, "next_offset": next_offset}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfiltered_pages_skip_encoding_earlier_records_and_lookahead() {
        struct Record(usize);
        impl Serialize for Record {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                assert!(
                    (50..100).contains(&self.0),
                    "must encode only requested page"
                );
                serializer.serialize_u64(self.0 as u64)
            }
        }
        let page = read_page((0..123).map(Record), 50).unwrap();
        assert_eq!(page["records"].as_array().unwrap().len(), 50);
        assert_eq!(page["next_offset"], 100);
    }

    #[test]
    fn byte_limited_pages_return_resume_offset_without_losing_records() {
        let body = "a".repeat(300_000);
        let records = [body.as_str(), body.as_str()];
        let first = read_page(records.iter(), 0).unwrap();
        assert_eq!(first["records"].as_array().unwrap().len(), 1);
        assert_eq!(first["next_offset"], 1);
        let second = read_page(records.iter(), 1).unwrap();
        assert_eq!(second["records"], first["records"]);
        assert!(second["next_offset"].is_null());
    }

    #[test]
    fn encoder_bounds_escaped_bytes_and_rejects_individually_oversized_records() {
        let mut writer = BoundedJson {
            bytes: Vec::new(),
            limit: 100,
        };
        assert!(serde_json::to_writer(&mut writer, &"\n".repeat(100)).is_err());
        assert!(writer.bytes.len() <= 100);
        assert!(read_page(["x".repeat(PAGE_BYTES + 1)].iter(), 0).is_err());
        assert!(read_page([1].iter(), 100_001).is_err());
    }
}
