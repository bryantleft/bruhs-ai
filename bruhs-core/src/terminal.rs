//! Terminal sessions that outlive their clients.
//!
//! [`TerminalManager`] owns every PTY the app spawns. A client *attaches* to
//! a session to get a replay of recent output plus a live stream, and
//! *detaches* by dropping the receiver; neither affects the shell. A session
//! ends when the shell exits (the manager drops it and emits
//! [`TerminalEvent::Exited`]) or when a client calls [`TerminalManager::close`].
//!
//! The Axum server exposes this over WebSockets and the desktop app drives
//! it in-process, so a browser reload or a view switch never kills a shell.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

/// Bytes of output kept per session for replay on attach.
pub const SCROLLBACK_CAPACITY: usize = 512 * 1024;
pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 32;
const READ_CHUNK: usize = 4096;
const OUTPUT_CHANNEL_CAPACITY: usize = 4096;
const EVENT_CHANNEL_CAPACITY: usize = 64;
/// Input is queued to each session's writer thread in chunks of at most
/// this many bytes...
pub const INPUT_CHUNK_BYTES: usize = 16 * 1024;
/// ...and at most this many chunks (4 MiB) may wait for the PTY.
pub const INPUT_QUEUE_CHUNKS: usize = 256;

/// Public description of a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub id: String,
    /// `None` = the home directory.
    pub cwd: Option<String>,
    pub title: String,
    pub project_name: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub created_at_ms: u64,
}

/// What to spawn: always the user's login shell (`$SHELL -l`, see
/// [`pick_shell`]). Everything is optional; the default is the home
/// directory at 120×32. Session ids are always generated.
#[derive(Clone, Debug, Default)]
pub struct TerminalSpec {
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    pub project_name: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    Created(TerminalInfo),
    /// The shell exited on its own; the session is gone.
    Exited {
        id: String,
    },
    /// A client closed the session.
    Closed {
        id: String,
    },
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TerminalError {
    #[error("no such terminal session: {0}")]
    NotFound(String),
    #[error("working directory does not exist: {0}")]
    MissingCwd(String),
    #[error("pty: {0}")]
    Pty(#[source] anyhow::Error),
    #[error("spawn: {0}")]
    Spawn(#[source] anyhow::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The session's input queue is full (the program is not reading); the
    /// bytes were not queued.
    #[error("terminal {0} is not accepting input fast enough")]
    InputBacklogFull(String),
    /// The session's writer has stopped (the PTY closed).
    #[error("terminal {0} no longer accepts input")]
    InputClosed(String),
    #[error("terminal session id already in use: {0}")]
    DuplicateId(String),
}

/// Replay + live stream handed to an attaching client. Dropping `output`
/// detaches without touching the shell.
pub struct TerminalAttachment {
    pub info: TerminalInfo,
    pub replay: Vec<u8>,
    pub output: broadcast::Receiver<Arc<[u8]>>,
}

/// Fixed-capacity byte ring that keeps the most recent output.
#[derive(Debug)]
pub struct Scrollback {
    buf: VecDeque<u8>,
    capacity: usize,
}

impl Scrollback {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity.min(READ_CHUNK * 16)),
            capacity,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.capacity {
            self.buf.clear();
            self.buf.extend(&bytes[bytes.len() - self.capacity..]);
            return;
        }
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.capacity);
        if overflow > 0 {
            self.buf.drain(..overflow);
        }
        self.buf.extend(bytes);
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// `$SHELL` when it exists, else zsh, else bash, else sh.
pub fn pick_shell(shell_env: Option<&str>) -> PathBuf {
    if let Some(s) = shell_env.filter(|s| !s.is_empty()) {
        let p = Path::new(s);
        if p.is_file() {
            return p.to_path_buf();
        }
    }
    for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if Path::new(candidate).is_file() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from("/bin/sh")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Lock `mutex`, recovering the data if a thread panicked while holding it
/// (every critical section here leaves the data consistent).
fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type PtyChild = Box<dyn Child + Send + Sync>;

/// Kill `child` (SIGHUP, a short grace period, then SIGKILL) and reap it on
/// a short-lived thread, so callers on async paths or the UI thread never
/// wait for the process.
fn reap_in_background(mut child: PtyChild) {
    let mut killer = child.clone_killer();
    let spawned = std::thread::Builder::new()
        .name("bruhs-pty-reap".to_string())
        .spawn(move || {
            if let Err(e) = child.kill() {
                tracing::debug!("terminal child already gone: {e}");
            }
            if let Err(e) = child.wait() {
                tracing::debug!("terminal child wait failed: {e}");
            }
        });
    if let Err(e) = spawned {
        // No thread to wait on it: at least signal it to exit.
        tracing::warn!("could not start terminal reaper thread: {e}");
        if let Err(e) = killer.kill() {
            tracing::debug!("terminal child already gone: {e}");
        }
    }
}

/// Feed queued input to the PTY until the queue closes or a write fails.
fn spawn_writer(
    name: String,
    mut writer: Box<dyn Write + Send>,
    mut input: mpsc::Receiver<Vec<u8>>,
) -> std::io::Result<()> {
    std::thread::Builder::new().name(name).spawn(move || {
        while let Some(chunk) = input.blocking_recv() {
            if let Err(e) = writer.write_all(&chunk).and_then(|()| writer.flush()) {
                tracing::debug!("terminal input writer stopped: {e}");
                break;
            }
        }
    })?;
    Ok(())
}

struct Session {
    info: TerminalInfo,
    master: Mutex<Box<dyn MasterPty + Send>>,
    /// Queue to the dedicated writer thread; PTY writes never block callers.
    input: mpsc::Sender<Vec<u8>>,
    /// Taken by whoever reaps it: the reader thread after EOF, or `close`.
    child: Mutex<Option<PtyChild>>,
    /// Scrollback and the broadcast are updated under this one lock so an
    /// attaching client sees each chunk exactly once (replay xor stream).
    scrollback: Mutex<Scrollback>,
    output: broadcast::Sender<Arc<[u8]>>,
    /// Set by `close` so the reader thread does not also report `Exited`.
    closing: AtomicBool,
}

struct Inner {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    events: broadcast::Sender<TerminalEvent>,
}

/// Cheap-to-clone handle to the process-wide session table.
#[derive(Clone)]
pub struct TerminalManager {
    inner: Arc<Inner>,
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalManager {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                sessions: Mutex::new(HashMap::new()),
                events,
            }),
        }
    }

    /// Spawn a shell and start streaming its output.
    pub fn create(&self, spec: TerminalSpec) -> Result<TerminalInfo, TerminalError> {
        self.create_with_program(spec, None)
    }

    /// `create`, running `program` (program + args) instead of the login
    /// shell. Tests use it for a predictable `/bin/sh`.
    fn create_with_program(
        &self,
        spec: TerminalSpec,
        program: Option<&[String]>,
    ) -> Result<TerminalInfo, TerminalError> {
        let id = uuid::Uuid::new_v4().to_string();
        let short_id: String = id.chars().take(8).collect();
        let cwd = match spec.cwd {
            Some(p) => p,
            None => home_dir().ok_or_else(|| TerminalError::MissingCwd("$HOME".to_string()))?,
        };
        if !cwd.is_dir() {
            return Err(TerminalError::MissingCwd(cwd.display().to_string()));
        }
        let cols = spec.cols.filter(|c| *c > 0).unwrap_or(DEFAULT_COLS);
        let rows = spec.rows.filter(|r| *r > 0).unwrap_or(DEFAULT_ROWS);

        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(TerminalError::Pty)?;

        let mut cmd = match program {
            Some([program, args @ ..]) => {
                let mut c = CommandBuilder::new(program);
                c.args(args);
                c
            }
            _ => {
                let shell = pick_shell(std::env::var("SHELL").ok().as_deref());
                let mut c = CommandBuilder::new(&shell);
                c.arg("-l");
                c
            }
        };
        cmd.cwd(&cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("BRUHS_TERMINAL", "1");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(TerminalError::Spawn)?;
        drop(pair.slave);
        let master = pair.master;
        let pipes = master
            .try_clone_reader()
            .and_then(|reader| Ok((reader, master.take_writer()?)));
        let (mut reader, writer) = match pipes {
            Ok(pipes) => pipes,
            Err(e) => {
                reap_in_background(child);
                return Err(TerminalError::Pty(e));
            }
        };

        let (input, input_rx) = mpsc::channel(INPUT_QUEUE_CHUNKS);
        if let Err(e) = spawn_writer(format!("bruhs-pty-in-{short_id}"), writer, input_rx) {
            reap_in_background(child);
            return Err(e.into());
        }

        let cwd_string = cwd.display().to_string();
        let is_home = home_dir().is_some_and(|h| h == cwd);
        let info = TerminalInfo {
            id: id.clone(),
            cwd: (!is_home).then_some(cwd_string.clone()),
            title: spec
                .title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| {
                    bruhs_models::logic::terminal::terminal_title(
                        (!is_home).then_some(cwd_string.as_str()),
                        spec.project_name.as_deref(),
                    )
                }),
            project_name: spec.project_name,
            cols,
            rows,
            created_at_ms: now_ms(),
        };

        let (output, _) = broadcast::channel(OUTPUT_CHANNEL_CAPACITY);
        let session = Arc::new(Session {
            info: info.clone(),
            master: Mutex::new(master),
            input,
            child: Mutex::new(Some(child)),
            scrollback: Mutex::new(Scrollback::new(SCROLLBACK_CAPACITY)),
            output,
            closing: AtomicBool::new(false),
        });

        // The reader only reports the exit once the session is registered;
        // if creation fails after this point the sender is dropped and the
        // reader just stops.
        let (registered_tx, registered_rx) = std::sync::mpsc::channel::<()>();
        let weak_inner: Weak<Inner> = Arc::downgrade(&self.inner);
        let reader_session = Arc::clone(&session);
        let reader_started = std::thread::Builder::new()
            .name(format!("bruhs-pty-{short_id}"))
            .spawn(move || {
                let mut buf = [0u8; READ_CHUNK];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk: Arc<[u8]> = Arc::from(&buf[..n]);
                            let mut sb = lock(&reader_session.scrollback);
                            sb.push(&chunk);
                            // No attached clients is fine: the scrollback
                            // keeps the output for the next attach.
                            let _ = reader_session.output.send(chunk);
                            // Held across the send so `attach` (which takes
                            // this lock to snapshot and subscribe) never
                            // misses or repeats a chunk.
                            drop(sb);
                        }
                    }
                }
                let child = lock(&reader_session.child).take();
                if let Some(mut child) = child {
                    if let Err(e) = child.wait() {
                        tracing::debug!("terminal child wait failed: {e}");
                    }
                }
                if registered_rx.recv().is_err() {
                    return; // creation was abandoned
                }
                if reader_session.closing.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(inner) = weak_inner.upgrade() {
                    let removed = {
                        let mut table = lock(&inner.sessions);
                        match table.get(&reader_session.info.id) {
                            Some(current) if Arc::ptr_eq(current, &reader_session) => {
                                table.remove(&reader_session.info.id).is_some()
                            }
                            _ => false,
                        }
                    };
                    if removed {
                        // Status only: nobody may be subscribed.
                        let _ = inner.events.send(TerminalEvent::Exited {
                            id: reader_session.info.id.clone(),
                        });
                    }
                }
            });
        if let Err(e) = reader_started {
            if let Some(child) = lock(&session.child).take() {
                reap_in_background(child);
            }
            return Err(e.into());
        }

        match lock(&self.inner.sessions).entry(id.clone()) {
            Entry::Occupied(_) => {
                if let Some(child) = lock(&session.child).take() {
                    reap_in_background(child);
                }
                return Err(TerminalError::DuplicateId(id));
            }
            Entry::Vacant(slot) => {
                slot.insert(Arc::clone(&session));
            }
        }
        // Status only: nobody may be subscribed.
        let _ = self.inner.events.send(TerminalEvent::Created(info.clone()));
        // The reader holds the receiver until it exits; a send error means
        // it already gave up, which only happens on abandoned creation.
        let _ = registered_tx.send(());

        Ok(info)
    }

    fn session(&self, id: &str) -> Result<Arc<Session>, TerminalError> {
        lock(&self.inner.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| TerminalError::NotFound(id.to_string()))
    }

    /// Live sessions, oldest first.
    pub fn list(&self) -> Vec<TerminalInfo> {
        let mut out: Vec<TerminalInfo> = lock(&self.inner.sessions)
            .values()
            .map(|s| s.info.clone())
            .collect();
        out.sort_by(|a, b| a.created_at_ms.cmp(&b.created_at_ms).then(a.id.cmp(&b.id)));
        out
    }

    pub fn get(&self, id: &str) -> Option<TerminalInfo> {
        self.session(id).ok().map(|s| s.info.clone())
    }

    /// Replay recent output and subscribe to the rest.
    pub fn attach(&self, id: &str) -> Result<TerminalAttachment, TerminalError> {
        let session = self.session(id)?;
        let sb = lock(&session.scrollback);
        let output = session.output.subscribe();
        let replay = sb.snapshot();
        drop(sb);
        Ok(TerminalAttachment {
            info: session.info.clone(),
            replay,
            output,
        })
    }

    /// Queue `bytes` for the shell without blocking. Either all of it is
    /// queued or none: `InputBacklogFull` when the program has stopped
    /// reading and the queue is full, `InputClosed` once the PTY is gone.
    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<(), TerminalError> {
        let session = self.session(id)?;
        let chunks: Vec<&[u8]> = bytes.chunks(INPUT_CHUNK_BYTES).collect();
        if chunks.is_empty() {
            return Ok(());
        }
        let permits = session
            .input
            .try_reserve_many(chunks.len())
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(()) => {
                    TerminalError::InputBacklogFull(id.to_string())
                }
                mpsc::error::TrySendError::Closed(()) => TerminalError::InputClosed(id.to_string()),
            })?;
        for (permit, chunk) in permits.zip(chunks) {
            permit.send(chunk.to_vec());
        }
        Ok(())
    }

    /// Queue `bytes` for the shell, waiting (asynchronously) for room in the
    /// queue instead of failing when it is full.
    pub async fn write_async(&self, id: &str, bytes: &[u8]) -> Result<(), TerminalError> {
        let input = self.session(id)?.input.clone();
        let closed = || TerminalError::InputClosed(id.to_string());
        let chunks: Vec<&[u8]> = bytes.chunks(INPUT_CHUNK_BYTES).collect();
        if chunks.len() <= INPUT_QUEUE_CHUNKS {
            // Reserve all slots first so concurrent writers never interleave.
            let permits = input
                .reserve_many(chunks.len())
                .await
                .map_err(|_| closed())?;
            for (permit, chunk) in permits.zip(chunks) {
                permit.send(chunk.to_vec());
            }
        } else {
            for chunk in chunks {
                input.send(chunk.to_vec()).await.map_err(|_| closed())?;
            }
        }
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), TerminalError> {
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        let session = self.session(id)?;
        let master = lock(&session.master);
        let result = master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(TerminalError::Pty);
        drop(master);
        result
    }

    /// Forget the session and stop the shell. `false` if it was not there.
    /// Returns immediately: the shell is killed and reaped on a background
    /// thread.
    pub fn close(&self, id: &str) -> bool {
        let removed = lock(&self.inner.sessions).remove(id);
        let Some(session) = removed else {
            return false;
        };
        session.closing.store(true, Ordering::SeqCst);
        if let Some(child) = lock(&session.child).take() {
            reap_in_background(child);
        }
        // Status only: nobody may be subscribed.
        let _ = self
            .inner
            .events
            .send(TerminalEvent::Closed { id: id.to_string() });
        true
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.inner.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(mut check: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn drain_events(rx: &mut broadcast::Receiver<TerminalEvent>) -> Vec<TerminalEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    fn sh_spec(dir: &Path) -> TerminalSpec {
        TerminalSpec {
            cwd: Some(dir.to_path_buf()),
            cols: Some(80),
            rows: Some(24),
            ..TerminalSpec::default()
        }
    }

    /// Spawn `argv` (instead of the user's login shell) in a temp dir.
    fn spawn(manager: &TerminalManager, argv: &[&str]) -> TerminalInfo {
        let argv: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        manager
            .create_with_program(sh_spec(&std::env::temp_dir()), Some(&argv))
            .expect("spawn")
    }

    #[test]
    fn scrollback_keeps_the_tail() {
        let mut sb = Scrollback::new(8);
        sb.push(b"abcdef");
        sb.push(b"ghij");
        assert_eq!(sb.snapshot(), b"cdefghij");
        sb.push(b"0123456789abcdefXYZ");
        assert_eq!(sb.snapshot(), b"bcdefXYZ");
        assert_eq!(sb.len(), 8);
        assert!(!sb.is_empty());
    }

    #[test]
    fn pick_shell_falls_back_to_an_existing_binary() {
        let picked = pick_shell(Some("/definitely/not/a/shell"));
        assert!(picked.is_file(), "{picked:?} should exist");
        assert_eq!(pick_shell(Some("/bin/sh")), PathBuf::from("/bin/sh"));
    }

    #[test]
    fn missing_cwd_is_an_error() {
        let manager = TerminalManager::new();
        let err = manager
            .create(TerminalSpec {
                cwd: Some(PathBuf::from("/definitely/not/here")),
                ..TerminalSpec::default()
            })
            .unwrap_err();
        assert!(matches!(err, TerminalError::MissingCwd(_)), "{err}");
        assert!(manager.list().is_empty());
    }

    #[test]
    fn output_survives_detach_and_replays_on_attach() {
        let dir = std::env::temp_dir();
        let manager = TerminalManager::new();
        let mut events = manager.subscribe();
        let info = manager
            .create_with_program(sh_spec(&dir), Some(&["/bin/sh".to_string()]))
            .expect("spawn sh");
        assert_eq!(info.cols, 80);
        assert_eq!(manager.list().len(), 1);
        assert!(matches!(
            drain_events(&mut events).as_slice(),
            [TerminalEvent::Created(_)]
        ));

        // A first client attaches, writes a command, and walks away.
        let first = manager.attach(&info.id).expect("attach");
        drop(first);
        manager
            .write(&info.id, b"echo bruhs-$((40+2))\n")
            .expect("write");

        // Later, a fresh client gets the earlier output back as replay.
        let mut seen = String::new();
        wait_for(
            || {
                let att = manager.attach(&info.id).expect("attach again");
                seen = String::from_utf8_lossy(&att.replay).into_owned();
                seen.contains("bruhs-42")
            },
            "echo output in the replay",
        );

        // Streaming continues after the replay.
        let mut live = manager.attach(&info.id).expect("attach live");
        manager.write(&info.id, b"echo more-$((1+1))\n").unwrap();
        let mut streamed = Vec::new();
        wait_for(
            || {
                while let Ok(chunk) = live.output.try_recv() {
                    streamed.extend_from_slice(&chunk);
                }
                String::from_utf8_lossy(&streamed).contains("more-2")
            },
            "live output",
        );

        // Resize is accepted; the shell exiting removes the session.
        manager.resize(&info.id, 100, 30).expect("resize");
        manager.write(&info.id, b"exit\n").unwrap();
        wait_for(|| manager.list().is_empty(), "session to go away");
        wait_for(
            || {
                drain_events(&mut events)
                    .iter()
                    .any(|e| matches!(e, TerminalEvent::Exited { id } if *id == info.id))
            },
            "Exited event",
        );
        assert!(matches!(
            manager.attach(&info.id),
            Err(TerminalError::NotFound(_))
        ));
    }

    #[test]
    fn close_kills_and_forgets() {
        let manager = TerminalManager::new();
        let mut events = manager.subscribe();
        let info = spawn(&manager, &["/bin/sh"]);
        assert!(manager.get(&info.id).is_some());
        assert!(manager.close(&info.id));
        assert!(!manager.close(&info.id));
        assert!(manager.list().is_empty());
        wait_for(
            || {
                drain_events(&mut events)
                    .iter()
                    .any(|e| matches!(e, TerminalEvent::Closed { id } if *id == info.id))
            },
            "Closed event",
        );
        // The reader thread must not also report Exited for a closed session.
        std::thread::sleep(Duration::from_millis(200));
        assert!(!drain_events(&mut events)
            .iter()
            .any(|e| matches!(e, TerminalEvent::Exited { .. })));
        assert!(matches!(
            manager.write(&info.id, b"x"),
            Err(TerminalError::NotFound(_))
        ));
    }

    /// A shell that is not reading its input used to block `write` (and the
    /// tokio worker or UI thread calling it) once the PTY buffer filled.
    #[test]
    fn write_never_blocks_when_the_program_is_not_reading() {
        let manager = TerminalManager::new();
        // In raw mode with no reader the PTY input queue fills and master
        // writes block (canonical mode may discard overflow instead).
        let info = spawn(&manager, &["/bin/sh", "-c", "stty raw -echo; sleep 60"]);
        let chunk = vec![b'x'; INPUT_CHUNK_BYTES];
        let started = Instant::now();
        let mut full = false;
        // Far more than the PTY buffer plus the whole input queue.
        for _ in 0..(INPUT_QUEUE_CHUNKS * 4) {
            match manager.write(&info.id, &chunk) {
                Ok(()) => {}
                Err(TerminalError::InputBacklogFull(id)) => {
                    assert_eq!(id, info.id);
                    full = true;
                    break;
                }
                Err(other) => panic!("unexpected error: {other}"),
            }
        }
        assert!(full, "the input queue never reported back-pressure");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "writes blocked for {:?}",
            started.elapsed()
        );
        // All-or-nothing: an oversized write is refused whole.
        assert!(matches!(
            manager.write(
                &info.id,
                &vec![b'y'; INPUT_CHUNK_BYTES * (INPUT_QUEUE_CHUNKS + 1)]
            ),
            Err(TerminalError::InputBacklogFull(_))
        ));
        assert!(manager.close(&info.id));
    }

    #[test]
    fn write_async_reaches_the_shell() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let manager = TerminalManager::new();
        let info = spawn(&manager, &["/bin/sh"]);
        let mut live = manager.attach(&info.id).unwrap();
        runtime
            .block_on(manager.write_async(&info.id, b"echo async-$((2+3))\n"))
            .unwrap();
        let mut streamed = Vec::new();
        wait_for(
            || {
                while let Ok(chunk) = live.output.try_recv() {
                    streamed.extend_from_slice(&chunk);
                }
                String::from_utf8_lossy(&streamed).contains("async-5")
            },
            "async write echo",
        );
        assert!(manager.close(&info.id));
        assert!(matches!(
            runtime.block_on(manager.write_async(&info.id, b"x")),
            Err(TerminalError::NotFound(_))
        ));
    }

    /// `close` used to SIGHUP, sleep through portable-pty's grace period,
    /// SIGKILL and `wait()` inline, on async paths and the UI thread.
    #[test]
    fn close_does_not_wait_for_a_stubborn_child() {
        let manager = TerminalManager::new();
        let info = spawn(
            &manager,
            &["/bin/sh", "-c", "trap '' HUP; while :; do sleep 1; done"],
        );
        // Let the trap install before closing.
        std::thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        assert!(manager.close(&info.id));
        let took = started.elapsed();
        assert!(took < Duration::from_millis(150), "close took {took:?}");
        assert!(manager.get(&info.id).is_none());
    }

    #[test]
    fn generated_ids_are_unique() {
        // Ids are generated; two sessions never share one.
        let manager = TerminalManager::new();
        let a = spawn(&manager, &["/bin/sh"]);
        let b = spawn(&manager, &["/bin/sh"]);
        assert_ne!(a.id, b.id);
        assert_eq!(manager.list().len(), 2);
        assert!(manager.close(&a.id));
        assert!(manager.close(&b.id));
    }
}
