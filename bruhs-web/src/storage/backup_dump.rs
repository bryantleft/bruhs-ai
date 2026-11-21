//! The browser's own part of a backup (D7, D19, rulings F27 and F28): every
//! IndexedDB store and the `bruhs.*` localStorage keys as JSON, with the
//! values JSON can't carry tagged under `$bruhs`, locked in the browser with
//! the backup password (kind 2, `KdfParams::WEB`) and put back in one
//! readwrite transaction.
//!
//! Only ciphertext leaves the page. The password is held in `Zeroizing` for
//! the call and is never logged; the random bytes come from
//! `crypto.getRandomValues` and nothing stands in for it.
#![expect(
    dead_code,
    reason = "used from Task 14: Settings, Backup calls the browser's part"
)]

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufReader, BufWriter, Write};

use bruhs_backup::{
    Decryptor, Encryptor, FormatError, Header, Kdf, KdfParams, Key, Kind, Secret, CHUNK_BYTES,
    HEADER_LEN,
};
use bruhs_models::backup::{
    normalize_password, read_tag, tag_value, WebDump, WebTag, DAMAGED, NEWER_BACKUP,
    NEWER_WEB_DATA, NOT_A_BACKUP, OLDER_WEB_DATA, WEB_DATA_MIGRATIONS, WEB_DUMP_VERSION,
    WEB_LOCK_FAILED, WEB_PLAIN_MAX, WEB_READ_FAILED, WEB_TAG, WEB_TOO_LARGE, WEB_UNSUPPORTED,
    WEB_UPLOAD_MAX, WEB_WRITE_FAILED, WRONG_PASSWORD,
};
use bruhs_models::num::ToIntSaturating;
use idb::{Database, TransactionMode};
use js_sys::{Array, ArrayBuffer, Object, Reflect, Uint8Array};
use serde_json::{Map, Number, Value};
use wasm_bindgen::{JsCast, JsValue};
use zeroize::Zeroizing;

use super::metadata::{self, LocalTransaction, SaveError};
use super::{open_db, DB_VERSION};

/// The most containers one record may nest, counting the maps tags add.
/// The dump wraps each record in three more, and the reader takes 127.
const MAX_DEPTH: usize = 120;
/// AES-GCM tag bytes after every chunk.
const TAG_BYTES: usize = 16;
/// Plaintext is read and written through buffers this size.
const BUFFER_BYTES: usize = 64 * 1024;
/// JavaScript's largest exact integer, 2^53 - 1.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// What [`restore_web`] put back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebRestoreReport {
    pub records: u64,
    /// Stores the dump has that this build doesn't, so it left them out.
    pub skipped_stores: Vec<String>,
}

// ── Dump ─────────────────────────────────────────────────────────────────────

/// Every IndexedDB store this database has, read in one readonly
/// transaction, and the `bruhs.*` localStorage keys. A value JSON can't
/// carry fails with `WEB_UNSUPPORTED` naming its store. Records are turned
/// into JSON one store at a time, so the browser's own copies of a store go
/// once it is done (ruling F28).
pub async fn dump_web() -> Result<WebDump, String> {
    let read_failed = |_| WEB_READ_FAILED.to_owned();
    let local_storage = read_local_storage().ok_or(WEB_READ_FAILED)?;
    let db = open_db().await.map_err(|_| WEB_READ_FAILED.to_owned())?;
    let names = db.store_names();
    let raw = read_stores(&db, &names).await.map_err(read_failed)?;
    let mut stores = BTreeMap::new();
    // One estimate for the whole dump, seeded with localStorage (I1).
    let mut estimate = Estimate::default();
    for (key, text) in &local_storage {
        let len = key.len().saturating_add(text.len());
        estimate
            .add(u64::try_from(len).unwrap_or(u64::MAX))
            .map_err(|stop| stop_copy(stop, "localStorage"))?;
    }
    for (name, values) in names.into_iter().zip(raw) {
        let stop = |stop| stop_copy(stop, &name);
        let mut records = Vec::with_capacity(values.len());
        for value in values {
            let mut found = Vec::new();
            find_blobs(&value, 0, &mut found).map_err(|()| unsupported(&name))?;
            for blob in &found {
                estimate
                    .add_binary(blob.size().to_u64_saturating())
                    .map_err(stop)?;
            }
            let mut read = read_blobs(found)
                .await
                .map_err(|()| WEB_READ_FAILED.to_owned())?;
            records.push(convert(&value, 0, &mut read, &mut estimate).map_err(stop)?);
            // Every blob read was placed (B Task 13 M1).
            if !read.is_empty() {
                return Err(unsupported(&name));
            }
        }
        stores.insert(name, records);
    }
    Ok(WebDump {
        version: WEB_DUMP_VERSION,
        db_version: DB_VERSION,
        stores,
        local_storage,
        made_at_ms: crate::clock::now(),
    })
}

async fn read_stores(db: &Database, names: &[String]) -> Result<Vec<Vec<JsValue>>, SaveError> {
    // A transaction over no stores is an error in IndexedDB.
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let tx = LocalTransaction::new(db.transaction(names, TransactionMode::ReadOnly)?);
    let result = async {
        let mut all = Vec::with_capacity(names.len());
        for name in names {
            all.push(tx.object_store(name)?.get_all(None, None)?.await?);
        }
        Ok(all)
    }
    .await;
    metadata::finish(tx, result).await
}

/// One IndexedDB value as JSON, with bytes, blobs, dates, `undefined` and
/// plain objects that have a `$bruhs` key of their own tagged (D7). Any
/// other value (a `Map`, a `BigInt`, a class instance, a number JSON can't
/// hold, or nesting past [`MAX_DEPTH`]) is `Err`. It is async because a
/// `Blob`'s bytes can only be read that way.
pub async fn js_to_json(value: &JsValue) -> Result<Value, ()> {
    let mut found = Vec::new();
    find_blobs(value, 0, &mut found)?;
    let mut estimate = Estimate::default();
    for blob in &found {
        estimate
            .add_binary(blob.size().to_u64_saturating())
            .map_err(|_| ())?;
    }
    let mut read = read_blobs(found).await?;
    let json = convert(value, 0, &mut read, &mut estimate).map_err(|_| ())?;
    if read.is_empty() {
        Ok(json)
    } else {
        Err(())
    }
}

/// Every `Blob` in `value`, in the order [`convert`] meets them.
fn find_blobs(value: &JsValue, depth: usize, found: &mut Vec<web_sys::Blob>) -> Result<(), ()> {
    if !value.is_object() {
        return Ok(());
    }
    if depth >= MAX_DEPTH {
        return Err(());
    }
    if let Some(blob) = value.dyn_ref::<web_sys::Blob>() {
        found.push(blob.clone());
    } else if Array::is_array(value) {
        for item in value.unchecked_ref::<Array>().iter() {
            find_blobs(&item, depth + 1, found)?;
        }
    } else if is_plain_object(value) {
        for key in Object::keys(value.unchecked_ref::<Object>()).iter() {
            let item = Reflect::get(value, &key).map_err(|_| ())?;
            find_blobs(&item, depth + 1, found)?;
        }
    }
    Ok(())
}

async fn read_blobs(blobs: Vec<web_sys::Blob>) -> Result<VecDeque<(web_sys::Blob, Vec<u8>)>, ()> {
    let mut read = VecDeque::with_capacity(blobs.len());
    for blob in blobs {
        let buffer = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
            .await
            .map_err(|_| ())?;
        let bytes = Uint8Array::new(&buffer).to_vec();
        read.push_back((blob, bytes));
    }
    Ok(read)
}

/// Why a value didn't make it into the dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// A value JSON can't carry: `WEB_UNSUPPORTED`.
    Unsupported,
    /// The dump's running estimate passed [`WEB_PLAIN_MAX`]: `WEB_TOO_LARGE`.
    TooLarge,
}

fn stop_copy(stop: Stop, store: &str) -> String {
    match stop {
        Stop::Unsupported => unsupported(store),
        Stop::TooLarge => WEB_TOO_LARGE.to_owned(),
    }
}

/// A running estimate of the dump's JSON, one across every store (B Task 13
/// I1). Each string and each byte copy is added before it is copied into
/// wasm memory, bytes as their base64 (4 for every 3), so a profile far
/// over the cap stops before it fills the tab's memory. `lock_dump`
/// measures the exact size later.
#[derive(Debug, Default)]
struct Estimate {
    bytes: u64,
}

impl Estimate {
    fn add(&mut self, bytes: u64) -> Result<(), Stop> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > WEB_PLAIN_MAX {
            Err(Stop::TooLarge)
        } else {
            Ok(())
        }
    }

    fn add_binary(&mut self, bytes: u64) -> Result<(), Stop> {
        self.add(bytes.div_ceil(3).saturating_mul(4))
    }

    /// A JS string's length in UTF-16 units, read without copying it.
    fn add_text(&mut self, text: &JsValue) -> Result<(), Stop> {
        self.add(u64::from(text.unchecked_ref::<js_sys::JsString>().length()))
    }
}

/// [`js_to_json`] with the blobs' bytes already read, in walk order. Blob
/// bytes were added to `estimate` before they were read.
fn convert(
    value: &JsValue,
    depth: usize,
    blobs: &mut VecDeque<(web_sys::Blob, Vec<u8>)>,
    estimate: &mut Estimate,
) -> Result<Value, Stop> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(Value::Bool(flag));
    }
    if let Some(number) = value.as_f64() {
        return match number_value(number) {
            Some(json) => Ok(json),
            // NaN and the infinities need a tag map.
            None if depth < MAX_DEPTH => Ok(tag_value(&WebTag::Number(number))),
            None => Err(Stop::Unsupported),
        };
    }
    if value.is_string() {
        estimate.add_text(value)?;
        return value
            .as_string()
            .map(Value::String)
            .ok_or(Stop::Unsupported);
    }
    // Every other answer is a container: a tag map or the value's own.
    if depth >= MAX_DEPTH {
        return Err(Stop::Unsupported);
    }
    if value.is_undefined() {
        return Ok(tag_value(&WebTag::Undefined));
    }
    // A BigInt, a symbol or a function.
    if !value.is_object() || value.is_function() {
        return Err(Stop::Unsupported);
    }
    if Array::is_array(value) {
        return value
            .unchecked_ref::<Array>()
            .iter()
            .map(|item| convert(&item, depth + 1, blobs, estimate))
            .collect::<Result<_, _>>()
            .map(Value::Array);
    }
    if let Some(date) = value.dyn_ref::<js_sys::Date>() {
        return Ok(tag_value(&WebTag::Date(date.get_time())));
    }
    if let Some(buffer) = value.dyn_ref::<ArrayBuffer>() {
        estimate.add_binary(u64::from(buffer.byte_length()))?;
        return Ok(tag_value(&WebTag::Bytes(Uint8Array::new(value).to_vec())));
    }
    if ArrayBuffer::is_view(value) {
        return view_bytes(value, estimate).map(|bytes| tag_value(&WebTag::Bytes(bytes)));
    }
    if let Some(blob) = value.dyn_ref::<web_sys::Blob>() {
        let (found, bytes) = blobs.pop_front().ok_or(Stop::Unsupported)?;
        if !Object::is(found.as_ref(), value) {
            return Err(Stop::Unsupported);
        }
        return Ok(tag_value(&WebTag::Blob {
            mime: blob.type_(),
            bytes,
        }));
    }
    if !is_plain_object(value) {
        return Err(Stop::Unsupported);
    }
    let keys = Object::keys(value.unchecked_ref::<Object>());
    let escaped = keys
        .iter()
        .any(|key| key.as_string().as_deref() == Some(WEB_TAG));
    // An escaped object's fields sit inside its tag map's `value` map.
    let inner = if escaped { depth + 2 } else { depth + 1 };
    if escaped && depth + 1 >= MAX_DEPTH {
        return Err(Stop::Unsupported);
    }
    let mut map = Map::new();
    for key in keys.iter() {
        estimate.add_text(&key)?;
        let name = key.as_string().ok_or(Stop::Unsupported)?;
        let item = Reflect::get(value, &key).map_err(|_| Stop::Unsupported)?;
        map.insert(name, convert(&item, inner, blobs, estimate)?);
    }
    Ok(if escaped {
        tag_value(&WebTag::Object(map))
    } else {
        Value::Object(map)
    })
}

/// The bytes a typed array or `DataView` covers, not its whole buffer,
/// added to `estimate` before they are copied.
fn view_bytes(value: &JsValue, estimate: &mut Estimate) -> Result<Vec<u8>, Stop> {
    let field = |name: &str| {
        Reflect::get(value, &JsValue::from_str(name))
            .ok()
            .and_then(|field| field.as_f64())
            .ok_or(Stop::Unsupported)
    };
    let offset = field("byteOffset")?;
    let length = field("byteLength")?;
    estimate.add_binary(length.to_u64_saturating())?;
    let buffer =
        Reflect::get(value, &JsValue::from_str("buffer")).map_err(|_| Stop::Unsupported)?;
    let view = Uint8Array::new_with_byte_offset_and_length(
        &buffer,
        offset.to_u32_saturating(),
        length.to_u32_saturating(),
    );
    Ok(view.to_vec())
}

/// An object whose prototype is `Object.prototype` or null: what IndexedDB
/// gives back for the app's own records.
fn is_plain_object(value: &JsValue) -> bool {
    if !value.is_object() || value.is_function() {
        return false;
    }
    let prototype = Object::get_prototype_of(value);
    prototype.is_null() || Object::is(&prototype, &Object::get_prototype_of(&Object::new()))
}

/// A JS number as JSON: whole numbers JS holds exactly as integers, so they
/// read back as the integers they were written as; `None` for NaN and the
/// infinities, which JSON has no number for (they are tagged instead).
fn number_value(number: f64) -> Option<Value> {
    let whole = number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER;
    // -0 is whole but has no integer form.
    if whole && !(number == 0.0 && number.is_sign_negative()) {
        return Some(Value::from(number.to_i64_saturating()));
    }
    Number::from_f64(number).map(Value::Number)
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Every key currently in localStorage. A key another tab removes while
/// this reads is left out.
fn local_keys(storage: &web_sys::Storage) -> Option<Vec<String>> {
    let length = storage.length().ok()?;
    Some(
        (0..length)
            .filter_map(|index| storage.key(index).ok().flatten())
            .collect(),
    )
}

fn read_local_storage() -> Option<BTreeMap<String, String>> {
    let storage = local_storage()?;
    let mut pairs = Vec::new();
    for key in local_keys(&storage)? {
        if is_bruhs_key(&key) {
            if let Some(value) = storage.get_item(&key).ok()? {
                pairs.push((key, value));
            }
        }
    }
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    Some(select_local_keys(&pairs))
}

fn is_bruhs_key(key: &str) -> bool {
    key.starts_with("bruhs.")
}

/// The pairs a dump takes from localStorage: `bruhs.*` keys only.
pub fn select_local_keys(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .filter(|(key, _)| is_bruhs_key(key))
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// `WEB_UNSUPPORTED` naming the store.
fn unsupported(store: &str) -> String {
    WEB_UNSUPPORTED.replace("{store}", store)
}

// ── Lock and unlock ──────────────────────────────────────────────────────────

/// The dump as a web part: its JSON encrypted with `password` (kind 2,
/// `KdfParams::WEB`, fresh random salt and nonce). A dump over
/// [`WEB_PLAIN_MAX`] is `WEB_TOO_LARGE`, found before any key is derived.
pub fn lock_dump(dump: &WebDump, password: &str) -> Result<Vec<u8>, &'static str> {
    lock_with(dump, password, fresh_random)
}

/// [`lock_dump`] with the random bytes from `random`, asked for only once
/// the size is known to fit.
fn lock_with(
    dump: &WebDump,
    password: &str,
    random: impl FnOnce() -> Result<[u8; 23], &'static str>,
) -> Result<Vec<u8>, &'static str> {
    let plain = plain_len(dump)?;
    let password = Zeroizing::new(normalize_password(password));
    let header = Header::new(Kind::WebPart, Kdf::Argon2id(KdfParams::WEB), random()?);
    let key = Key::derive(&password, &header).map_err(|_| WEB_LOCK_FAILED)?;
    let chunk = usize::try_from(CHUNK_BYTES).map_err(|_| WEB_LOCK_FAILED)?;
    let capacity = usize::try_from(plain)
        .ok()
        .and_then(|plain| plain.checked_add((plain / chunk + 1) * TAG_BYTES + HEADER_LEN))
        .ok_or(WEB_TOO_LARGE)?;
    let encryptor =
        Encryptor::new(Vec::with_capacity(capacity), &header, &key).map_err(|_| WEB_LOCK_FAILED)?;
    let mut writer = BufWriter::with_capacity(BUFFER_BYTES, encryptor);
    serde_json::to_writer(&mut writer, dump).map_err(|_| WEB_LOCK_FAILED)?;
    let encryptor = writer.into_inner().map_err(|_| WEB_LOCK_FAILED)?;
    encryptor.finish().map_err(|_| WEB_LOCK_FAILED)
}

/// Counts JSON bytes without keeping them, and stops the writer once past
/// the cap, so a dump far over it isn't walked to its end.
struct Measure {
    bytes: u64,
}

impl Write for Measure {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        self.bytes = self.bytes.saturating_add(len);
        if self.bytes > WEB_PLAIN_MAX {
            return Err(io::Error::other(WEB_TOO_LARGE));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn plain_len(dump: &WebDump) -> Result<u64, &'static str> {
    let mut measure = Measure { bytes: 0 };
    match serde_json::to_writer(&mut measure, dump) {
        Ok(()) => Ok(measure.bytes),
        Err(_) if measure.bytes > WEB_PLAIN_MAX => Err(WEB_TOO_LARGE),
        Err(_) => Err(WEB_LOCK_FAILED),
    }
}

#[cfg(target_arch = "wasm32")]
fn fresh_random() -> Result<[u8; 23], &'static str> {
    let mut bytes = [0; 23];
    web_sys::window()
        .ok_or(WEB_LOCK_FAILED)?
        .crypto()
        .map_err(|_| WEB_LOCK_FAILED)?
        .get_random_values_with_u8_array(&mut bytes)
        .map_err(|_| WEB_LOCK_FAILED)?;
    Ok(bytes)
}

/// Off the browser there is no `crypto.getRandomValues`, and no other
/// source stands in for it.
#[cfg(not(target_arch = "wasm32"))]
fn fresh_random() -> Result<[u8; 23], &'static str> {
    Err(WEB_LOCK_FAILED)
}

/// A web part back as its dump. Only a web part opens (any other kind is
/// `NOT_A_BACKUP`); a wrong password is `WRONG_PASSWORD`; a dump from a
/// newer Bruhs is `NEWER_WEB_DATA` and one from before an upgrade that
/// moved data is `OLDER_WEB_DATA` (ruling F27).
pub fn unlock_dump(blob: &[u8], password: &str) -> Result<WebDump, &'static str> {
    if u64::try_from(blob.len()).map_or(true, |len| len > WEB_UPLOAD_MAX) {
        return Err(WEB_TOO_LARGE);
    }
    let password = Zeroizing::new(normalize_password(password));
    let decryptor =
        Decryptor::open(blob, Secret::Password(&password), Kind::WebPart).map_err(opening)?;
    let mut reader =
        BufReader::with_capacity(BUFFER_BYTES, io::Read::take(decryptor, WEB_PLAIN_MAX));
    let dump: WebDump = serde_json::from_reader(&mut reader).map_err(|_| DAMAGED)?;
    reader
        .into_inner()
        .into_inner()
        .finish()
        .map_err(|_| DAMAGED)?;
    check_versions(&dump, DB_VERSION, WEB_DATA_MIGRATIONS)?;
    Ok(dump)
}

/// The decryptor's first answer, as copy.
fn opening(error: FormatError) -> &'static str {
    match error {
        FormatError::NotABackup => NOT_A_BACKUP,
        FormatError::NewerVersion => NEWER_BACKUP,
        FormatError::WrongPasswordOrDamaged => WRONG_PASSWORD,
        FormatError::BadHeader
        | FormatError::BadParams
        | FormatError::Truncated
        | FormatError::TrailingData
        | FormatError::Io(_) => DAMAGED,
    }
}

/// Whether this build can put `dump` back: not version 0 of either
/// (`DAMAGED`), not from a newer format or database (`current`), and not
/// from before the newest of `migrations`.
fn check_versions(dump: &WebDump, current: u32, migrations: &[u32]) -> Result<(), &'static str> {
    // No build writes version 0 of either.
    if dump.version == 0 || dump.db_version == 0 {
        return Err(DAMAGED);
    }
    if dump.version > WEB_DUMP_VERSION || dump.db_version > current {
        return Err(NEWER_WEB_DATA);
    }
    if migrations
        .iter()
        .max()
        .is_some_and(|&newest| dump.db_version < newest)
    {
        return Err(OLDER_WEB_DATA);
    }
    Ok(())
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// Puts `dump` back (D19): one readwrite transaction over every store this
/// database has clears each and puts every record, then the `bruhs.*`
/// localStorage keys are replaced. Stores this build lacks are skipped and
/// named. Every record is turned back into a JS value first, so a damaged
/// dump changes nothing. Run [`unlock_dump`] (and the path rewrite) first.
pub async fn restore_web(dump: WebDump) -> Result<WebRestoreReport, String> {
    let db = open_db().await.map_err(|_| WEB_WRITE_FAILED.to_owned())?;
    let known = db.store_names();
    let skipped_stores = skipped_stores(&known, &dump);
    let WebDump {
        stores,
        local_storage,
        ..
    } = dump;
    let mut puts = Vec::new();
    let mut records = 0_u64;
    for (name, values) in stores {
        if !known.contains(&name) {
            continue;
        }
        let values = values
            .iter()
            .map(json_to_js)
            .collect::<Result<Vec<_>, ()>>()
            .map_err(|()| DAMAGED.to_owned())?;
        records = records.saturating_add(u64::try_from(values.len()).unwrap_or(u64::MAX));
        puts.push((name, values));
    }
    if !known.is_empty() {
        let tx = LocalTransaction::new(
            db.transaction(&known, TransactionMode::ReadWrite)
                .map_err(|_| WEB_WRITE_FAILED.to_owned())?,
        );
        let result = async {
            for name in &known {
                tx.object_store(name)?.clear()?.await?;
            }
            for (name, values) in &puts {
                let store = tx.object_store(name)?;
                for value in values {
                    store.put(value, None)?.await?;
                }
            }
            Ok(())
        }
        .await;
        metadata::finish(tx, result)
            .await
            .map_err(|_| WEB_WRITE_FAILED.to_owned())?;
    }
    replace_local_storage(&local_storage).ok_or(WEB_WRITE_FAILED)?;
    Ok(WebRestoreReport {
        records,
        skipped_stores,
    })
}

/// The dump's stores this database doesn't have.
fn skipped_stores(known: &[String], dump: &WebDump) -> Vec<String> {
    dump.stores
        .keys()
        .filter(|name| !known.contains(name))
        .cloned()
        .collect()
}

fn replace_local_storage(wanted: &BTreeMap<String, String>) -> Option<()> {
    let storage = local_storage()?;
    let current = local_keys(&storage)?;
    let (remove, set) = local_plan(&current, wanted);
    for key in remove {
        storage.remove_item(key).ok()?;
    }
    for (key, value) in set {
        storage.set_item(key, value).ok()?;
    }
    Some(())
}

/// The `bruhs.*` keys to remove (those `wanted` lacks) and to set. Other
/// keys are left alone, and a dump's non-`bruhs.*` keys are never written.
fn local_plan<'a>(
    current: &'a [String],
    wanted: &'a BTreeMap<String, String>,
) -> (Vec<&'a str>, Vec<(&'a str, &'a str)>) {
    let remove = current
        .iter()
        .filter(|key| is_bruhs_key(key) && !wanted.contains_key(*key))
        .map(String::as_str)
        .collect();
    let set = wanted
        .iter()
        .filter(|(key, _)| is_bruhs_key(key))
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    (remove, set)
}

/// JSON back as the IndexedDB value [`js_to_json`] read: tags become a
/// `Uint8Array`, a `Blob` with its type, a `Date`, `undefined` or the plain
/// object they escaped. A damaged tag is `Err`.
pub fn json_to_js(value: &Value) -> Result<JsValue, ()> {
    Ok(match value {
        Value::Null => JsValue::NULL,
        Value::Bool(flag) => JsValue::from_bool(*flag),
        Value::Number(number) => JsValue::from_f64(number.as_f64().ok_or(())?),
        Value::String(text) => JsValue::from_str(text),
        Value::Array(items) => {
            let array = Array::new();
            for item in items {
                array.push(&json_to_js(item)?);
            }
            array.into()
        }
        Value::Object(map) => match read_tag(value) {
            None => plain_object(map)?,
            Some(Ok(tag)) => tag_to_js(tag)?,
            Some(Err(_)) => return Err(()),
        },
    })
}

fn tag_to_js(tag: WebTag) -> Result<JsValue, ()> {
    Ok(match tag {
        WebTag::Bytes(bytes) => Uint8Array::from(bytes.as_slice()).into(),
        WebTag::Blob { mime, bytes } => {
            let parts = Array::of1(&Uint8Array::from(bytes.as_slice()));
            let options = web_sys::BlobPropertyBag::new();
            options.set_type(&mime);
            web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
                .map_err(|_| ())?
                .into()
        }
        WebTag::Date(ms) => js_sys::Date::new(&JsValue::from_f64(ms)).into(),
        WebTag::Undefined => JsValue::UNDEFINED,
        WebTag::Number(number) => JsValue::from_f64(number),
        WebTag::Object(map) => plain_object(&map)?,
    })
}

fn plain_object(map: &Map<String, Value>) -> Result<JsValue, ()> {
    let object = Object::new();
    for (key, item) in map {
        let item = json_to_js(item)?;
        let key = JsValue::from_str(key);
        if key == "__proto__" {
            // Assigning `__proto__` would set the prototype; an own field
            // of that name is defined instead.
            let field = Object::new();
            for (name, set) in [
                ("value", item),
                ("writable", JsValue::TRUE),
                ("enumerable", JsValue::TRUE),
                ("configurable", JsValue::TRUE),
            ] {
                Reflect::set(&field, &JsValue::from_str(name), &set).map_err(|_| ())?;
            }
            Reflect::define_property(&object, &key, &field).map_err(|_| ())?;
        } else {
            Reflect::set(&object, &key, &item).map_err(|_| ())?;
        }
    }
    Ok(object.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::Cell;

    use bruhs_models::EpochMs;
    use serde_json::json;

    const PASSWORD: &str = "correct horse battery";
    const RANDOM: [u8; 23] = [7; 23];

    fn fixed() -> Result<[u8; 23], &'static str> {
        Ok(RANDOM)
    }

    fn every_tag_dump() -> WebDump {
        let mut dump = WebDump {
            version: WEB_DUMP_VERSION,
            db_version: DB_VERSION,
            made_at_ms: EpochMs::new(1_700_000_000_000).unwrap(),
            ..WebDump::default()
        };
        dump.stores.insert(
            "metadata".into(),
            vec![json!({
                "key": "mail.file",
                "blob": tag_value(&WebTag::Blob { mime: "image/png".into(), bytes: vec![0, 1, 254, 255] }),
            })],
        );
        dump.stores.insert(
            "knowledge_attachments".into(),
            vec![json!({
                "id": "a1",
                "bytes": tag_value(&WebTag::Bytes((0..=255).collect())),
                "when": tag_value(&WebTag::Date(1.5e12)),
                "never": tag_value(&WebTag::Date(f64::NAN)),
                "gone": tag_value(&WebTag::Undefined),
                "nan": tag_value(&WebTag::Number(f64::NAN)),
                "far": [tag_value(&WebTag::Number(f64::INFINITY)), tag_value(&WebTag::Number(f64::NEG_INFINITY))],
                "own": tag_value(&WebTag::Object(Map::from_iter([
                    ("$bruhs".to_string(), json!("theirs")),
                ]))),
                "n": [0, -1, 9_007_199_254_740_991_i64, 0.1, 1e300, -0.0],
                "text": "é \u{0} \"quoted\"",
                "none": null,
            })],
        );
        dump.stores.insert("threads".into(), Vec::new());
        dump.local_storage
            .insert("bruhs.model".into(), "\"claude/opus\"".into());
        dump
    }

    #[test]
    fn dump_values_round_trip() {
        let dump = every_tag_dump();
        let blob = lock_with(&dump, PASSWORD, fixed).unwrap();
        assert_eq!(&blob[..8], b"BRUHSBAK");
        assert_eq!(unlock_dump(&blob, PASSWORD).unwrap(), dump);
        // The password is taken in its NFC form, as the server takes it.
        let nfc = lock_with(&dump, "caf\u{e9} au lait 1234", fixed).unwrap();
        assert_eq!(unlock_dump(&nfc, "cafe\u{301} au lait 1234").unwrap(), dump);
        assert_eq!(unlock_dump(&blob, "not the password"), Err(WRONG_PASSWORD));

        let mut newer = dump;
        newer.db_version = DB_VERSION + 1;
        let blob = lock_with(&newer, PASSWORD, fixed).unwrap();
        assert_eq!(unlock_dump(&blob, PASSWORD), Err(NEWER_WEB_DATA));

        // Checked before any random bytes are asked for or the key derived.
        let asked = Cell::new(false);
        let mut large = WebDump::default();
        large.local_storage.insert(
            "bruhs.big".into(),
            "x".repeat(usize::try_from(WEB_PLAIN_MAX).unwrap()),
        );
        let refused = lock_with(&large, PASSWORD, || {
            asked.set(true);
            fixed()
        });
        assert_eq!(refused, Err(WEB_TOO_LARGE));
        assert!(!asked.get());
    }

    #[test]
    fn only_web_parts_open_and_versions_are_checked() {
        // A full backup's header is refused before any key is derived.
        let header = Header::new(Kind::Full, Kdf::Argon2id(KdfParams::FLOOR), RANDOM);
        let key = Key::derive(PASSWORD, &header).unwrap();
        let full = Encryptor::new(Vec::new(), &header, &key)
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(unlock_dump(&full, PASSWORD), Err(NOT_A_BACKUP));
        assert_eq!(unlock_dump(b"plain text", PASSWORD), Err(NOT_A_BACKUP));

        let mut dump = WebDump {
            version: WEB_DUMP_VERSION,
            db_version: 13,
            ..WebDump::default()
        };
        assert_eq!(check_versions(&dump, 13, &[]), Ok(()));
        assert_eq!(check_versions(&dump, 14, &[12, 14]), Err(OLDER_WEB_DATA));
        dump.db_version = 14;
        assert_eq!(check_versions(&dump, 14, &[12, 14]), Ok(()));
        assert_eq!(check_versions(&dump, 13, &[]), Err(NEWER_WEB_DATA));
        dump.version = WEB_DUMP_VERSION + 1;
        assert_eq!(check_versions(&dump, 14, &[]), Err(NEWER_WEB_DATA));
        // No build writes version 0 of either.
        dump.version = 0;
        assert_eq!(check_versions(&dump, 14, &[]), Err(DAMAGED));
        dump.version = WEB_DUMP_VERSION;
        dump.db_version = 0;
        assert_eq!(check_versions(&dump, 14, &[]), Err(DAMAGED));
    }

    #[test]
    fn the_deepest_record_a_dump_takes_reads_back() {
        let mut deep = json!("leaf");
        for _ in 0..MAX_DEPTH {
            deep = json!([deep]);
        }
        let mut dump = WebDump {
            version: WEB_DUMP_VERSION,
            db_version: DB_VERSION,
            ..WebDump::default()
        };
        dump.stores.insert("threads".into(), vec![deep]);
        let blob = lock_with(&dump, PASSWORD, fixed).unwrap();
        assert_eq!(unlock_dump(&blob, PASSWORD).unwrap(), dump);
    }

    #[test]
    fn past_the_depth_cap_a_dump_would_not_read_back() {
        let mut deep = json!("leaf");
        for _ in 0..MAX_DEPTH + 8 {
            deep = json!([deep]);
        }
        let mut dump = WebDump {
            version: WEB_DUMP_VERSION,
            db_version: DB_VERSION,
            ..WebDump::default()
        };
        dump.stores.insert("threads".into(), vec![deep]);
        let blob = lock_with(&dump, PASSWORD, fixed).unwrap();
        assert_eq!(unlock_dump(&blob, PASSWORD), Err(DAMAGED));
    }

    #[test]
    fn the_size_estimate_stops_the_dump_past_the_cap() {
        let mut estimate = Estimate::default();
        assert_eq!(estimate.add(WEB_PLAIN_MAX - 12), Ok(()));
        // Bytes count as their base64: 3 bytes are 4, 4 bytes are 8.
        assert_eq!(estimate.add_binary(3), Ok(()));
        assert_eq!(estimate.add_binary(4), Ok(()));
        assert_eq!(estimate.bytes, WEB_PLAIN_MAX);
        // The running total carries on: one byte more stops it.
        assert_eq!(estimate.add(1), Err(Stop::TooLarge));

        let mut estimate = Estimate::default();
        assert_eq!(estimate.add_binary(WEB_PLAIN_MAX / 4 * 3), Ok(()));
        assert_eq!(estimate.add_binary(1), Err(Stop::TooLarge));
        let mut estimate = Estimate::default();
        assert_eq!(estimate.add(u64::MAX), Err(Stop::TooLarge));
        assert_eq!(stop_copy(Stop::TooLarge, "threads"), WEB_TOO_LARGE);
        assert_eq!(
            stop_copy(Stop::Unsupported, "threads"),
            unsupported("threads")
        );
    }

    #[test]
    fn whole_numbers_stay_integers() {
        assert_eq!(number_value(1.0), Some(json!(1)));
        assert_eq!(number_value(-42.0), Some(json!(-42)));
        assert_eq!(
            number_value(MAX_SAFE_INTEGER),
            Some(json!(9_007_199_254_740_991_i64))
        );
        assert_eq!(number_value(0.5), Some(json!(0.5)));
        let past = number_value(MAX_SAFE_INTEGER + 1.0).unwrap();
        assert!(past.is_f64(), "{past}");
        let negative_zero = number_value(-0.0).unwrap().as_f64().unwrap();
        assert!(negative_zero == 0.0 && negative_zero.is_sign_negative());
        for none in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(number_value(none), None);
        }
    }

    #[test]
    fn this_build_has_no_fallback_random_source() {
        // Off the browser there is no crypto.getRandomValues, and nothing
        // stands in for it.
        assert_eq!(
            lock_dump(&WebDump::default(), PASSWORD),
            Err(WEB_LOCK_FAILED)
        );
    }

    #[test]
    fn only_bruhs_keys_are_taken() {
        let taken = select_local_keys(&[
            ("bruhs.model", "\"opus\""),
            ("other.key", "1"),
            ("bruhsX", "2"),
            ("bruhs.mailDraft.body.a", "hi"),
        ]);
        assert_eq!(
            taken.keys().map(String::as_str).collect::<Vec<_>>(),
            ["bruhs.mailDraft.body.a", "bruhs.model"]
        );
        assert_eq!(taken["bruhs.model"], "\"opus\"");
    }

    #[test]
    fn a_restore_replaces_only_bruhs_keys_and_skips_unknown_stores() {
        let wanted = BTreeMap::from([
            ("bruhs.model".to_string(), "a".to_string()),
            ("other.key".to_string(), "b".to_string()),
        ]);
        let current = ["bruhs.model", "bruhs.old", "other.mine", "bruhsX"].map(String::from);
        let (remove, set) = local_plan(&current, &wanted);
        assert_eq!(remove, ["bruhs.old"]);
        assert_eq!(set, [("bruhs.model", "a")]);

        let mut dump = WebDump::default();
        for name in ["threads", "from_a_newer_bruhs", "metadata"] {
            dump.stores.insert(name.into(), Vec::new());
        }
        let known = ["metadata", "projects", "threads"].map(String::from);
        assert_eq!(skipped_stores(&known, &dump), ["from_a_newer_bruhs"]);
    }

    #[test]
    fn unsupported_names_the_store() {
        assert_eq!(
            unsupported("whiteboards"),
            "This browser holds data Bruhs can't put in a backup: whiteboards."
        );
    }
}
