//! Streaming the records out of one export JSON file within fixed
//! budgets.
//!
//! A ChatGPT `conversations.json` can be hundreds of MB on one line, so a
//! file is never held whole: a top-level array yields one record at a
//! time, and a top-level object yields its `conversations` array's
//! elements while every other value is skipped unheld. Only manifests,
//! memory files and small bare objects are read whole, within
//! `small_json`.
//!
//! The bytes reach serde through two filters:
//! - [`LoneSurrogates`] fixes what real exports get wrong: unpaired
//!   UTF-16 surrogate escapes become `�`, raw control bytes inside
//!   strings become escapes, and a leading UTF-8 byte order mark is
//!   dropped. It also counts bracket depth, so a value serde skips
//!   without counting can't nest past [`MAX_DEPTH`].
//! - `RecordBudget` counts the bytes serde reads against the current
//!   record's budget and the file's total.
//!
//! A refusal's copy travels through a shared slot (F11): serde's error
//! doesn't keep the I/O error that carried it.

use super::archive::{EntryKind, Limits};
use super::ImportError;
use bruhs_models::imports::{CONVERSATION_TOO_LARGE, DAMAGED_EXPORT};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::cell::Cell;
use std::fmt;
use std::io::{self, BufReader, Read};
use std::rc::Rc;

/// Where a reader leaves the fixed copy of the refusal that stopped it.
pub(crate) type Refusal = Rc<Cell<Option<&'static str>>>;

/// Brackets open at once past which a file is damage. serde_json's own
/// limit for the values it builds is 128 as well.
pub(crate) const MAX_DEPTH: u32 = 128;

/// Input bytes a filter holds while it decides: `😀` is 12.
const LOOKAHEAD: usize = 12;
const CHUNK: usize = 8 * 1024;
const REPLACEMENT: &[u8; 6] = b"\\uFFFD";
const BOM: &[u8; 3] = b"\xEF\xBB\xBF";

/// The refusal error a reader returns after filling its slot.
pub(crate) fn refuse(slot: &Refusal, copy: &'static str) -> io::Error {
    if slot.get().is_none() {
        slot.set(Some(copy));
    }
    io::Error::new(io::ErrorKind::InvalidData, copy)
}

/// Rewrites a JSON byte stream so one bad string doesn't fail the whole
/// file:
/// - an unpaired `\uD800` to `\uDFFF` escape becomes `�` (a pair
///   and an escaped backslash pass unchanged);
/// - a raw control byte inside a string becomes its `\u00XX` escape;
/// - a leading UTF-8 byte order mark is dropped.
///
/// It holds at most [`LOOKAHEAD`] input bytes while it decides. Opening
/// brackets past [`MAX_DEPTH`] are refused as damage.
pub(crate) struct LoneSurrogates<R: Read> {
    inner: R,
    buf: Box<[u8]>,
    start: usize,
    end: usize,
    eof: bool,
    out: Vec<u8>,
    out_pos: usize,
    at_start: bool,
    in_string: bool,
    depth: u32,
    refusal: Refusal,
}

impl<R: Read> LoneSurrogates<R> {
    pub(crate) fn new(inner: R, refusal: Refusal) -> Self {
        Self {
            inner,
            buf: vec![0; CHUNK].into_boxed_slice(),
            start: 0,
            end: 0,
            eof: false,
            out: Vec::with_capacity(CHUNK),
            out_pos: 0,
            at_start: true,
            in_string: false,
            depth: 0,
            refusal,
        }
    }

    /// Input bytes available, reading until there are `want` or the
    /// input ends.
    fn ensure(&mut self, want: usize) -> io::Result<usize> {
        while self.end - self.start < want && !self.eof {
            if self.start > 0 {
                self.buf.copy_within(self.start..self.end, 0);
                self.end -= self.start;
                self.start = 0;
            }
            match self.inner.read(&mut self.buf[self.end..]) {
                Ok(0) => self.eof = true,
                Ok(n) => self.end += n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(self.end - self.start)
    }

    /// Fills `out` with at least one byte, unless the input has ended.
    fn produce(&mut self) -> io::Result<()> {
        self.out.clear();
        self.out_pos = 0;
        while self.out.len() < CHUNK {
            if self.ensure(1)? == 0 {
                return Ok(());
            }
            if self.at_start {
                self.at_start = false;
                if self.ensure(BOM.len())? >= BOM.len()
                    && self.buf[self.start..self.start + BOM.len()] == BOM[..]
                {
                    self.start += BOM.len();
                }
                continue;
            }
            let byte = self.buf[self.start];
            if !self.in_string {
                match byte {
                    b'"' => self.in_string = true,
                    b'[' | b'{' => {
                        self.depth += 1;
                        if self.depth > MAX_DEPTH {
                            return Err(refuse(&self.refusal, DAMAGED_EXPORT));
                        }
                    }
                    b']' | b'}' => self.depth = self.depth.saturating_sub(1),
                    _ => {}
                }
                self.out.push(byte);
                self.start += 1;
            } else if byte == b'"' {
                self.in_string = false;
                self.out.push(byte);
                self.start += 1;
            } else if byte == b'\\' {
                self.escape()?;
            } else if byte < 0x20 {
                self.out.extend_from_slice(b"\\u00");
                self.out.push(hex_digit(byte >> 4));
                self.out.push(hex_digit(byte & 0x0F));
                self.start += 1;
            } else {
                self.out.push(byte);
                self.start += 1;
            }
            if !self.out.is_empty() && self.end == self.start && !self.eof {
                // Hand out what is ready before blocking on more input.
                return Ok(());
            }
        }
        Ok(())
    }

    /// One escape inside a string, starting at its backslash.
    fn escape(&mut self) -> io::Result<()> {
        if self.ensure(2)? < 2 {
            // Cut short: serde reports it.
            self.out.push(b'\\');
            self.start += 1;
            return Ok(());
        }
        let at = self.start;
        if self.buf[at + 1] != b'u' {
            self.out.extend_from_slice(&self.buf[at..at + 2]);
            self.start += 2;
            return Ok(());
        }
        let code = if self.ensure(6)? >= 6 {
            let at = self.start;
            hex4(&self.buf[at + 2..at + 6])
        } else {
            None
        };
        let Some(code) = code else {
            // Not an escape serde accepts either: passed on for serde to
            // refuse.
            self.out.extend_from_slice(b"\\u");
            self.start += 2;
            return Ok(());
        };
        let width = match code {
            0xD800..=0xDBFF => {
                let paired = self.ensure(LOOKAHEAD)? >= LOOKAHEAD && {
                    let at = self.start;
                    self.buf[at + 6] == b'\\'
                        && self.buf[at + 7] == b'u'
                        && hex4(&self.buf[at + 8..at + 12])
                            .is_some_and(|low| (0xDC00..=0xDFFF).contains(&low))
                };
                if paired {
                    LOOKAHEAD
                } else {
                    self.out.extend_from_slice(REPLACEMENT);
                    self.start += 6;
                    return Ok(());
                }
            }
            0xDC00..=0xDFFF => {
                self.out.extend_from_slice(REPLACEMENT);
                self.start += 6;
                return Ok(());
            }
            _ => 6,
        };
        let at = self.start;
        self.out.extend_from_slice(&self.buf[at..at + width]);
        self.start += width;
        Ok(())
    }
}

impl<R: Read> Read for LoneSurrogates<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.out_pos == self.out.len() {
            self.produce()?;
        }
        let ready = &self.out[self.out_pos..];
        let n = ready.len().min(buf.len());
        buf[..n].copy_from_slice(&ready[..n]);
        self.out_pos += n;
        Ok(n)
    }
}

fn hex_digit(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'A' + (nibble - 10),
    }
}

fn hex4(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0_u32, |code, digit| {
        let value = char::from(*digit).to_digit(16)?;
        Some(code * 16 + value)
    })
}

/// A budget: bytes, and values (counted at each `[`, `{` and `,` outside
/// strings), refused with fixed copy.
#[derive(Clone, Copy)]
struct Budget {
    bytes: u64,
    values: u64,
    copy: &'static str,
}

/// The budget the current record counts against, shared with the visitor
/// that resets it before each record.
#[derive(Clone, Default)]
struct BudgetHandle {
    used: Rc<Cell<(u64, u64)>>,
    limit: Rc<Cell<Option<Budget>>>,
}

impl BudgetHandle {
    /// From here, at most `bytes` bytes and `values` values, refused with
    /// `copy`.
    fn set(&self, bytes: u64, values: u64, copy: &'static str) {
        self.used.set((0, 0));
        self.limit.set(Some(Budget {
            bytes,
            values,
            copy,
        }));
    }

    /// From here, only the file's total counts.
    fn lift(&self) {
        self.used.set((0, 0));
        self.limit.set(None);
    }

    /// What is left of the current record's budget.
    fn left(&self) -> Option<Budget> {
        let (bytes, values) = self.used.get();
        self.limit.get().map(|limit| Budget {
            bytes: limit.bytes.saturating_sub(bytes),
            values: limit.values.saturating_sub(values),
            copy: limit.copy,
        })
    }
}

/// Counts what serde reads against the current record's budget and the
/// file's total: bytes, and values, since a dense record (`[0,0,…]`)
/// builds a `Value` many times its size (I Task 3 I1). It hands out no
/// byte past either byte budget.
struct RecordBudget<R> {
    inner: R,
    record: BudgetHandle,
    total: Option<Budget>,
    refusal: Refusal,
    in_string: bool,
    escaped: bool,
}

impl<R: Read> RecordBudget<R> {
    /// Values opened in `bytes`, keeping track of strings across reads.
    fn values_in(&mut self, bytes: &[u8]) -> u64 {
        let mut values = 0;
        for &byte in bytes {
            if self.escaped {
                self.escaped = false;
            } else if self.in_string {
                match byte {
                    b'\\' => self.escaped = true,
                    b'"' => self.in_string = false,
                    _ => {}
                }
            } else {
                match byte {
                    b'"' => self.in_string = true,
                    b'[' | b'{' | b',' => values += 1,
                    _ => {}
                }
            }
        }
        values
    }
}

impl<R: Read> Read for RecordBudget<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let budgets = [self.record.left(), self.total];
        let allowed = budgets
            .into_iter()
            .flatten()
            .map(|budget| budget.bytes)
            .min()
            .unwrap_or(u64::MAX);
        let want =
            usize::try_from(allowed.saturating_add(1)).map_or(buf.len(), |cap| cap.min(buf.len()));
        let n = self.inner.read(&mut buf[..want])?;
        let got = u64::try_from(n).map_err(io::Error::other)?;
        let values = self.values_in(&buf[..n]);
        for budget in budgets.into_iter().flatten() {
            if got > budget.bytes || values > budget.values {
                return Err(refuse(&self.refusal, budget.copy));
            }
        }
        let (bytes_used, values_used) = self.record.used.get();
        self.record.used.set((
            bytes_used.saturating_add(got),
            values_used.saturating_add(values),
        ));
        if let Some(total) = &mut self.total {
            total.bytes -= got;
            total.values -= values;
        }
        Ok(n)
    }
}

/// Calls `f` once per record in one export JSON file.
///
/// - A top-level array gives each element, each within `record` bytes
///   and `values` values (`CONVERSATION_TOO_LARGE`).
/// - A top-level object gives its `conversations` array's elements and
///   skips every other key unheld. A [`EntryKind::Manifest`] or
///   [`EntryKind::Memory`] object, or a [`EntryKind::Bare`] file of at most
///   `small_json` bytes (`bare_len`), is instead given once, whole.
/// - Manifests, memory files and small bare files are read within
///   `small_json` bytes and `values` values in all (`DAMAGED_EXPORT`).
///
/// `f` checks for cancelling between records: its error ends the stream
/// and is returned as it is.
///
/// # Errors
///
/// `f`'s error, a budget's refusal, or `Refused(DAMAGED_EXPORT)` for
/// anything serde can't read (nesting past 128 included). An I/O error
/// from `reader` is damage too: the caller checks its own refusal slot
/// first.
pub(crate) fn for_each_record(
    reader: impl Read,
    kind: EntryKind,
    bare_len: Option<u64>,
    limits: &Limits,
    f: &mut dyn FnMut(Value) -> Result<(), ImportError>,
) -> Result<(), ImportError> {
    let small_bare =
        kind == EntryKind::Bare && bare_len.is_some_and(|len| len <= limits.small_json);
    let small = matches!(kind, EntryKind::Manifest | EntryKind::Memory) || small_bare;
    let refusal = Refusal::default();
    let record = BudgetHandle::default();
    let budget = RecordBudget {
        inner: BufReader::new(LoneSurrogates::new(reader, refusal.clone())),
        record: record.clone(),
        total: small.then_some(Budget {
            bytes: limits.small_json,
            values: limits.values,
            copy: DAMAGED_EXPORT,
        }),
        refusal: refusal.clone(),
        in_string: false,
        escaped: false,
    };
    let failed = Cell::new(None);
    let top = Top {
        f,
        record: &record,
        limits,
        whole_object: small,
        failed: &failed,
    };
    let mut de = serde_json::Deserializer::from_reader(budget);
    let result = de::Deserializer::deserialize_any(&mut de, top).and_then(|()| {
        record.lift();
        de.end()
    });
    match result {
        Ok(()) => Ok(()),
        Err(_) => Err(failed
            .take()
            .or_else(|| refusal.get().map(ImportError::Refused))
            .unwrap_or(ImportError::Refused(DAMAGED_EXPORT))),
    }
}

/// The top-level value: an array of records or an object.
struct Top<'a> {
    f: &'a mut dyn FnMut(Value) -> Result<(), ImportError>,
    record: &'a BudgetHandle,
    limits: &'a Limits,
    whole_object: bool,
    failed: &'a Cell<Option<ImportError>>,
}

impl Top<'_> {
    fn give<E: de::Error>(&mut self, value: Value) -> Result<(), E> {
        (self.f)(value).map_err(|error| {
            self.failed.set(Some(error));
            E::custom("the record visitor stopped")
        })
    }

    fn records<'de, A: SeqAccess<'de>>(&mut self, mut seq: A) -> Result<(), A::Error> {
        loop {
            self.record.set(
                self.limits.record,
                self.limits.values,
                CONVERSATION_TOO_LARGE,
            );
            let Some(value) = seq.next_element::<Value>()? else {
                return Ok(());
            };
            self.record.lift();
            self.give(value)?;
        }
    }
}

impl<'de> Visitor<'de> for Top<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array or object")
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, seq: A) -> Result<(), A::Error> {
        self.records(seq)
    }

    fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<(), A::Error> {
        if self.whole_object {
            let mut object = serde_json::Map::new();
            while let Some(key) = map.next_key::<String>()? {
                let value = map.next_value::<Value>()?;
                object.insert(key, value);
            }
            return self.give(Value::Object(object));
        }
        loop {
            // A key is held while it is compared, so it has a budget too.
            self.record
                .set(self.limits.record, self.limits.values, DAMAGED_EXPORT);
            let Some(key) = map.next_key::<String>()? else {
                return Ok(());
            };
            self.record.lift();
            if key == "conversations" {
                map.next_value_seed(Conversations(&mut self))?;
            } else {
                // Skipped unheld: serde reads past it without keeping it.
                map.next_value::<IgnoredAny>()?;
            }
        }
    }
}

/// The `conversations` array of a top-level object, streamed.
struct Conversations<'a, 'b>(&'a mut Top<'b>);

impl<'de> DeserializeSeed<'de> for Conversations<'_, '_> {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Conversations<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of conversations")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<(), A::Error> {
        self.0.records(seq)
    }
}
