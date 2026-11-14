//! Polling sources reconcile durable observations instead of relying on live-only notifications.

use super::{ReactiveError, ReactiveRuntime, SourceEmission};
use bruhs_models::reactive::{EventSource, SourceKind, MAX_PAYLOAD_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    task::{AbortHandle, JoinHandle, JoinSet},
};

// A source can hold two 32 KiB process pipes or one HTTP body within the
// 64 KiB event budget. Oversized encoded events fail before their cursor commits.
const MAX_SOURCE_BODY_BYTES: usize = MAX_PAYLOAD_BYTES / 2;
// At most 4 MiB of persisted folder inventory per source. The entry estimate
// bounds traversal allocation; the encoded-size check enforces the actual budget.
const MAX_FOLDER_CURSOR_BYTES: usize = 4 * 1024 * 1024;
const MAX_TRACKED_FILES: usize = MAX_FOLDER_CURSOR_BYTES / 1024;

/// The returned task owns every source task. Aborting it stops polling and commands.
pub(super) fn start(runtime: ReactiveRuntime) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = manage(runtime).await {
            tracing::error!("Reactive event sources stopped: {error}");
        }
    })
}

async fn manage(runtime: ReactiveRuntime) -> Result<(), ReactiveError> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|error| source_error("Could not create the source HTTP client", error))?;
    let mut changes = runtime.watch_changes();
    let mut tasks = JoinSet::new();
    let mut active: HashMap<String, (EventSource, AbortHandle)> = HashMap::new();

    loop {
        let configured = runtime.source_configs().await?;
        active.retain(|id, (previous, task)| {
            let keep = configured
                .iter()
                .any(|source| source.id == *id && source.enabled && source == previous);
            if !keep {
                task.abort();
            }
            keep
        });
        for source in configured {
            if !source.enabled
                || polling_interval(&source.kind).is_none()
                || active.contains_key(&source.id)
            {
                continue;
            }
            let (source_runtime, source_client, config) =
                (runtime.clone(), client.clone(), source.clone());
            let handle = tasks.spawn(async move {
                drive(source_runtime, config, source_client).await;
            });
            active.insert(source.id.clone(), (source, handle));
        }

        tokio::select! {
            result = changes.changed() => {
                if result.is_err() { return Ok(()); }
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(error) = result {
                    let failed = active.iter().find(|(_, (_, task))| task.id() == error.id()).map(|(id, _)| id.clone());
                    if let Some(id) = failed {
                        runtime.source_status(&id, Some("The event source worker stopped. Disable and enable the source to restart it.".into())).await?;
                    }
                }
            }
        }
    }
}

async fn drive(runtime: ReactiveRuntime, source: EventSource, client: reqwest::Client) {
    let Some(interval) = polling_interval(&source.kind) else {
        return;
    };
    loop {
        let error = poll_once(&runtime, &source, &client)
            .await
            .err()
            .map(|error| error.to_string());
        if let Err(error) = runtime.source_status(&source.id, error).await {
            tracing::error!(source_id = %source.id, "Could not save event source health: {error}");
            return;
        }
        // Sleeping after the completed poll prevents overlap and missed-tick bursts.
        tokio::time::sleep(interval).await;
    }
}

fn polling_interval(kind: &SourceKind) -> Option<Duration> {
    let millis = match kind {
        SourceKind::Folder { poll_ms, .. }
        | SourceKind::Http { poll_ms, .. }
        | SourceKind::Command { poll_ms, .. } => *poll_ms,
        SourceKind::Schedule { every_ms } => *every_ms,
        SourceKind::Webhook | SourceKind::AppEvent { .. } | SourceKind::Custom => return None,
    };
    Some(Duration::from_millis(millis.max(1)))
}

/// One scan plus a transactional batch of emitted events and the new checkpoint.
pub async fn poll_once(
    runtime: &ReactiveRuntime,
    source: &EventSource,
    client: &reqwest::Client,
) -> Result<(), ReactiveError> {
    let cursor_key = match &source.kind {
        SourceKind::Folder { .. } => "folder",
        SourceKind::Http { .. } => "http",
        SourceKind::Command { .. } => "command",
        SourceKind::Schedule { .. } => "schedule",
        SourceKind::Webhook | SourceKind::AppEvent { .. } | SourceKind::Custom => return Ok(()),
    };
    let cursor = runtime.cursor(&source.id, cursor_key).await?;
    let batch = match &source.kind {
        SourceKind::Folder {
            path,
            pattern,
            recursive,
            settle_ms,
            ..
        } => {
            let (path, pattern, recursive) = (path.clone(), pattern.clone(), *recursive);
            let files = tokio::task::spawn_blocking(move || {
                scan_folder(Path::new(&path), &pattern, recursive)
            })
            .await
            .map_err(|_| {
                ReactiveError::Invalid(
                    "The folder scan worker stopped. No checkpoint was saved.".into(),
                )
            })??;
            let capacity = runtime.ingestion_capacity(source).await?;
            folder_batch(cursor, files, source, now_ms()?, *settle_ms, capacity)?
        }
        SourceKind::Http { url, poll_ms } => {
            let body = read_http(client, url, *poll_ms).await?;
            body_batch(cursor, "http", url, &body, json!({ "url": url }))?
        }
        SourceKind::Command {
            command,
            cwd,
            timeout_ms,
            ..
        } => {
            let body = read_command(command, Path::new(cwd), *timeout_ms).await?;
            body_batch(
                cursor,
                "command",
                &format!("{cwd}\n{command}"),
                &body,
                json!({ "cwd": cwd }),
            )?
        }
        SourceKind::Schedule { every_ms } => schedule_batch(cursor, now_ms()?, *every_ms)?,
        SourceKind::Webhook | SourceKind::AppEvent { .. } | SourceKind::Custom => return Ok(()),
    };
    for event in &batch.events {
        if serde_json::to_vec(&event.payload)
            .map_err(encoding_error)?
            .len()
            > MAX_PAYLOAD_BYTES
        {
            return Err(ReactiveError::Invalid(format!("Source event exceeds the {MAX_PAYLOAD_BYTES} byte payload budget. No checkpoint was saved.")));
        }
    }
    let waiting_for_capacity = batch.backpressured && batch.events.is_empty();
    runtime
        .ingest(source, batch.events, cursor_key, batch.cursor)
        .await?;
    if waiting_for_capacity {
        return Err(ReactiveError::Conflict("Settled files are waiting for event queue space. Review or cancel pending runs before the next folder scan.".into()));
    }
    Ok(())
}

struct PollBatch {
    events: Vec<SourceEmission>,
    cursor: Value,
    backpressured: bool,
}

fn now_ms() -> Result<u64, ReactiveError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| source_error("The system clock is before the Unix epoch", error))?
        .as_millis()
        .try_into()
        .map_err(|_| {
            ReactiveError::Invalid(
                "The system clock exceeds the supported source timestamp range.".into(),
            )
        })
}

fn source_error(operation: &str, error: impl std::fmt::Display) -> ReactiveError {
    ReactiveError::Invalid(format!("{operation}: {error}"))
}

fn encoding_error(error: serde_json::Error) -> ReactiveError {
    source_error("The source checkpoint could not be encoded", error)
}

fn decode_cursor<T: serde::de::DeserializeOwned + Default>(
    cursor: Option<Value>,
) -> Result<T, ReactiveError> {
    cursor.map(serde_json::from_value).transpose()
        .map_err(|error| source_error("The saved source checkpoint is invalid. Restore its database backup before retrying", error))
        .map(|cursor| cursor.unwrap_or_default())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug)]
struct ScannedFile {
    path: String,
    relative: String,
    size: u64,
    fingerprint: String,
}

fn scan_folder(
    root: &Path,
    pattern: &str,
    recursive: bool,
) -> Result<Vec<ScannedFile>, ReactiveError> {
    let root = root
        .canonicalize()
        .map_err(|error| source_error("Could not open the watched folder", error))?;
    if !root.is_dir() {
        return Err(ReactiveError::Invalid(
            "The watched path is not a folder.".into(),
        ));
    }
    let mut pending = vec![root.clone()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let kind = directory
            .symlink_metadata()
            .map_err(|error| source_error("Could not inspect a watched subfolder", error))?;
        if kind.file_type().is_symlink() {
            continue;
        }
        let current = directory
            .canonicalize()
            .map_err(|error| source_error("Could not resolve a watched subfolder", error))?;
        if !current.starts_with(&root) {
            continue;
        }
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| source_error("Could not read the watched folder", error))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| source_error("Could not read a folder entry", error))?;
            let path = entry.path();
            let metadata = match path.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(source_error("Could not inspect a watched file", error)),
            };
            // A symlink is not a file or subfolder belonging to this source.
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if recursive && !ignored_directory(&entry.file_name()) {
                    if pending.len() + files.len() >= MAX_TRACKED_FILES {
                        return Err(folder_budget_error());
                    }
                    pending.push(path);
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let relative = path.strip_prefix(&root).map_err(|_| {
                ReactiveError::Invalid("A watched file left its configured folder.".into())
            })?;
            let relative = path_text(relative)?;
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    ReactiveError::Invalid(
                        "A watched filename is not valid UTF-8. Rename it before retrying.".into(),
                    )
                })?;
            let candidate = if pattern.contains('/') {
                relative.as_str()
            } else {
                filename
            };
            if !pattern
                .split(',')
                .any(|mask| wildcard_match(mask.trim(), candidate))
            {
                continue;
            }
            if files.len() >= MAX_TRACKED_FILES {
                return Err(folder_budget_error());
            }
            let modified = metadata.modified().map_err(|error| {
                source_error("Could not read a watched file's modification time", error)
            })?;
            let stamp = match modified.duration_since(UNIX_EPOCH) {
                Ok(duration) => format!("{}", duration.as_nanos()),
                Err(error) => format!("-{}", error.duration().as_nanos()),
            };
            #[cfg(unix)]
            let identity = {
                use std::os::unix::fs::MetadataExt;
                format!(
                    "{}:{}:{}:{}",
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec()
                )
            };
            #[cfg(not(unix))]
            let identity = format!("{:?}", metadata.created().ok());
            let fingerprint = digest(format!("{}:{stamp}:{identity}", metadata.len()).as_bytes());
            files.push(ScannedFile {
                path: path_text(&path)?,
                relative,
                size: metadata.len(),
                fingerprint,
            });
        }
    }
    files.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn path_text(path: &Path) -> Result<String, ReactiveError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        ReactiveError::Invalid(
            "A watched path is not valid UTF-8. Rename it before retrying.".into(),
        )
    })
}

fn ignored_directory(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            ".git"
                | ".hg"
                | ".svn"
                | "node_modules"
                | "target"
                | ".venv"
                | "__pycache__"
                | "DerivedData"
        )
    )
}

fn folder_budget_error() -> ReactiveError {
    ReactiveError::Invalid(format!("The watched folder exceeds the {MAX_TRACKED_FILES} entry scan budget. Select a narrower folder or filename pattern. No checkpoint was saved."))
}

// Filename masks intentionally support only '*' and '?'; source validation and the UI
// describe that contract, rather than implying full regular-expression semantics.
fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let candidate: Vec<char> = candidate.chars().collect();
    let (mut p, mut c, mut star, mut after_star) = (0, 0, None, 0);
    while c < candidate.len() {
        if p < pattern.len()
            && (pattern[p] == '?' || pattern[p].eq_ignore_ascii_case(&candidate[c]))
        {
            p += 1;
            c += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            p += 1;
            after_star = c;
        } else if let Some(previous) = star {
            after_star += 1;
            c = after_star;
            p = previous + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderCursor {
    configuration: String,
    sequence: u64,
    files: BTreeMap<String, FileObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileObservation {
    fingerprint: String,
    first_seen_ms: u64,
    emitted: bool,
}

fn folder_batch(
    cursor: Option<Value>,
    files: Vec<ScannedFile>,
    source: &EventSource,
    now: u64,
    settle_ms: u64,
    capacity: usize,
) -> Result<PollBatch, ReactiveError> {
    let SourceKind::Folder {
        path,
        pattern,
        recursive,
        ..
    } = &source.kind
    else {
        return Err(ReactiveError::Invalid(
            "The folder scan received a different source type.".into(),
        ));
    };
    let configuration = digest(format!("{path}\n{pattern}\n{recursive}").as_bytes());
    let mut previous: FolderCursor = decode_cursor(cursor)?;
    if previous.configuration != configuration {
        previous.files.clear();
    }
    let mut next = FolderCursor {
        configuration,
        sequence: previous.sequence,
        files: BTreeMap::new(),
    };
    let mut events = Vec::new();
    let mut backpressured = false;
    for file in files {
        let mut observation = previous
            .files
            .remove(&file.relative)
            .filter(|old| old.fingerprint == file.fingerprint)
            .unwrap_or(FileObservation {
                fingerprint: file.fingerprint.clone(),
                first_seen_ms: now,
                emitted: false,
            });
        // Empty and growing files do not wake agents with an unfinished document.
        if !observation.emitted
            && file.size > 0
            && now.saturating_sub(observation.first_seen_ms) >= settle_ms
        {
            if events.len() < capacity {
                next.sequence = next.sequence.checked_add(1).ok_or_else(|| {
                    ReactiveError::Invalid(
                        "The folder event sequence is exhausted. Create a new source.".into(),
                    )
                })?;
                let key = format!(
                    "folder:{}:{}",
                    next.sequence,
                    digest(format!("{}:{}", file.relative, file.fingerprint).as_bytes())
                );
                events.push(SourceEmission { key, payload: json!({ "event": "file_changed", "path": file.path, "file": file.relative, "size": file.size, "fingerprint": file.fingerprint }) });
                observation.emitted = true;
            } else {
                backpressured = true;
            }
        }
        next.files.insert(file.relative, observation);
    }
    let cursor = serde_json::to_value(next).map_err(encoding_error)?;
    if serde_json::to_vec(&cursor).map_err(encoding_error)?.len() > MAX_FOLDER_CURSOR_BYTES {
        return Err(folder_budget_error());
    }
    Ok(PollBatch {
        events,
        cursor,
        backpressured,
    })
}

async fn read_http(
    client: &reqwest::Client,
    url: &str,
    timeout_ms: u64,
) -> Result<Vec<u8>, ReactiveError> {
    let mut response = client.get(url).timeout(Duration::from_millis(timeout_ms))
        .send().await.map_err(|_| ReactiveError::Invalid("The watched website could not be reached before its poll deadline. Its checkpoint was kept.".into()))?;
    if !response.status().is_success() {
        return Err(ReactiveError::Invalid(format!(
            "The watched website returned {}. Its checkpoint was kept.",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(MAX_SOURCE_BODY_BYTES).unwrap_or(u64::MAX))
    {
        return Err(body_budget_error("Website response"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        ReactiveError::Invalid(
            "The watched website response ended early. Its checkpoint was kept.".into(),
        )
    })? {
        if body.len().saturating_add(chunk.len()) > MAX_SOURCE_BODY_BYTES {
            return Err(body_budget_error("Website response"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn body_budget_error(label: &str) -> ReactiveError {
    ReactiveError::Invalid(format!("{label} exceeds the {MAX_SOURCE_BODY_BYTES} byte source body budget. Use a smaller endpoint or a command that extracts the relevant content. No checkpoint was saved."))
}

async fn read_limited(
    reader: impl AsyncRead + Unpin,
    label: &str,
) -> Result<Vec<u8>, ReactiveError> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(MAX_SOURCE_BODY_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| source_error("Could not read the source command output", error))?;
    if bytes.len() > MAX_SOURCE_BODY_BYTES {
        return Err(body_budget_error(label));
    }
    Ok(bytes)
}

#[cfg(unix)]
struct CommandGroup(Option<i32>);

#[cfg(unix)]
impl Drop for CommandGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0.filter(|pid| *pid > 0) {
            // SAFETY: this is our child's positive PID, assigned a fresh process group
            // with process_group(0). The negative PID targets only that group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

async fn read_command(
    command: &str,
    cwd: &Path,
    timeout_ms: u64,
) -> Result<Vec<u8>, ReactiveError> {
    #[cfg(unix)]
    let mut process = {
        let mut process = Command::new("/bin/sh");
        process.args(["-c", command]).process_group(0);
        process
    };
    #[cfg(not(unix))]
    let mut process = {
        let mut process = Command::new("cmd.exe");
        process.args(["/C", command]);
        process
    };
    let mut child = process
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| source_error("Could not start the configured source command", error))?;
    #[cfg(unix)]
    let group = CommandGroup(child.id().and_then(|id| i32::try_from(id).ok()));
    let stdout = child.stdout.take().ok_or_else(|| {
        ReactiveError::Invalid("The source command stdout pipe was not created.".into())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ReactiveError::Invalid("The source command stderr pipe was not created.".into())
    })?;
    let result = tokio::time::timeout(Duration::from_millis(timeout_ms), async {
        tokio::try_join!(
            async {
                child
                    .wait()
                    .await
                    .map_err(|error| source_error("Could not wait for the source command", error))
            },
            read_limited(stdout, "Command stdout"),
            read_limited(stderr, "Command stderr"),
        )
    })
    .await;
    #[cfg(unix)]
    drop(group);
    match result {
        Ok(Ok((status, stdout, _stderr))) if status.success() => Ok(stdout),
        Ok(Ok((status, _, _))) => Err(ReactiveError::Invalid(format!(
            "The source command exited with {status}. No checkpoint was saved."
        ))),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            Err(error)
        }
        Err(_) => {
            let _ = child.kill().await;
            Err(ReactiveError::Invalid(format!("The source command exceeded its {timeout_ms} ms timeout and was stopped. No checkpoint was saved.")))
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyCursor {
    configuration: String,
    digest: String,
    sequence: u64,
}

fn body_batch(
    cursor: Option<Value>,
    label: &str,
    configuration: &str,
    body: &[u8],
    metadata: Value,
) -> Result<PollBatch, ReactiveError> {
    let mut cursor: BodyCursor = decode_cursor(cursor)?;
    let configuration = digest(configuration.as_bytes());
    let fingerprint = digest(body);
    let mut events = Vec::new();
    if cursor.configuration != configuration || cursor.digest != fingerprint {
        cursor.sequence = cursor.sequence.checked_add(1).ok_or_else(|| {
            ReactiveError::Invalid(
                "The source event sequence is exhausted. Create a new source.".into(),
            )
        })?;
        if !body.is_empty() {
            let text = std::str::from_utf8(body).map_err(|_| {
                ReactiveError::Invalid(
                    "Source output is not UTF-8 text. Configure a text or JSON endpoint instead."
                        .into(),
                )
            })?;
            let value =
                serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.into()));
            events.push(SourceEmission { key: format!("{label}:{}:{fingerprint}", cursor.sequence), payload: json!({ "event": "content_changed", "value": value, "metadata": metadata, "fingerprint": fingerprint }) });
        }
        cursor.configuration = configuration;
        cursor.digest = fingerprint;
    }
    Ok(PollBatch {
        events,
        cursor: serde_json::to_value(cursor).map_err(encoding_error)?,
        backpressured: false,
    })
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleCursor {
    every_ms: u64,
    next_due_ms: u64,
    sequence: u64,
}

fn schedule_batch(
    cursor: Option<Value>,
    now: u64,
    every_ms: u64,
) -> Result<PollBatch, ReactiveError> {
    if every_ms == 0 {
        return Err(ReactiveError::Invalid(
            "A schedule interval must be greater than zero.".into(),
        ));
    }
    let mut cursor: ScheduleCursor = decode_cursor(cursor)?;
    if cursor.every_ms != every_ms || cursor.next_due_ms == 0 {
        cursor.every_ms = every_ms;
        cursor.next_due_ms = now.checked_add(every_ms).ok_or_else(|| {
            ReactiveError::Invalid(
                "The schedule interval exceeds the supported timestamp range.".into(),
            )
        })?;
    }
    let mut events = Vec::new();
    if now >= cursor.next_due_ms {
        let scheduled_at_ms = cursor.next_due_ms;
        cursor.sequence = cursor.sequence.checked_add(1).ok_or_else(|| {
            ReactiveError::Invalid(
                "The schedule event sequence is exhausted. Create a new source.".into(),
            )
        })?;
        let missed = now.saturating_sub(scheduled_at_ms) / every_ms;
        let advance = missed
            .checked_add(1)
            .and_then(|slots| slots.checked_mul(every_ms))
            .and_then(|duration| scheduled_at_ms.checked_add(duration))
            .ok_or_else(|| {
                ReactiveError::Invalid("The schedule exceeds the supported timestamp range.".into())
            })?;
        cursor.next_due_ms = advance;
        // Exactly one catch-up notification, with the skipped count visible to the agent.
        events.push(SourceEmission { key: format!("schedule:{}:{scheduled_at_ms}", cursor.sequence), payload: json!({ "event": "schedule_due", "scheduled_at_ms": scheduled_at_ms, "observed_at_ms": now, "missed_intervals": missed }) });
    }
    Ok(PollBatch {
        events,
        cursor: serde_json::to_value(cursor).map_err(encoding_error)?,
        backpressured: false,
    })
}

#[cfg(test)]
mod tests;
