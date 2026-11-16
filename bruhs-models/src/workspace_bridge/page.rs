//! Bounded read pages: at most [`RECORD_LIMIT`] records within a 512 KiB
//! encoded budget, with `next_offset` to resume. A record that cannot fit on
//! its own fails explicitly; nothing is truncated.
use super::{LowercaseQuery, MAX_OFFSET, MAX_QUERY_BYTES, RECORD_LIMIT};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, Write};

// Leave room for the response envelope, record ID and section inside 512 KiB.
const PAGE_BYTES: usize = 511 * 1024;
/// The whole-response budget of [`read_records`].
const RESPONSE_BYTES: usize = 512 * 1024;

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

// An unfiltered page can skip directly through the source iterator. Keeping a
// filter that always returns true would still visit every preceding record.
pub fn read_matching_page<R, T: Serialize>(
    records: impl Iterator<Item = R>,
    unfiltered: bool,
    predicate: impl FnMut(&R) -> bool,
    project: impl FnMut(R) -> T,
    offset: usize,
) -> Result<Value, String> {
    if unfiltered {
        encode_page(records.skip(offset).map(project), offset)
    } else {
        encode_page(records.filter(predicate).skip(offset).map(project), offset)
    }
}

pub fn read_page<T: Serialize>(
    records: impl Iterator<Item = T>,
    offset: usize,
) -> Result<Value, String> {
    encode_page(records.skip(offset), offset)
}

fn encode_page<T: Serialize>(
    records: impl Iterator<Item = T>,
    offset: usize,
) -> Result<Value, String> {
    if offset > MAX_OFFSET {
        return Err("Offset is too large.".into());
    }
    let mut values = Vec::new();
    let mut remaining = PAGE_BYTES;
    let mut next_offset = None;
    for record in records {
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

/// Whole records of a generic domain read (Tasks, Calendar, Capture,
/// Knowledge, Inbox). `query` matches the record's complete JSON encoding,
/// case-insensitively; `offset` counts matching records.
pub fn read_records<T: Serialize>(
    records: &[T],
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    if query.len() > MAX_QUERY_BYTES || offset > MAX_OFFSET {
        return Err("Query or offset is too large.".into());
    }
    let query = LowercaseQuery::new(query);
    let mut matcher = query.matcher();
    let mut encoded = Vec::new();
    let mut matching = Vec::new();
    let mut skipped = 0;
    for record in records {
        // Unfiltered paging must not serialize all earlier pages on the UI thread.
        if query.is_empty() && skipped < offset {
            skipped += 1;
            continue;
        }
        let value =
            serde_json::to_value(record).map_err(|_| "A workspace record could not be encoded.")?;
        if !query.is_empty() {
            encoded.clear();
            serde_json::to_writer(&mut encoded, &value)
                .map_err(|_| "A workspace record could not be encoded.")?;
            // SAFETY: the cleared buffer was written only by the successful
            // default serde_json serializer, which always emits valid UTF-8.
            // serde_json::to_string relies on the same guarantee.
            let text = unsafe { std::str::from_utf8_unchecked(&encoded) };
            if !matcher.matches(text) {
                continue;
            }
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        matching.push(value);
        if matching.len() > RECORD_LIMIT {
            break;
        }
    }
    // Scanning is finished. Release a large normalized field before allocating
    // the final response envelope and checking its encoded size.
    drop(matcher);
    let more = matching.len() > RECORD_LIMIT;
    matching.truncate(RECORD_LIMIT);
    let result = json!({"records":matching,"next_offset":more.then_some(offset + RECORD_LIMIT)});
    if result.to_string().len() > RESPONSE_BYTES {
        return Err("Records are too large for one response. Use a more specific query.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unfiltered_matching_pages_skip_predicates_and_prior_projections() {
        let projected = std::cell::Cell::new(0);
        let page = read_matching_page(
            0..123,
            true,
            |_| panic!("an unfiltered page must not inspect skipped records"),
            |record| {
                assert!((50..=100).contains(&record));
                projected.set(projected.get() + 1);
                record
            },
            50,
        )
        .unwrap();
        assert_eq!(
            page["records"],
            serde_json::json!((50..100).collect::<Vec<_>>())
        );
        assert_eq!(page["next_offset"], 100);
        // The existing page reader projects one lookahead without encoding it.
        assert_eq!(projected.get(), 51);
    }

    #[test]
    fn filtered_matching_pages_keep_offsets_relative_to_matches() {
        let first = read_matching_page(
            0..123,
            false,
            |r| r % 2 == 0,
            |r| {
                assert!(r >= 20);
                r
            },
            10,
        )
        .unwrap();
        assert_eq!(
            first["records"],
            serde_json::json!((20..120).step_by(2).collect::<Vec<_>>())
        );
        assert_eq!(first["next_offset"], 60);
        let last = read_matching_page(0..123, false, |r| r % 2 == 0, |r| r, 60).unwrap();
        assert_eq!(last["records"], serde_json::json!([120, 122]));
        assert!(last["next_offset"].is_null());
    }

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

    #[test]
    fn serialized_record_queries_keep_the_original_casing_and_escape_semantics() {
        let record =
            json!({"id":"case-record", "title":"ΟΣ Café İ K", "controls":"\0A", "literal":"\\n"});
        for query in [
            "Σ",
            "ς",
            "i",
            "İ",
            "k",
            "K",
            "CAFÉ",
            "cafe\u{301}",
            "\\u0000",
            "\0",
            "\\n",
            "TITLE",
        ] {
            let expected = record
                .to_string()
                .to_lowercase()
                .contains(&query.to_lowercase());
            let page = read_records(std::slice::from_ref(&record), query, 0).unwrap();
            let records = page["records"].as_array().unwrap();
            assert_eq!(records.len(), usize::from(expected), "{query:?}");
            if expected {
                assert_eq!(records[0], record);
            }
            assert!(page["next_offset"].is_null());
        }
    }

    #[test]
    fn pages_are_bounded_and_offsets_apply_to_filtered_records() {
        let records: Vec<_> = (0..123)
            .map(|id| json!({"id":id,"title":if id % 2 == 0 {"Needle"} else {"other"}}))
            .collect();
        let page = read_records(&records, "", 50).expect("page");
        assert_eq!(page["records"].as_array().expect("records").len(), 50);
        assert_eq!(page["records"][0]["id"], 50);
        assert_eq!(page["next_offset"], 100);
        let filtered = read_records(&records, "NEEDLE", 50).expect("filtered page");
        assert_eq!(filtered["records"].as_array().expect("records").len(), 12);
        assert_eq!(filtered["records"][0]["id"], 100);
        assert!(filtered["next_offset"].is_null());
        assert!(read_records(&records, &"x".repeat(513), 0).is_err());
        assert!(read_records(&records, "", 100_001).is_err());
    }

    #[test]
    fn complete_json_search_keeps_sorted_keys_nested_text_and_reused_buffer_boundaries() {
        #[derive(serde::Serialize)]
        struct Record {
            z_last: &'static str,
            a_first: &'static str,
            history: Vec<&'static str>,
        }
        let records: Vec<_> = (0..123)
            .map(|index| Record {
                z_last: if index % 2 == 0 { "Needle" } else { "other" },
                a_first: "ΟΣ Café İ K\0\n\"\\",
                history: vec!["Earlier complete nested context", "👩🏽‍💻 e\u{301}"],
            })
            .collect();
        let values: Vec<_> = records
            .iter()
            .map(|record| serde_json::to_value(record).unwrap())
            .collect();
        for query in [
            "needle",
            "NESTED",
            "Σ",
            "ς",
            "i",
            "K",
            "CAFÉ",
            "cafe\u{301}",
            "\\u0000",
            "\0",
            "\\n",
            "👩🏽‍💻",
            "] ,",
            "\"],\"z_last\":",
            "no matching value",
        ] {
            for offset in [0, 1, 49, 50, 51, 99, 123] {
                let mut expected: Vec<_> = values
                    .iter()
                    .filter(|value| {
                        value
                            .to_string()
                            .to_lowercase()
                            .contains(&query.to_lowercase())
                    })
                    .skip(offset)
                    .take(51)
                    .cloned()
                    .collect();
                let more = expected.len() > 50;
                expected.truncate(50);
                assert_eq!(
                    read_records(&records, query, offset).unwrap(),
                    json!({"records":expected,"next_offset":more.then_some(offset + 50)}),
                    "{query:?}, {offset}"
                );
            }
        }
    }

    #[test]
    fn a_large_nonmatching_record_does_not_change_later_matches_or_page_limits() {
        let records = [
            json!({"id":"large", "body":"x".repeat(600_000)}),
            json!({"id":"match", "body":"Needle ΟΣ İ"}),
            json!({"id":"short", "body":"unrelated"}),
        ];
        for query in ["needle", "ς", "İ"] {
            let page = read_records(&records, query, 0).unwrap();
            assert_eq!(page["records"], json!([records[1]]), "{query}");
            assert!(page["next_offset"].is_null());
        }
        assert!(read_records(&records, "", 0).is_err());
        assert_eq!(
            read_records(&records, "", 1).unwrap()["records"],
            json!(records[1..])
        );
    }

    #[test]
    fn serialization_failures_still_abort_scans_but_unread_pages_are_not_encoded() {
        struct Record(usize);
        impl serde::Serialize for Record {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                if self.0 == 99 {
                    return Err(serde::ser::Error::custom("unencodable record"));
                }
                serializer.serialize_u64(u64::try_from(self.0).expect("fixture integer"))
            }
        }
        let records: Vec<_> = (0..123).map(Record).collect();
        assert!(read_records(&records, "", 0).is_ok());
        assert!(read_records(&records, "absent", 0).is_err());
        assert!(read_records(&records, "", 50).is_err());
        assert_eq!(read_records(&records, "", 100).unwrap()["records"][0], 100);
    }
}
