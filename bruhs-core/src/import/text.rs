//! Text rules every export reader shares (D6): reading the timestamps the
//! services write, stripping the markup their text carries, and cutting a
//! part that is too long.

use bruhs_models::imports::cut_short_line;
use bruhs_models::EpochMs;
use serde_json::Value;

/// Below this, a number is epoch seconds; at or above it, epoch ms (the
/// year 5138 in seconds, early 1973 in ms).
const SECONDS_BELOW: f64 = 100_000_000_000.0;

/// A timestamp in any form an export writes, as epoch ms: epoch seconds
/// (a float or an integer), epoch ms, a numeric string, ISO 8601, or
/// Mongo's `{"$date": {"$numberLong": "<ms>"}}` and `{"$date": "<iso>"}`.
/// `None` for `null`, a negative or non-finite number, and anything else.
pub(crate) fn export_time(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => from_number(number),
        Value::String(text) => from_text(text),
        Value::Object(object) => match object.get("$date")? {
            Value::Object(date) => date
                .get("$numberLong")
                .and_then(Value::as_str)
                .and_then(|ms| ms.trim().parse::<i64>().ok())
                .filter(|ms| *ms >= 0)
                .map(|ms| EpochMs::saturating_from_i64(ms).as_millis()),
            date @ (Value::Number(_) | Value::String(_)) => export_time(date),
            _ => None,
        },
        _ => None,
    }
}

fn from_number(number: &serde_json::Number) -> Option<i64> {
    if let Some(whole) = number.as_i64() {
        return from_whole(whole);
    }
    if let Some(whole) = number.as_u64() {
        return Some(
            EpochMs::saturating_from_i64(i64::try_from(whole).unwrap_or(i64::MAX)).as_millis(),
        );
    }
    from_float(number.as_f64()?)
}

fn from_whole(whole: i64) -> Option<i64> {
    if whole < 0 {
        return None;
    }
    let ms = if whole < 100_000_000_000 {
        whole.saturating_mul(1000)
    } else {
        whole
    };
    Some(EpochMs::saturating_from_i64(ms).as_millis())
}

fn from_float(value: f64) -> Option<i64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let ms = if value < SECONDS_BELOW {
        (value * 1000.0).round()
    } else {
        value
    };
    Some(EpochMs::saturating_from_f64(ms).as_millis())
}

fn from_text(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(whole) = text.parse::<i64>() {
        return from_whole(whole);
    }
    if text.starts_with(|c: char| c.is_ascii_digit()) {
        if let Ok(float) = text.parse::<f64>() {
            return from_float(float);
        }
    }
    if let Some(ms) = super::parse_time(text) {
        return Some(EpochMs::saturating_from_i64(ms).as_millis());
    }
    // ISO 8601 without an offset reads as UTC.
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"]
        .iter()
        .find_map(|format| chrono::NaiveDateTime::parse_from_str(text, format).ok())
        .map(|time| EpochMs::saturating_from_i64(time.and_utc().timestamp_millis()).as_millis())
}

/// `text` without ChatGPT's citation markers: the private-use spans
/// `U+E200 … U+E201` and the older `【…†…】`. A private-use marker that
/// never closes loses only its own character, so no text after it is lost.
pub(crate) fn strip_citations(text: &str) -> String {
    if !text.contains(|c| matches!(c, '\u{e200}'..='\u{e202}' | '【')) {
        return text.to_string();
    }
    let text = without_spans(text, '\u{e200}', '\u{e201}', |_| true);
    let text: String = text
        .chars()
        .filter(|c| !matches!(c, '\u{e200}'..='\u{e202}'))
        .collect();
    without_spans(&text, '【', '】', |inner| inner.contains('†'))
}

/// `text` without each `open … close` span whose inner text passes `drop`.
fn without_spans(text: &str, open: char, close: char, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len_utf8()..];
        let Some(end) = after.find(close) else {
            break;
        };
        out.push_str(&rest[..start]);
        let inner = &after[..end];
        if !drop(inner) {
            out.push(open);
            out.push_str(inner);
            out.push(close);
        }
        rest = &after[end + close.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// `text` without the opener and closer lines of ChatGPT's Canvas
/// directives (`:::writing{…}` … `:::`); the text between them stays. A
/// `:::` line outside a directive stays too.
pub(crate) fn strip_writing_directives(text: &str) -> String {
    if !text.contains(":::writing{") {
        return text.to_string();
    }
    let mut open = false;
    let mut kept: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with(":::writing{") {
            open = true;
            continue;
        }
        if open && trimmed == ":::" {
            open = false;
            continue;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// `text` without Grok's `<grok:render …>…</grok:render>` blocks (removed
/// whole) and its `<xaiArtifact …>` tags (their inner text stays). A
/// render block that never closes loses only its opening tag.
pub(crate) fn strip_grok_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<grok:render") {
        out.push_str(&rest[..start]);
        let block = &rest[start..];
        let Some(tag_end) = block.find('>') else {
            rest = "";
            break;
        };
        if block[..tag_end].ends_with('/') {
            rest = &block[tag_end + 1..];
            continue;
        }
        rest = match block.find("</grok:render>") {
            Some(end) => &block[end + "</grok:render>".len()..],
            None => &block[tag_end + 1..],
        };
    }
    out.push_str(rest);
    without_tags(&out, "xaiArtifact")
}

/// `text` without `<tag …>` and `</tag>` tags, keeping what they wrap.
fn without_tags(text: &str, tag: &str) -> String {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next_open = rest.find(&open).filter(|&at| {
            rest[at + open.len()..]
                .chars()
                .next()
                .is_some_and(|c| c == '>' || c == '/' || c.is_whitespace())
        });
        let next_close = rest.find(&close);
        let (at, len) = match (next_open, next_close) {
            (Some(o), Some(c)) if c < o => (c, close.len()),
            (Some(o), _) => match rest[o..].find('>') {
                Some(end) => (o, end + 1),
                None => break,
            },
            (None, Some(c)) => (c, close.len()),
            (None, None) => break,
        };
        out.push_str(&rest[..at]);
        rest = &rest[at + len..];
    }
    out.push_str(rest);
    out
}

/// `text` cut to `limit` characters, at a character boundary, ending with
/// `cut_short_line` for what was left out.
pub(crate) fn clip_part(text: String, limit: usize) -> String {
    let Some((cut, _)) = text.char_indices().nth(limit) else {
        return text;
    };
    let more = text[cut..].chars().count();
    let mut out = text;
    out.truncate(cut);
    out.push('\n');
    out.push_str(&cut_short_line(more));
    out
}
