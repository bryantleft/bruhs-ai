//! Metadata-only CLI usage import. Native and Bruhs totals stay separate because
//! older Bruhs receipts cannot prove which native sessions overlap.
use super::{private_directory, WRITER};
use bruhs_models::agent_extensions::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

// Reuse the local API upload budget for retained metadata. Scanned bytes have
// an explicit owner-selected budget, since native logs can exceed an upload.
pub const SCAN_BYTES: u64 = 50 << 20;
// Same single-JSON-frame bound as the ACP transport and default JSON API.
const FRAME_BYTES: u64 = 2 << 20;
const LEDGER: &str = "native-usage.json";
const PREVIEW: &str = "history-preview.json";
#[derive(Serialize, Deserialize, Default)]
struct Ledger {
    records: BTreeMap<String, UsageRecord>,
    imported_at: Option<i64>,
    #[serde(default)]
    last_committed_preview: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Frozen {
    preview: UsageHistoryPreview,
    records: BTreeMap<String, UsageRecord>,
}
fn warning(warnings: &mut Vec<String>, message: &str) {
    if !warnings.iter().any(|previous| previous == message) {
        warnings.push(message.into());
    }
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
pub fn source_root(request: &UsageHistoryRequest) -> PathBuf {
    request.root.as_ref().map_or_else(
        || {
            let roots = crate::import::ImportRoots::from_env();
            match request.source {
                UsageHistorySource::Claude => roots.claude,
                UsageHistorySource::Codex => roots.codex,
            }
        },
        PathBuf::from,
    )
}
fn open_regular(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid(
            "Usage import accepts regular files only; symlinks and special files are excluded.",
        ));
    }
    Ok(file)
}
fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let file = open_regular(path)?;
    if file.metadata()?.len() > SCAN_BYTES {
        return Err(metadata_limit());
    }
    let mut bytes = Vec::new();
    file.take(SCAN_BYTES + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > SCAN_BYTES {
        return Err(metadata_limit());
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
fn metadata_limit() -> io::Error {
    invalid("Native usage metadata exceeds the 50 MiB storage budget. No import was applied; archive or reset the private imported-usage ledger before importing more history.")
}
fn read_ledger(root: &Path) -> io::Result<Ledger> {
    match bounded_json(&root.join(LEDGER)) {
        Ok(ledger) => Ok(ledger),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(e),
    }
}
struct BoundedWriter<W> {
    inner: W,
    remaining: u64,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > self.remaining {
            return Err(metadata_limit());
        }
        let written = self.inner.write(bytes)?;
        self.remaining = self
            .remaining
            .saturating_sub(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
struct HistoryLock(File);
impl Drop for HistoryLock {
    fn drop(&mut self) {
        // fork may briefly inherit a CLOEXEC descriptor. Release the shared
        // file-description lock explicitly, before that child's eventual exec.
        if let Err(error) = self.0.unlock() {
            tracing::warn!(%error, "Could not explicitly release the usage history lock");
        }
    }
}
fn process_lock(root: &Path) -> io::Result<HistoryLock> {
    private_directory(root)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(root.join("usage-history.lock"))?;
    if !file.metadata()?.is_file() {
        return Err(invalid("The usage history lock is not a regular file."));
    }
    file.try_lock().map_err(|_| {
        invalid("Another companion is updating usage history. Retry after its import finishes.")
    })?;
    Ok(HistoryLock(file))
}
fn atomic<T: Serialize>(root: &Path, path: &Path, value: &T) -> io::Result<()> {
    private_directory(root)?;
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    {
        let mut bounded = BoundedWriter {
            inner: &mut file,
            remaining: SCAN_BYTES,
        };
        serde_json::to_writer(&mut bounded, value).map_err(io::Error::other)?;
        bounded.flush()?;
    }
    file.as_file().sync_all()?;
    file.persist(path).map_err(io::Error::other)?;
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    Ok(())
}
fn aggregate(
    records: &BTreeMap<String, UsageRecord>,
    prices: &BTreeMap<String, ModelPrice>,
    at: Option<i64>,
) -> ImportedUsageSnapshot {
    let mut out = ImportedUsageSnapshot {
        records: records.len(),
        imported_at: at,
        ..Default::default()
    };
    for record in records.values() {
        let price = prices.get(&record.model);
        out.by_model
            .entry(record.model.clone())
            .or_default()
            .add(record, price);
        out.by_project
            .entry(record.project.clone())
            .or_default()
            .add(record, price);
        if let Some(time) = chrono::DateTime::from_timestamp(record.at, 0) {
            out.by_day
                .entry(time.format("%Y-%m-%d").to_string())
                .or_default()
                .add(record, price);
        }
    }
    out
}
pub fn snapshot_at(
    root: &Path,
    prices: &BTreeMap<String, ModelPrice>,
) -> io::Result<ImportedUsageSnapshot> {
    let ledger = read_ledger(root)?;
    Ok(aggregate(&ledger.records, prices, ledger.imported_at))
}
fn discover(root: &Path, budget: u64) -> io::Result<(Vec<PathBuf>, bool)> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut bytes = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            // Paths and queued directory entries share the explicit scan budget.
            let cost = u64::try_from(path.as_os_str().len())
                .unwrap_or(u64::MAX)
                .saturating_add(64);
            bytes = bytes.saturating_add(cost);
            if bytes > budget {
                files.sort();
                return Ok((files, true));
            }
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok((files, false))
}
struct LogTree {
    root: PathBuf,
    #[cfg(unix)]
    directory: File,
}
impl LogTree {
    fn open(root: PathBuf) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&root)?;
            Ok(Self { root, directory })
        }
        #[cfg(not(unix))]
        {
            Ok(Self { root })
        }
    }
    fn file(&self, path: &Path) -> io::Result<File> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            invalid("A native log moved outside the chosen CLI directory. Restart the preview.")
        })?;
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd as _, FromRawFd as _},
                unix::ffi::OsStrExt as _,
            };
            let mut directory = self.directory.try_clone()?;
            let mut components = relative.components().peekable();
            while let Some(component) = components.next() {
                let std::path::Component::Normal(name) = component else {
                    return Err(invalid("A native log path contains an invalid component."));
                };
                let name = std::ffi::CString::new(name.as_bytes())
                    .map_err(|_| invalid("A native log filename contains NUL."))?;
                let last = components.peek().is_none();
                let flags = libc::O_RDONLY
                    | libc::O_CLOEXEC
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | if last { 0 } else { libc::O_DIRECTORY };
                // SAFETY: directory owns an open directory descriptor; name is a live
                // NUL-terminated single path component. No pointer is retained by openat.
                let descriptor =
                    unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
                if descriptor < 0 {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: a successful openat returned a new owned descriptor. This
                // File is its only owner and closes it on every return/error path.
                let file = unsafe { File::from_raw_fd(descriptor) };
                if last {
                    if !file.metadata()?.is_file() {
                        return Err(invalid(
                            "Native logs must be regular files; special files are excluded.",
                        ));
                    }
                    return Ok(file);
                }
                directory = file;
            }
            Err(invalid(
                "Choose a native log file inside the CLI log directory.",
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = relative;
            if std::fs::canonicalize(path)? != path {
                return Err(invalid(
                    "A native log path changed or became a symlink. Restart the preview.",
                ));
            }
            open_regular(path)
        }
    }
}
fn inventory(root: &Path, paths: &[PathBuf], source: UsageHistorySource) -> String {
    let mut hash = Sha256::new();
    hash.update(match source {
        UsageHistorySource::Claude => b"claude".as_slice(),
        UsageHistorySource::Codex => b"codex".as_slice(),
    });
    for path in std::iter::once(root).chain(paths.iter().map(PathBuf::as_path)) {
        let bytes = path.as_os_str().as_encoded_bytes();
        hash.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}
fn timestamp(row: &Value) -> Option<i64> {
    row["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|time| time.timestamp())
}
#[derive(Default, Clone, Copy)]
struct Counters {
    input: u64,
    output: u64,
    read: u64,
    write: u64,
}
impl Counters {
    fn parse(value: &Value, codex: bool) -> Option<Self> {
        let input = value["input_tokens"].as_u64()?;
        let output = value["output_tokens"].as_u64()?;
        let read = value[if codex {
            "cached_input_tokens"
        } else {
            "cache_read_input_tokens"
        }]
        .as_u64()
        .unwrap_or(0);
        let write = value["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        if codex && read > input {
            return None;
        }
        Some(Self {
            input,
            output,
            read,
            write,
        })
    }
    fn delta(self, before: Self) -> Option<Self> {
        Some(Self {
            input: self.input.checked_sub(before.input)?,
            output: self.output.checked_sub(before.output)?,
            read: self.read.checked_sub(before.read)?,
            write: self.write.checked_sub(before.write)?,
        })
    }
    fn empty(self) -> bool {
        self.input == 0 && self.output == 0 && self.read == 0 && self.write == 0
    }
}
fn merge(records: &mut BTreeMap<String, UsageRecord>, record: UsageRecord) -> io::Result<()> {
    if let Some(old) = records.get_mut(&record.id) {
        if old.model != record.model
            || old.project != record.project
            || old.thread_id != record.thread_id
        {
            return Err(invalid("A native usage identity has conflicting model/project metadata. No import was applied."));
        }
        if record.id.starts_with("codex:")
            && (old.input != record.input
                || old.output != record.output
                || old.cache_read != record.cache_read
                || old.cache_write != record.cache_write)
        {
            return Err(invalid("A Codex cumulative identity has conflicting deltas in overlapping session logs. No import was applied. Choose an authoritative copy of those session logs before retrying."));
        }
        old.input = old.input.max(record.input);
        old.output = old.output.max(record.output);
        old.cache_read = old.cache_read.max(record.cache_read);
        old.cache_write = old.cache_write.max(record.cache_write);
    } else {
        records.insert(record.id.clone(), record);
    }
    Ok(())
}
fn parse_file(
    path: &Path,
    file: File,
    source: UsageHistorySource,
    budget: &mut u64,
    warnings: &mut Vec<String>,
) -> io::Result<(BTreeMap<String, UsageRecord>, bool)> {
    let mut reader = BufReader::new(file);
    let mut records = BTreeMap::new();
    let mut session = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_owned();
    let mut project = "Unknown project".to_owned();
    let mut model = None;
    let mut previous = Counters::default();
    let mut retained = 0usize;
    loop {
        if *budget == 0 {
            return Ok((records, !reader.fill_buf()?.is_empty()));
        }
        let limit = FRAME_BYTES.min(*budget);
        let mut line = Vec::new();
        let n = (&mut reader)
            .take(limit.saturating_add(1))
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        let n = u64::try_from(n).map_err(io::Error::other)?;
        if n > *budget {
            warning(warnings, "Scan byte budget reached. This file was excluded; increase the budget to retry it from the beginning.");
            return Ok((records, true));
        }
        *budget -= n;
        if n > FRAME_BYTES {
            if matches!(source, UsageHistorySource::Codex) {
                warning(warnings, "A Codex frame exceeded 2 MiB and could contain model/session context. This file was excluded; no usage was attributed to a stale model.");
                return Ok((BTreeMap::new(), false));
            }
            let mut ended = line.last() == Some(&b'\n');
            while !ended {
                if *budget == 0 {
                    return Ok((records, true));
                }
                line.clear();
                let n = (&mut reader)
                    .take(FRAME_BYTES.min(*budget))
                    .read_until(b'\n', &mut line)?;
                *budget = budget.saturating_sub(u64::try_from(n).unwrap_or(u64::MAX));
                ended = n == 0 || line.last() == Some(&b'\n');
            }
            warning(
                warnings,
                "A native JSON frame exceeded the 2 MiB metadata budget and was skipped.",
            );
            continue;
        }
        let row: Value = match serde_json::from_slice(&line) {
            Ok(row) => row,
            Err(_) => {
                if matches!(source, UsageHistorySource::Codex) {
                    warning(warnings, "A Codex row was incomplete or malformed and could contain model/session context. This file was excluded; retry after its log is complete.");
                    return Ok((BTreeMap::new(), false));
                }
                warning(
                    warnings,
                    "An incomplete or malformed native row was skipped.",
                );
                continue;
            }
        };
        if let Some(cwd) = row["cwd"]
            .as_str()
            .or_else(|| row["payload"]["cwd"].as_str())
        {
            project = cwd.into();
        }
        if let Some(id) = row["sessionId"].as_str() {
            session = id.into();
        }
        let (id, counters) = match source {
            UsageHistorySource::Claude => {
                if row["type"] != "assistant" {
                    continue;
                }
                let message = &row["message"];
                let Some(name) = message["model"].as_str() else {
                    continue;
                };
                model = Some(name.to_owned());
                let Some(id) = message["id"].as_str() else {
                    warning(warnings, "A Claude usage row has no message identity.");
                    continue;
                };
                let Some(counters) = Counters::parse(&message["usage"], false) else {
                    continue;
                };
                (format!("claude:{session}:{id}"), counters)
            }
            UsageHistorySource::Codex => {
                if row["type"] == "session_meta" {
                    if let Some(id) = row["payload"]["id"].as_str() {
                        session = id.into();
                    }
                    continue;
                }
                if row["type"] == "turn_context" {
                    model = row["payload"]["model"].as_str().map(str::to_owned);
                    continue;
                }
                if row["type"] != "event_msg" || row["payload"]["type"] != "token_count" {
                    continue;
                }
                let Some(total) =
                    Counters::parse(&row["payload"]["info"]["total_token_usage"], true)
                else {
                    continue;
                };
                let Some(delta) = total.delta(previous) else {
                    warning(warnings, "A Codex session counter decreased; that session was excluded rather than guessing a reset.");
                    return Ok((BTreeMap::new(), false));
                };
                previous = total;
                let Some(input) = delta.input.checked_sub(delta.read) else {
                    warning(
                        warnings,
                        "A Codex cache delta exceeded its input; the session was excluded.",
                    );
                    return Ok((BTreeMap::new(), false));
                };
                (
                    format!(
                        "codex:{session}:{}:{}:{}",
                        total.input, total.output, total.read
                    ),
                    Counters { input, ..delta },
                )
            }
        };
        if counters.empty() {
            continue;
        }
        let (Some(model), Some(at)) = (model.clone(), timestamp(&row)) else {
            warning(warnings, "Usage without a model or timestamp was skipped.");
            continue;
        };
        let record = UsageRecord {
            id,
            thread_id: session.clone(),
            model,
            project: project.clone(),
            at,
            input: counters.input,
            output: counters.output,
            cache_read: counters.read,
            cache_write: counters.write,
        };
        if !records.contains_key(&record.id) {
            retained = retained
                .saturating_add(record.id.len())
                .saturating_add(serde_json::to_vec(&record).map_err(io::Error::other)?.len());
            if u64::try_from(retained).unwrap_or(u64::MAX) > SCAN_BYTES {
                return Err(invalid("Native usage metadata exceeded 50 MiB. Select fewer files or archive older native logs before importing."));
            }
        }
        merge(&mut records, record)?;
    }
    Ok((records, false))
}
pub fn preview_at(store: &Path, request: UsageHistoryRequest) -> io::Result<UsageHistoryPreview> {
    if request.max_files == 0
        || request.max_files > bruhs_models::reactive::HISTORY_LIMIT
        || request.max_bytes == 0
    {
        return Err(invalid(
            "Choose 1–50 files and a nonzero scan budget per preview.",
        ));
    }
    let cli_root = std::fs::canonicalize(source_root(&request))?;
    let logs = std::fs::canonicalize(cli_root.join(match request.source {
        UsageHistorySource::Claude => "projects",
        UsageHistorySource::Codex => "sessions",
    }))?;
    if !logs.starts_with(&cli_root) {
        return Err(invalid("The CLI log directory resolves outside the chosen CLI data directory. Choose its real parent directory; no logs were imported."));
    }
    let tree = LogTree::open(logs.clone())?;
    let (paths, directory_limited) = discover(&logs, SCAN_BYTES)?;
    if directory_limited {
        return Err(invalid("Native log directory metadata exceeded 50 MiB. Archive older log directories before importing; no partial directory listing was imported."));
    }
    let inventory = inventory(&cli_root, &paths, request.source);
    if request.offset > 0 && request.inventory.as_deref() != Some(&inventory) {
        return Err(invalid("The native log file inventory changed or this page has no first-page identity. Restart the preview from the first file page; no import was applied."));
    }
    let mut limited = false;
    let mut remaining = request.max_bytes;
    let mut records = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut files = 0;
    let mut minimum_next_bytes = None;
    for path in paths.iter().skip(request.offset).take(request.max_files) {
        let (rows, cut) = parse_file(
            path,
            tree.file(path)?,
            request.source,
            &mut remaining,
            &mut warnings,
        )?;
        if cut {
            limited = true;
            minimum_next_bytes = Some(std::fs::metadata(path)?.len().saturating_add(1));
            warning(&mut warnings, "An incomplete file was excluded. The next page retries that file with a larger scan budget.");
            break;
        }
        for row in rows.into_values() {
            merge(&mut records, row)?;
        }
        files += 1;
        if serde_json::to_vec(&records)
            .map_err(io::Error::other)?
            .len()
            > usize::try_from(SCAN_BYTES).unwrap_or(usize::MAX)
        {
            return Err(invalid(
                "Imported metadata exceeded the preview budget. Choose fewer files.",
            ));
        }
    }
    if serde_json::to_vec(&records)
        .map_err(io::Error::other)?
        .len()
        > usize::try_from(SCAN_BYTES).unwrap_or(usize::MAX)
    {
        return Err(invalid(
            "Imported metadata exceeded the preview budget. Choose fewer files.",
        ));
    }
    let config = super::load_config_at(store)?;
    let preview = UsageHistoryPreview {
        id: uuid::Uuid::new_v4().to_string(),
        source: request.source,
        root: cli_root.to_string_lossy().into_owned(),
        inventory,
        files_read: files,
        bytes_read: request.max_bytes - remaining,
        next_offset: (request.offset + files < paths.len()).then_some(request.offset + files),
        minimum_next_bytes,
        limited,
        warnings,
        usage: aggregate(&records, &config.prices, None),
    };
    let _guard = WRITER
        .lock()
        .map_err(|_| invalid("Usage import lock failed."))?;
    let _process = process_lock(store)?;
    atomic(
        store,
        &store.join(PREVIEW),
        &Frozen {
            preview: preview.clone(),
            records,
        },
    )?;
    Ok(preview)
}
pub fn commit_at(store: &Path, id: &str) -> io::Result<ImportedUsageSnapshot> {
    uuid::Uuid::parse_str(id).map_err(|_| invalid("Choose an existing usage import preview."))?;
    let _guard = WRITER
        .lock()
        .map_err(|_| invalid("Usage import lock failed."))?;
    let _process = process_lock(store)?;
    let mut ledger = read_ledger(store)?;
    let config = super::load_config_at(store)?;
    if ledger.last_committed_preview.as_deref() == Some(id) {
        drop(_process);
        drop(_guard);
        return Ok(aggregate(
            &ledger.records,
            &config.prices,
            ledger.imported_at,
        ));
    }
    let path = store.join(PREVIEW);
    let frozen: Frozen = bounded_json(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            invalid(
                "This usage preview was consumed or replaced. Preview and review the import again.",
            )
        } else {
            error
        }
    })?;
    if frozen.preview.id != id {
        return Err(invalid("This usage preview was replaced by a newer preview. Review the latest metadata before importing."));
    }
    for record in frozen.records.into_values() {
        merge(&mut ledger.records, record)?;
    }
    ledger.imported_at = Some(chrono::Utc::now().timestamp());
    ledger.last_committed_preview = Some(id.to_owned());
    atomic(store, &store.join(LEDGER), &ledger)?;
    std::fs::remove_file(path)?;
    #[cfg(unix)]
    File::open(store)?.sync_all()?;
    drop(_process);
    drop(_guard);
    Ok(aggregate(
        &ledger.records,
        &config.prices,
        ledger.imported_at,
    ))
}

#[cfg(test)]
mod tests;
