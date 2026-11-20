//! The browser's part of a backup (D7): every IndexedDB store and the
//! `bruhs.*` localStorage keys as JSON, with the values JSON can't carry
//! tagged under [`WEB_TAG`], and the path rewrite a restore runs over it
//! (D19).

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{rewrite_json, rewrite_path, PathRewrite, DAMAGED};
use crate::EpochMs;

/// The key that marks a tagged value.
pub const WEB_TAG: &str = "$bruhs";
/// The newest [`WebDump`] format this build writes and reads.
pub const WEB_DUMP_VERSION: u32 = 1;
/// A browser's dump is at most this many bytes of JSON (ruling F28).
pub const WEB_PLAIN_MAX: u64 = 256 * 1024 * 1024;
/// An uploaded web part is at most this many bytes: the plaintext cap plus
/// 16 bytes per chunk and the header, rounded up (ruling F28).
pub const WEB_UPLOAD_MAX: u64 = 300 * 1024 * 1024;
// 1 MiB chunks, 16 bytes of tag each and the 64-byte header.
const _: () = assert!(WEB_UPLOAD_MAX >= WEB_PLAIN_MAX + WEB_PLAIN_MAX / (1024 * 1024) * 16 + 64);

const BYTES: &str = "bytes";
const BLOB: &str = "blob";
const DATE: &str = "date";
const UNDEFINED: &str = "undefined";
const OBJECT: &str = "object";

/// Everything the browser keeps, as the web part's plaintext.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct WebDump {
    pub version: u32,
    /// The IndexedDB version the records were written under.
    pub db_version: u32,
    /// Each object store's records, by store name.
    #[serde(default)]
    pub stores: BTreeMap<String, Vec<Value>>,
    /// The `bruhs.*` localStorage keys and their values.
    #[serde(default)]
    pub local_storage: BTreeMap<String, String>,
    pub made_at_ms: EpochMs,
}

/// A value JSON can't carry as is.
#[derive(Debug, Clone, PartialEq)]
pub enum WebTag {
    /// An `ArrayBuffer` or typed array.
    Bytes(Vec<u8>),
    Blob {
        mime: String,
        bytes: Vec<u8>,
    },
    /// A `Date`, in milliseconds. An invalid date is NaN.
    Date(f64),
    Undefined,
    /// A plain object that has a [`WEB_TAG`] key of its own, escaped.
    Object(Map<String, Value>),
}

/// The JSON form of `tag`: an object with [`WEB_TAG`] naming the kind.
pub fn tag_value(tag: &WebTag) -> Value {
    match tag {
        WebTag::Bytes(bytes) => json!({ WEB_TAG: BYTES, "b64": STANDARD.encode(bytes) }),
        WebTag::Blob { mime, bytes } => {
            json!({ WEB_TAG: BLOB, "type": mime, "b64": STANDARD.encode(bytes) })
        }
        // JSON has no NaN, so an invalid date is written as null.
        WebTag::Date(ms) if ms.is_finite() => json!({ WEB_TAG: DATE, "ms": ms }),
        WebTag::Date(_) => json!({ WEB_TAG: DATE, "ms": null }),
        WebTag::Undefined => json!({ WEB_TAG: UNDEFINED }),
        WebTag::Object(map) => json!({ WEB_TAG: OBJECT, "value": map }),
    }
}

/// The tag `value` carries: `None` when it isn't a tagged object, and
/// [`DAMAGED`] when it is one with an unknown kind, missing or extra
/// fields, or bad base64.
pub fn read_tag(value: &Value) -> Option<Result<WebTag, &'static str>> {
    let map = value.as_object()?;
    let kind = map.get(WEB_TAG)?;
    Some(parse_tag(kind, map).ok_or(DAMAGED))
}

fn parse_tag(kind: &Value, map: &Map<String, Value>) -> Option<WebTag> {
    let only = |fields: &[&str]| {
        map.len() == fields.len() + 1 && fields.iter().all(|field| map.contains_key(*field))
    };
    let bytes = || STANDARD.decode(map.get("b64")?.as_str()?).ok();
    match kind.as_str()? {
        BYTES if only(&["b64"]) => Some(WebTag::Bytes(bytes()?)),
        BLOB if only(&["type", "b64"]) => Some(WebTag::Blob {
            mime: map.get("type")?.as_str()?.to_owned(),
            bytes: bytes()?,
        }),
        DATE if only(&["ms"]) => match map.get("ms")? {
            Value::Null => Some(WebTag::Date(f64::NAN)),
            ms => ms.as_f64().map(WebTag::Date),
        },
        UNDEFINED if only(&[]) => Some(WebTag::Undefined),
        OBJECT if only(&["value"]) => map.get("value")?.as_object().cloned().map(WebTag::Object),
        _ => None,
    }
}

/// Rewrites paths in every record and localStorage value of `dump` and
/// returns how many changed. Bytes, blobs and dates are left alone; an
/// escaped object's own fields are data and are rewritten. A localStorage
/// value is walked as JSON when it parses as JSON, else rewritten as a
/// whole string.
pub fn rewrite_dump(dump: &mut WebDump, rules: &[PathRewrite]) -> usize {
    if rules.is_empty() {
        return 0;
    }
    let mut changed = 0;
    for record in dump.stores.values_mut().flatten() {
        changed += rewrite_record(record, rules);
    }
    for text in dump.local_storage.values_mut() {
        if let Ok(mut value) = serde_json::from_str::<Value>(text) {
            let count = rewrite_record(&mut value, rules);
            if count > 0 {
                if let Ok(serialized) = serde_json::to_string(&value) {
                    *text = serialized;
                    changed += count;
                }
            }
        } else if let Some(rewritten) = rewrite_path(text, rules) {
            *text = rewritten;
            changed += 1;
        }
    }
    changed
}

/// [`rewrite_json`], except that a tagged value's payload is skipped.
fn rewrite_record(value: &mut Value, rules: &[PathRewrite]) -> usize {
    match value {
        Value::Object(map) if map.contains_key(WEB_TAG) => {
            let escaped = map.get(WEB_TAG).and_then(Value::as_str) == Some(OBJECT);
            match map.get_mut("value") {
                Some(Value::Object(inner)) if escaped => rewrite_fields(inner, rules),
                _ => 0,
            }
        }
        Value::Object(map) => rewrite_fields(map, rules),
        Value::Array(items) => items
            .iter_mut()
            .map(|item| rewrite_record(item, rules))
            .sum(),
        other => rewrite_json(other, rules),
    }
}

/// An object's keys and values, with values walked by [`rewrite_record`].
fn rewrite_fields(map: &mut Map<String, Value>, rules: &[PathRewrite]) -> usize {
    let mut changed = 0;
    for (key, mut item) in std::mem::take(map) {
        changed += rewrite_record(&mut item, rules);
        let key = match rewrite_path(&key, rules) {
            Some(rewritten) => {
                changed += 1;
                rewritten
            }
            None => key,
        };
        map.insert(key, item);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_values_round_trip_through_tags() {
        for tag in [
            WebTag::Bytes(vec![0, 255]),
            WebTag::Blob {
                mime: "image/png".into(),
                bytes: vec![1],
            },
            WebTag::Date(1.5e12),
            WebTag::Undefined,
            WebTag::Object(Map::from_iter([("$bruhs".to_string(), json!("x"))])),
        ] {
            assert_eq!(read_tag(&tag_value(&tag)), Some(Ok(tag)));
        }
        assert_eq!(read_tag(&json!({"a": 1})), None);
        assert!(matches!(
            read_tag(&json!({"$bruhs": "bytes", "b64": "%%"})),
            Some(Err(_))
        ));
        let mut dump = WebDump {
            version: 1,
            db_version: 13,
            ..Default::default()
        };
        dump.stores.insert(
            "projects".into(),
            vec![
                json!({"id": "p", "path": "/home/ada/src"}),
                tag_value(&WebTag::Bytes(b"/home/ada".to_vec())),
            ],
        );
        dump.local_storage
            .insert("bruhs.recentProjects".into(), r#"["/home/ada/src"]"#.into());
        let rules = vec![PathRewrite {
            from: "/home/ada".into(),
            to: "/Users/ada".into(),
        }];
        assert_eq!(rewrite_dump(&mut dump, &rules), 2);
        assert_eq!(
            dump.local_storage["bruhs.recentProjects"],
            r#"["/Users/ada/src"]"#
        );
    }

    #[test]
    fn malformed_tags_are_damaged() {
        for bad in [
            json!({"$bruhs": "map"}),
            json!({"$bruhs": 1}),
            json!({"$bruhs": "bytes"}),
            json!({"$bruhs": "bytes", "b64": "", "extra": 1}),
            json!({"$bruhs": "blob", "b64": ""}),
            json!({"$bruhs": "date", "ms": "soon"}),
            json!({"$bruhs": "undefined", "x": null}),
            json!({"$bruhs": "object", "value": [1]}),
        ] {
            assert_eq!(read_tag(&bad), Some(Err(DAMAGED)), "{bad}");
        }
        match read_tag(&tag_value(&WebTag::Date(f64::NAN))) {
            Some(Ok(WebTag::Date(ms))) => assert!(ms.is_nan()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rewrite_dump_walks_escaped_objects_and_plain_strings() {
        let rules = vec![PathRewrite {
            from: "/home/ada".into(),
            to: "/Users/ada".into(),
        }];
        let mut dump = WebDump::default();
        dump.stores.insert(
            "threads".into(),
            vec![
                tag_value(&WebTag::Object(Map::from_iter([
                    ("$bruhs".to_string(), json!("/home/ada/a")),
                    ("/home/ada/k".to_string(), json!(1)),
                ]))),
                tag_value(&WebTag::Blob {
                    mime: "/home/ada".into(),
                    bytes: vec![],
                }),
            ],
        );
        dump.local_storage
            .insert("bruhs.dir".into(), "/home/ada/notes".into());
        dump.local_storage.insert("bruhs.n".into(), "42".into());
        assert_eq!(rewrite_dump(&mut dump, &rules), 3);
        let Some(Ok(WebTag::Object(map))) = read_tag(&dump.stores["threads"][0]) else {
            panic!("escaped object");
        };
        assert_eq!(map["$bruhs"], "/Users/ada/a");
        assert!(map.contains_key("/Users/ada/k"));
        assert_eq!(
            read_tag(&dump.stores["threads"][1]),
            Some(Ok(WebTag::Blob {
                mime: "/home/ada".into(),
                bytes: vec![]
            }))
        );
        assert_eq!(dump.local_storage["bruhs.dir"], "/Users/ada/notes");
        assert_eq!(dump.local_storage["bruhs.n"], "42");
        assert_eq!(rewrite_dump(&mut dump, &[]), 0);
    }

    #[test]
    fn dumps_round_trip_and_caps_hold() {
        let mut dump = WebDump {
            version: WEB_DUMP_VERSION,
            db_version: 13,
            made_at_ms: EpochMs::new(1_700_000_000_000).unwrap(),
            ..Default::default()
        };
        dump.stores
            .insert("projects".into(), vec![json!({"id": "p"})]);
        let text = serde_json::to_string(&dump).unwrap();
        assert_eq!(serde_json::from_str::<WebDump>(&text).unwrap(), dump);
        assert_eq!(WEB_PLAIN_MAX, 268_435_456);
    }
}
