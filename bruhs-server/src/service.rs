//! Run bruhs-server as a login service (launchd on macOS, a systemd user unit
//! on Linux) so reactive agents keep working after the app quits.
//!
//! The service starts this same binary with the `BRUHS_*` settings and `PATH`
//! of the process that installed it (agents need `PATH` to find their CLIs).
//! It restarts only after a failure: while the app's own server holds the
//! port and the automation database, the service's start fails and is retried
//! every 30 seconds, so it takes over once the app quits.
//!
//! The service never runs the installing binary in place. Install copies the
//! binary and the web bundle into a verified release under the app data
//! directory (`service/releases/<id>`) and the definition points there, so a
//! build that lives in a folder a background job may not read, or that later
//! moves or is rebuilt, does not decide whether the service starts. Releases
//! are owned by a manifest; upgrade keeps the previous release for rollback and
//! uninstall removes only owned releases, never logs or other app data.

use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
#[cfg(unix)]
pub(crate) use restore::{start_after_restore, stop_for_restore};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
mod restore;

const LABEL: &str = "dev.bruhs.server";
const UNIT: &str = "bruhs-server.service";
/// Settings that describe this process's own lifetime, not the service's,
/// the acceptance script's in-memory secrets (a service installed from such
/// a server would start with an empty vault and keep nothing), and a debug
/// build's test app upstream (D24).
const SKIPPED_ENV: [&str; 3] = [
    "BRUHS_EXIT_WITH_PARENT",
    "BRUHS_SECRET_STORE",
    bruhs_core::apps::APPS_UPSTREAM_ENV,
];

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Failed(String),
}

fn home() -> Result<PathBuf, ServiceError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| ServiceError::Failed("HOME is not set.".into()))
}

fn definition_path() -> Result<PathBuf, ServiceError> {
    if cfg!(target_os = "macos") {
        Ok(home()?
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    } else if cfg!(target_os = "linux") {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map_or_else(|| home().map(|home| home.join(".config")), Ok)?;
        Ok(config.join("systemd/user").join(UNIT))
    } else {
        Err(ServiceError::Unsupported(
            "Background service install supports macOS and Linux.".into(),
        ))
    }
}

/// Whether the installer copies variable `key` into the service.
fn carried(key: &str) -> bool {
    (key.starts_with("BRUHS_") && !SKIPPED_ENV.contains(&key)) || key == "PATH"
}

/// Environment carried into the service, sorted for a stable definition.
fn environment(web_dist: &Path) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().filter(|(key, _)| carried(key)).collect();
    env.retain(|(key, _)| key != "BRUHS_WEB_DIST");
    env.push(("BRUHS_WEB_DIST".into(), web_dist.display().to_string()));
    env.sort();
    env
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn launchd_plist(binary: &Path, env: &[(String, String)], log_dir: &Path) -> String {
    let variables = env
        .iter()
        .map(|(key, value)| {
            format!(
                "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                xml(key),
                xml(value)
            )
        })
        .collect::<String>();
    let log = |name: &str| xml(&log_dir.join(name).display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{LABEL}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n{variables}\t</dict>\n\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\t<key>ThrottleInterval</key>\n\t<integer>30</integer>\n\t<key>ProcessType</key>\n\t<string>Background</string>\n\t<key>StandardOutPath</key>\n\t<string>{}</string>\n\t<key>StandardErrorPath</key>\n\t<string>{}</string>\n</dict>\n</plist>\n",
        xml(&binary.display().to_string()),
        log("bruhs-server.service.log"),
        log("bruhs-server.service.log"),
    )
}

fn systemd_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

pub(crate) fn systemd_unit(binary: &Path, env: &[(String, String)]) -> String {
    let variables = env
        .iter()
        .map(|(key, value)| format!("Environment={}\n", systemd_quote(&format!("{key}={value}"))))
        .collect::<String>();
    format!(
        "[Unit]\nDescription=bruhs local server and automations\nAfter=network-online.target\n\n[Service]\nExecStart={}\n{variables}Restart=on-failure\nRestartSec=30\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(&binary.display().to_string())
    )
}

fn run(program: &str, args: &[&str]) -> Result<String, ServiceError> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| ServiceError::Failed(format!("Could not run {program}: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(ServiceError::Failed(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn gui_domain() -> Result<String, ServiceError> {
    Ok(format!("gui/{}", run("id", &["-u"])?))
}

fn log_dir() -> Result<PathBuf, ServiceError> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_local_dir().join("logs"))
        .ok_or_else(|| ServiceError::Failed("Could not find the app data directory.".into()))
}

// ── Service releases ───────────────────────────────────────────────────────

/// Marks a release directory as written by `service install`.
const RELEASE_MANIFEST: &str = "bruhs-service-release.json";
/// Marks an in-progress or abandoned staging directory as ours.
const STAGING_MARKER: &str = "bruhs-service-staging.json";
const RELEASE_BINARY: &str = "bruhs-server";
const RELEASE_WEB: &str = "web";
/// Written into every marker; a directory without it is never ours.
const OWNER: &str = "bruhs-server service install";
/// How long install/uninstall waits for another one to finish.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Where the service's own copies live, outside any user folder.
fn service_home() -> Result<PathBuf, ServiceError> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_local_dir().join("service"))
        .ok_or_else(|| ServiceError::Failed("Could not find the app data directory.".into()))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ReleaseManifest {
    owner: String,
    binary_sha256: String,
    /// Web bundle files by relative path, with their sha256.
    web: BTreeMap<String, String>,
    /// Where the binary was copied from, for the record only.
    installed_from: String,
    /// The crew protocol this release speaks; 0 for a release older than the field.
    #[serde(default)]
    reactive_protocol: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StagingMarker {
    owner: String,
    pid: u32,
    started_ns: u128,
}

/// A complete, verified release the definition can point at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Release {
    pub dir: PathBuf,
    pub binary: PathBuf,
    pub web: PathBuf,
}

fn io_failed(what: &str, path: &Path, error: impl std::fmt::Display) -> ServiceError {
    ServiceError::Failed(format!("Could not {what} {}: {error}", path.display()))
}

/// The entry itself, never what a symbolic link points at.
fn entry_kind(path: &Path) -> Option<std::fs::FileType> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| meta.file_type())
}
fn is_real_dir(path: &Path) -> bool {
    entry_kind(path).is_some_and(|kind| kind.is_dir())
}
fn is_real_file(path: &Path) -> bool {
    entry_kind(path).is_some_and(|kind| kind.is_file())
}
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 == 0o111)
}
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    is_real_file(path)
}

/// Flush a directory's entries (new names, renames) to disk.
fn sync_dir(dir: &Path) -> Result<(), ServiceError> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)
            .and_then(|handle| handle.sync_all())
            .map_err(|e| io_failed("flush", dir, e))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

fn sha256_file(path: &Path) -> Result<String, ServiceError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| io_failed("read", path, e))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| io_failed("read", path, e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Every regular file under `root` by relative path. `root` itself must be a
/// real directory (or absent, which reads as empty); symbolic links anywhere
/// inside are refused so a release never points back at its source.
fn hash_tree(root: &Path) -> Result<BTreeMap<String, String>, ServiceError> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), ServiceError> {
        let entries = std::fs::read_dir(dir).map_err(|e| io_failed("list", dir, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_failed("list", dir, e))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| io_failed("inspect", &path, e))?;
            if kind.is_symlink() {
                return Err(ServiceError::Failed(format!(
                    "The web bundle contains a symbolic link ({}); install needs plain files.",
                    path.display()
                )));
            } else if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| io_failed("inspect", &path, e))?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, sha256_file(&path)?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    match entry_kind(root) {
        None => {}
        Some(kind) if kind.is_dir() => walk(root, root, &mut out)?,
        Some(_) => {
            return Err(ServiceError::Failed(format!(
                "The web bundle at {} is not a plain directory.",
                root.display()
            )))
        }
    }
    Ok(out)
}

/// Content address of a release: the binary and every web file.
fn release_id(binary_sha256: &str, web: &BTreeMap<String, String>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(binary_sha256.as_bytes());
    for (path, sha) in web {
        hasher.update(b"\0");
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(sha.as_bytes());
    }
    hasher.finalize()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Our manifest, read only from a real directory and a real file.
fn read_manifest(dir: &Path) -> Option<ReleaseManifest> {
    let file = dir.join(RELEASE_MANIFEST);
    if !is_real_dir(dir) || !is_real_file(&file) {
        return None;
    }
    let manifest: ReleaseManifest = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    (manifest.owner == OWNER).then_some(manifest)
}

fn read_staging_marker(dir: &Path) -> Option<StagingMarker> {
    let file = dir.join(STAGING_MARKER);
    if !is_real_dir(dir) || !is_real_file(&file) {
        return None;
    }
    let marker: StagingMarker = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    (marker.owner == OWNER).then_some(marker)
}

/// A release is usable only if its boundary is made of real entries (no
/// symbolic links at the directory, binary, manifest or web root), the binary
/// is executable, and every byte matches the manifest.
fn verify_release(dir: &Path, expected: &ReleaseManifest) -> bool {
    let binary = dir.join(RELEASE_BINARY);
    let web = dir.join(RELEASE_WEB);
    is_real_dir(dir)
        && is_real_file(&binary)
        && is_executable(&binary)
        && is_real_dir(&web)
        && read_manifest(dir).as_ref() == Some(expected)
        && sha256_file(&binary).ok().as_deref() == Some(expected.binary_sha256.as_str())
        && hash_tree(&web).ok().as_ref() == Some(&expected.web)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), ServiceError> {
    std::fs::create_dir_all(to).map_err(|e| io_failed("create", to, e))?;
    if is_real_dir(from) {
        for entry in std::fs::read_dir(from).map_err(|e| io_failed("list", from, e))? {
            let entry = entry.map_err(|e| io_failed("list", from, e))?;
            let (source, target) = (entry.path(), to.join(entry.file_name()));
            let kind = entry
                .file_type()
                .map_err(|e| io_failed("inspect", &source, e))?;
            if kind.is_dir() {
                copy_tree(&source, &target)?;
            } else if kind.is_file() {
                copy_synced(&source, &target)?;
            }
        }
    }
    sync_dir(to)
}

/// A real copy (never a link), flushed to disk.
fn copy_synced(from: &Path, to: &Path) -> Result<(), ServiceError> {
    std::fs::copy(from, to).map_err(|e| io_failed("copy", from, e))?;
    std::fs::File::open(to)
        .and_then(|file| file.sync_all())
        .map_err(|e| io_failed("flush", to, e))
}

/// Write a file durably: flushed before it is visible under its name.
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ServiceError> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).map_err(|e| io_failed("write", path, e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_failed("write", path, e))
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

/// Whether a process with this id still exists (signal 0 delivers nothing).
fn process_alive(pid: u32) -> bool {
    pid == std::process::id()
        || Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|output| output.status.success())
}

/// Serialises install, uninstall and pruning for one service home: held from
/// before the first change until the last one, released on drop.
pub(crate) struct HomeLock {
    _file: std::fs::File,
}

pub(crate) fn lock_home(home: &Path, wait: std::time::Duration) -> Result<HomeLock, ServiceError> {
    std::fs::create_dir_all(home).map_err(|e| io_failed("create", home, e))?;
    let path = home.join(".lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| io_failed("open", &path, e))?;
    let deadline = std::time::Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(HomeLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(ServiceError::Failed(
                    "Another service install or uninstall is running. Try again when it finishes."
                        .into(),
                ))
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(io_failed("lock", &path, error))
            }
        }
    }
}

/// Copy `binary` and `web_dist` into a verified release under `home`, reusing
/// an identical owned release. Must be called with the home lock held.
///
/// Work happens in a marked staging directory that is renamed into place only
/// after every byte is verified and flushed. A directory at the release's name
/// that is not ours (no owned manifest, or not a real directory) is left
/// byte-for-byte untouched and the install stops.
pub(crate) fn stage_release(
    _lock: &HomeLock,
    home: &Path,
    binary: &Path,
    web_dist: &Path,
) -> Result<Release, ServiceError> {
    if !is_real_file(binary) {
        return Err(ServiceError::Failed(format!(
            "The server binary at {} is not a plain file.",
            binary.display()
        )));
    }
    let manifest = ReleaseManifest {
        owner: OWNER.into(),
        binary_sha256: sha256_file(binary)?,
        web: hash_tree(web_dist)?,
        installed_from: binary.display().to_string(),
        reactive_protocol: bruhs_models::crew::CREW_PROTOCOL,
    };
    let releases = home.join("releases");
    std::fs::create_dir_all(&releases).map_err(|e| io_failed("create", &releases, e))?;
    if !is_real_dir(&releases) {
        return Err(ServiceError::Failed(format!(
            "{} is not a plain directory; nothing was installed.",
            releases.display()
        )));
    }
    let id = release_id(&manifest.binary_sha256, &manifest.web);
    let dir = releases.join(&id);
    let release = Release {
        binary: dir.join(RELEASE_BINARY),
        web: dir.join(RELEASE_WEB),
        dir: dir.clone(),
    };
    let existing = entry_kind(&dir).is_some();
    let owned = read_manifest(&dir);
    if let Some(found) = &owned {
        let same = found.binary_sha256 == manifest.binary_sha256 && found.web == manifest.web;
        if same && verify_release(&dir, found) {
            return Ok(release);
        }
    } else if existing {
        return Err(ServiceError::Failed(format!(
            "{} exists but was not created by service install; it was left untouched and nothing was installed.",
            dir.display()
        )));
    }
    let nonce = now_ns();
    let staging = releases.join(format!(".staging-{}-{nonce}", std::process::id()));
    let staged = (|| {
        std::fs::create_dir(&staging).map_err(|e| io_failed("create", &staging, e))?;
        let marker = StagingMarker {
            owner: OWNER.into(),
            pid: std::process::id(),
            started_ns: nonce,
        };
        write_synced(
            &staging.join(STAGING_MARKER),
            &serde_json::to_vec(&marker).map_err(|e| ServiceError::Failed(e.to_string()))?,
        )?;
        copy_synced(binary, &staging.join(RELEASE_BINARY))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                staging.join(RELEASE_BINARY),
                std::fs::Permissions::from_mode(0o755),
            )
            .map_err(|e| io_failed("mark executable", &staging, e))?;
        }
        copy_tree(web_dist, &staging.join(RELEASE_WEB))?;
        write_synced(
            &staging.join(RELEASE_MANIFEST),
            &serde_json::to_vec_pretty(&manifest)
                .map_err(|e| ServiceError::Failed(e.to_string()))?,
        )?;
        // The marker goes once the release is complete; the manifest now owns it.
        std::fs::remove_file(staging.join(STAGING_MARKER))
            .map_err(|e| io_failed("finish", &staging, e))?;
        sync_dir(&staging)?;
        if !verify_release(&staging, &manifest) {
            return Err(ServiceError::Failed(
                "The copied service files did not match their source; nothing was installed."
                    .into(),
            ));
        }
        // Our own unusable release with this id is moved aside, then removed.
        if owned.is_some() {
            let aside = releases.join(format!(".stale-{id}-{nonce}"));
            std::fs::rename(&dir, &aside).map_err(|e| io_failed("replace", &dir, e))?;
            let _ = std::fs::remove_dir_all(&aside);
        }
        std::fs::rename(&staging, &dir).map_err(|e| io_failed("publish", &dir, e))?;
        sync_dir(&releases)
    })();
    if let Err(error) = staged {
        if read_staging_marker(&staging).is_some() || read_manifest(&staging).is_some() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        return Err(error);
    }
    Ok(release)
}

/// Remove owned releases except `keep`. Owned means a real directory directly
/// under `home/releases` that carries our release manifest, or our staging
/// marker from a process that no longer exists. Names alone never count, and
/// nothing else, inside or outside `home/releases`, is touched. Must be called
/// with the home lock held.
pub(crate) fn prune_releases(_lock: &HomeLock, home: &Path, keep: &[&Path]) -> Vec<PathBuf> {
    let releases = home.join("releases");
    if !is_real_dir(&releases) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&releases) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if keep.contains(&path.as_path()) || !is_real_dir(&path) {
            continue;
        }
        let owned = read_manifest(&path).is_some()
            || read_staging_marker(&path).is_some_and(|marker| !process_alive(marker.pid));
        if owned && std::fs::remove_dir_all(&path).is_ok() {
            removed.push(path);
        }
    }
    let _ = sync_dir(&releases);
    removed
}

/// The release directory a definition's binary belongs to, if it is one of ours.
fn release_of(home: &Path, binary: &Path) -> Option<PathBuf> {
    let dir = binary.parent()?;
    (dir.parent()? == home.join("releases") && read_manifest(dir).is_some())
        .then(|| dir.to_path_buf())
}

// ── Definition switch ──────────────────────────────────────────────────────

/// How an unload ended. Absence is a confirmed outcome, not a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unloaded {
    /// The service was loaded and is now confirmed not running.
    Stopped,
    /// The service manager confirms there was nothing loaded.
    WasAbsent,
}

/// What a service manager query established about the job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JobState {
    /// The manager positively reports no such job.
    NotLoaded,
    /// The job is known but not running (systemd: loaded, inactive, no PID).
    Inactive,
    /// The job is loaded or running.
    Running,
    /// The query failed or answered in a way that settles nothing.
    Indeterminate(String),
}

/// One finished external command, or why it could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueryOutput {
    /// `None` when the process ended without an exit code (a signal).
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a manager command; a fake in tests.
pub(crate) type Runner = dyn Fn(&str, &[&str]) -> Result<QueryOutput, String>;

fn real_runner(program: &str, args: &[&str]) -> Result<QueryOutput, String> {
    Command::new(program)
        .args(args)
        .output()
        .map(|output| QueryOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
        .map_err(|error| format!("{program} could not run: {error}"))
}

fn summary(output: &QueryOutput) -> String {
    let text = format!("{} {}", output.stdout.trim(), output.stderr.trim());
    let text = text.trim();
    match output.code {
        Some(code) => format!("exit {code}: {text}"),
        None => format!("ended by a signal: {text}"),
    }
}

/// `launchctl print <domain>/<label>`: exit 0 means the job is in the domain.
/// Only launchd's own "no such service" answer (exit 113 naming this label)
/// means absent; a missing domain (112), a permission error or anything else
/// is indeterminate.
pub(crate) fn parse_launchctl_print(output: &QueryOutput, label: &str) -> JobState {
    let text = format!("{}\n{}", output.stdout, output.stderr);
    match output.code {
        Some(0) => JobState::Running,
        Some(113) if text.contains(&format!("Could not find service \"{label}\"")) => {
            JobState::NotLoaded
        }
        _ => JobState::Indeterminate(summary(output)),
    }
}

/// `systemctl --user show <unit> --property=LoadState,ActiveState,MainPID`.
/// systemd answers this with exit 0 even for unknown units, so a nonzero exit
/// (no user bus, permission) is indeterminate, as is any missing field.
pub(crate) fn parse_systemctl_show(output: &QueryOutput) -> JobState {
    if output.code != Some(0) {
        return JobState::Indeterminate(summary(output));
    }
    let field = |name: &str| {
        output
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(str::trim)
    };
    let (Some(load), Some(active), Some(pid)) =
        (field("LoadState"), field("ActiveState"), field("MainPID"))
    else {
        return JobState::Indeterminate(summary(output));
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return JobState::Indeterminate(summary(output));
    };
    match (load, active, pid) {
        (_, "active" | "activating" | "deactivating" | "reloading" | "refreshing", _) => {
            JobState::Running
        }
        (_, _, pid) if pid != 0 => JobState::Running,
        ("not-found", "inactive", 0) => JobState::NotLoaded,
        ("loaded" | "masked", "inactive" | "failed", 0) => JobState::Inactive,
        _ => JobState::Indeterminate(summary(output)),
    }
}

/// Turn an unload attempt and the follow-up state into a confirmed outcome.
fn confirm_unloaded(
    unload_succeeded: bool,
    state: JobState,
    detail: &str,
) -> Result<Unloaded, ServiceError> {
    match state {
        JobState::NotLoaded if unload_succeeded => Ok(Unloaded::Stopped),
        JobState::NotLoaded => Ok(Unloaded::WasAbsent),
        JobState::Inactive => Ok(Unloaded::Stopped),
        JobState::Running => Err(ServiceError::Failed(format!(
            "The service is still loaded after the attempt to stop it.{detail}"
        ))),
        JobState::Indeterminate(why) => Err(ServiceError::Failed(format!(
            "Could not confirm that the service stopped ({why}).{detail}"
        ))),
    }
}

/// Time for the stop-confirmation wait; the real clock in use, a fake in tests
/// so a persistent or unknown state does not cost the full deadline.
trait Clock {
    fn now(&self) -> std::time::Instant;
    fn sleep(&self, pause: std::time::Duration);
}

struct RealClock;
impl Clock for RealClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
    fn sleep(&self, pause: std::time::Duration) {
        std::thread::sleep(pause);
    }
}
static REAL_CLOCK: RealClock = RealClock;

/// How long a job may still be listed after a stop command that succeeded.
#[derive(Debug, Clone, Copy)]
struct Settle {
    deadline: std::time::Duration,
    interval: std::time::Duration,
}
const SETTLE: Settle = Settle {
    deadline: std::time::Duration::from_secs(10),
    interval: std::time::Duration::from_millis(200),
};

/// After a stop command succeeded, a manager may keep listing the job while it
/// tears it down. Re-query a `Running` answer until the deadline; any other
/// answer is final at once, so an unknown state never waits and never becomes
/// absence. `Running` at the deadline stays `Running`.
fn settle_state(query: &dyn Fn() -> JobState, clock: &dyn Clock, settle: Settle) -> JobState {
    let started = clock.now();
    loop {
        let state = query();
        if state != JobState::Running
            || clock.now().saturating_duration_since(started) >= settle.deadline
        {
            return state;
        }
        clock.sleep(settle.interval);
    }
}

/// What loads and unloads the service; launchd or systemd in use, a fake in tests.
pub(crate) trait Manager {
    /// Stop and unload the service, and confirm it. Absence is `Ok(WasAbsent)`;
    /// a service that may still be running, or a state that cannot be read, is
    /// an error.
    fn unload(&self) -> Result<Unloaded, ServiceError>;
    /// Load (and start) the definition at `path`.
    fn load(&self, path: &Path) -> Result<(), ServiceError>;
    /// One query of the job's current state; it never changes anything.
    fn state(&self) -> JobState;
}

fn ran(output: Result<QueryOutput, String>) -> Result<QueryOutput, ServiceError> {
    match output {
        Ok(output) if output.code == Some(0) => Ok(output),
        Ok(output) => Err(ServiceError::Failed(summary(&output))),
        Err(error) => Err(ServiceError::Failed(error)),
    }
}

fn detail_of(attempt: &Result<QueryOutput, ServiceError>) -> String {
    attempt
        .as_ref()
        .err()
        .map_or_else(String::new, |error| format!(" ({error})"))
}

pub(crate) struct Launchd<'a> {
    domain: String,
    label: &'a str,
    runner: &'a Runner,
    clock: &'a dyn Clock,
    settle: Settle,
}
impl Manager for Launchd<'_> {
    fn unload(&self) -> Result<Unloaded, ServiceError> {
        let target = format!("{}/{}", self.domain, self.label);
        let bootout = ran((self.runner)("launchctl", &["bootout", &target]));
        // A refused bootout gets one query: there is no teardown to wait for.
        let state = if bootout.is_ok() {
            settle_state(&|| self.state(), self.clock, self.settle)
        } else {
            self.state()
        };
        confirm_unloaded(bootout.is_ok(), state, &detail_of(&bootout))
    }
    fn state(&self) -> JobState {
        let target = format!("{}/{}", self.domain, self.label);
        match (self.runner)("launchctl", &["print", &target]) {
            Ok(output) => parse_launchctl_print(&output, self.label),
            Err(error) => JobState::Indeterminate(error),
        }
    }
    fn load(&self, path: &Path) -> Result<(), ServiceError> {
        ran((self.runner)(
            "launchctl",
            &["bootstrap", &self.domain, &path.display().to_string()],
        ))
        .map(drop)
    }
}

pub(crate) struct Systemd<'a> {
    unit: &'a str,
    runner: &'a Runner,
    clock: &'a dyn Clock,
    settle: Settle,
}
impl Manager for Systemd<'_> {
    fn unload(&self) -> Result<Unloaded, ServiceError> {
        let disable = ran((self.runner)(
            "systemctl",
            &["--user", "disable", "--now", self.unit],
        ));
        let state = if disable.is_ok() {
            settle_state(&|| self.state(), self.clock, self.settle)
        } else {
            self.state()
        };
        confirm_unloaded(disable.is_ok(), state, &detail_of(&disable))
    }
    fn state(&self) -> JobState {
        match (self.runner)(
            "systemctl",
            &[
                "--user",
                "show",
                self.unit,
                "--property=LoadState,ActiveState,MainPID",
            ],
        ) {
            Ok(output) => parse_systemctl_show(&output),
            Err(error) => JobState::Indeterminate(error),
        }
    }
    fn load(&self, _path: &Path) -> Result<(), ServiceError> {
        ran((self.runner)("systemctl", &["--user", "daemon-reload"]))?;
        ran((self.runner)(
            "systemctl",
            &["--user", "enable", "--now", self.unit],
        ))
        .map(drop)
    }
}

/// The definition currently on disk: `None` only when it is confirmed absent.
/// Any other read failure is an error, never an absent definition.
pub(crate) fn read_definition(path: &Path) -> Result<Option<Vec<u8>>, ServiceError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_failed("read", path, error)),
    }
}

/// Replace `path` atomically: write a private sibling, flush it, rename it over
/// the old definition, then flush the directory. A failure before the rename
/// leaves the old bytes in place; a failure after it (the directory flush) has
/// already switched the file, which is why callers check what is on disk.
pub(crate) fn replace_durably(path: &Path, bytes: &[u8]) -> Result<(), ServiceError> {
    let parent = path
        .parent()
        .ok_or_else(|| ServiceError::Failed(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(parent).map_err(|e| io_failed("create", parent, e))?;
    let temp = parent.join(format!(
        ".{}.{}-{}.tmp",
        path.file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        std::process::id(),
        now_ns()
    ));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // The definition can carry configured BRUHS_* credentials.
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options
            .open(&temp)
            .map_err(|e| io_failed("write", &temp, e))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| io_failed("write", &temp, e))?;
        std::fs::rename(&temp, path).map_err(|e| io_failed("replace", path, e))?;
        sync_dir(parent)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

fn remove_durably(path: &Path) -> Result<(), ServiceError> {
    match std::fs::remove_file(path) {
        Ok(()) => path.parent().map_or(Ok(()), sync_dir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_failed("remove", path, error)),
    }
}

/// Writes a definition file; `replace_durably` in use, injected in tests.
pub(crate) type DefinitionWriter<'a> = dyn Fn(&Path, &[u8]) -> Result<(), ServiceError> + 'a;

/// Put the definition back to exactly `previous` (or its absence). Checks the
/// disk rather than trusting the writer's result, since a writer can switch the
/// file and still fail afterwards.
fn restore_definition(
    path: &Path,
    previous: Option<&[u8]>,
    write: &DefinitionWriter<'_>,
) -> Result<(), ServiceError> {
    // A read that fails for any reason but absence settles nothing: report it.
    if read_definition(path)?.as_deref() == previous {
        return Ok(());
    }
    let restored = match previous {
        Some(old) => write(path, old),
        None => remove_durably(path),
    };
    let matches = read_definition(path)?.as_deref() == previous;
    match (restored, matches) {
        // The bytes are right even if a late flush complained.
        (_, true) => Ok(()),
        (Err(error), false) => Err(error),
        (Ok(()), false) => Err(ServiceError::Failed(format!(
            "{} does not hold the previous service setup after restoring it.",
            path.display()
        ))),
    }
}

fn compound(error: ServiceError, undo: Result<(), ServiceError>, restored: &str) -> ServiceError {
    match undo {
        Ok(()) => ServiceError::Failed(format!("{error} {restored}")),
        Err(undo) => ServiceError::Failed(format!(
            "{error} Restoring the previous service setup also failed: {undo}"
        )),
    }
}

/// Switch the service to `new` at `path`. Every failure after the first change
/// is undone, or reported together with why it could not be:
/// - write fails: the definition is restored on disk (the writer may already
///   have switched it); the manager is not touched.
/// - the old service cannot be confirmed stopped: the old definition is put back.
///   One new query then decides: only a confirmed absent or inactive job lets
///   that restored definition load again; a running or unknown job is left as
///   it is, so a duplicate is never loaded.
/// - the new definition fails to load: it is unloaded; once that is confirmed
///   the old definition is restored and loaded again. If the new one cannot be
///   confirmed stopped, everything is left as it is and reported.
pub(crate) fn switch_definition(
    manager: &dyn Manager,
    path: &Path,
    new: &[u8],
    previous: Option<&[u8]>,
    write: &DefinitionWriter<'_>,
) -> Result<(), ServiceError> {
    const RESTORED: &str = "The previous service setup was restored.";
    if let Err(error) = write(path, new) {
        return Err(compound(
            error,
            restore_definition(path, previous, write),
            RESTORED,
        ));
    }
    if let Err(error) = manager.unload() {
        let restored = restore_definition(path, previous, write);
        // Only a new confirmation that the old job is gone allows reloading it.
        let mut follow_up_query_error = None;
        let reloaded = match (&restored, previous) {
            (Ok(()), Some(_)) => match manager.state() {
                JobState::NotLoaded | JobState::Inactive => Some(manager.load(path)),
                JobState::Running => None,
                JobState::Indeterminate(why) => {
                    follow_up_query_error = Some(why);
                    None
                }
            },
            _ => None,
        };
        return Err(match reloaded {
            Some(Ok(())) => ServiceError::Failed(format!(
                "{error} The old service then stopped, so the previous service setup was restored and loaded again."
            )),
            Some(Err(load)) => ServiceError::Failed(format!(
                "{error} The old service then stopped and the previous service setup was restored, but loading it again failed: {load}"
            )),
            None => {
                let kept = compound(
                    error,
                    restored,
                    "The previous service definition was kept for the service that may still be running; nothing new was loaded.",
                );
                match follow_up_query_error {
                    Some(why) => ServiceError::Failed(format!(
                        "{kept} The follow-up service state could not be confirmed ({why})."
                    )),
                    None => kept,
                }
            }
        });
    }
    let Err(error) = manager.load(path) else {
        return Ok(());
    };
    if let Err(stop) = manager.unload() {
        return Err(ServiceError::Failed(format!(
            "{error} The new service could not be confirmed stopped, so nothing was rolled back: {stop}"
        )));
    }
    let undo = restore_definition(path, previous, write).and_then(|()| match previous {
        Some(_) => manager.load(path),
        None => Ok(()),
    });
    Err(compound(error, undo, RESTORED))
}

/// Remove the service, keeping its definition and releases unless the manager
/// confirms it is stopped (or was never loaded).
pub(crate) fn uninstall(
    lock: &HomeLock,
    manager: &dyn Manager,
    home: &Path,
    path: &Path,
) -> Result<Unloaded, ServiceError> {
    let outcome = manager.unload().map_err(|error| {
        ServiceError::Failed(format!(
            "{error} The service definition and its files were kept; try again."
        ))
    })?;
    remove_durably(path)?;
    // Only owned releases go; logs and other app data stay.
    prune_releases(lock, home, &[]);
    Ok(outcome)
}

/// Who is asking for the status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asker {
    /// A running server answering `/api/service`; it is the process on the
    /// port, so it never probes itself.
    Server { pid: u32 },
    /// The `service` command, which probes the port once.
    Cli { port: u16 },
}

/// What answered a readiness probe on the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Responder {
    /// Nothing answered in time.
    Silent,
    /// Something answered without saying which process it is.
    Unidentified,
    /// A bruhs-server that named its process.
    Process(u32),
}

pub fn status(asker: Asker) -> BackgroundService {
    let path = match definition_path() {
        Ok(path) => path,
        Err(error) => {
            return BackgroundService {
                detail: error.to_string(),
                debug_build: cfg!(debug_assertions),
                ..BackgroundService::default()
            }
        }
    };
    let installed = path.is_file();
    let manager = installed.then(manager_state).flatten();
    let loaded = manager.is_some();
    let service_pid = manager.flatten();
    let binary = installed.then(|| installed_binary(&path)).flatten();
    let reactive_protocol = binary.as_deref().and_then(|binary| {
        let manifest = read_manifest(Path::new(binary).parent()?)?;
        Some(manifest.reactive_protocol)
    });
    let log_path = installed
        .then(service_log)
        .flatten()
        .map(|path| path.display().to_string());
    let responder = match asker {
        Asker::Server { pid } => Responder::Process(pid),
        // Only worth a probe when the service has a process to find.
        Asker::Cli { port } if service_pid.is_some() => probe(port),
        Asker::Cli { .. } => Responder::Silent,
    };
    let readiness = readiness(installed, loaded, service_pid, responder);
    BackgroundService {
        supported: true,
        installed,
        loaded,
        binary,
        detail: readiness.detail(log_path.as_deref(), asker),
        serving: readiness == Readiness::Serving,
        service_pid,
        responder_pid: match responder {
            Responder::Process(pid) => Some(pid),
            _ => None,
        },
        log_path,
        reactive_protocol,
        debug_build: cfg!(debug_assertions),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    NotInstalled,
    /// Installed, but the service manager has not loaded it.
    Waiting,
    /// Loaded without a running process (failed start, retrying).
    NotRunning,
    /// The service has a process, but another server holds the port.
    PortHeld,
    /// The service has a process, but nothing answers on the port.
    Silent,
    /// Something answers, but it does not identify itself.
    Unverified,
    Serving,
}

/// Serving needs the service's own nonzero process to be the one answering;
/// a missing, zero or different process never counts.
fn readiness(
    installed: bool,
    loaded: bool,
    service_pid: Option<u32>,
    responder: Responder,
) -> Readiness {
    if !installed {
        return Readiness::NotInstalled;
    }
    if !loaded {
        return Readiness::Waiting;
    }
    let Some(service) = service_pid.filter(|pid| *pid != 0) else {
        return Readiness::NotRunning;
    };
    match responder {
        Responder::Process(pid) if pid != 0 && pid == service => Readiness::Serving,
        Responder::Process(_) => Readiness::PortHeld,
        Responder::Unidentified => Readiness::Unverified,
        Responder::Silent => Readiness::Silent,
    }
}

impl Readiness {
    fn detail(self, log: Option<&str>, asker: Asker) -> String {
        let see_log = log.map_or_else(String::new, |log| format!(" Its log is at {log}."));
        let address = match asker {
            Asker::Cli { port } => format!("127.0.0.1:{port}"),
            Asker::Server { .. } => "its port".into(),
        };
        match self {
            Self::NotInstalled => "Automations stop when this server stops.".into(),
            Self::Waiting => {
                "Installed, waiting to start. It takes over once the running server exits.".into()
            }
            Self::NotRunning => match asker {
                // Expected while this server holds the port.
                Asker::Server { .. } => {
                    "Installed. The service starts once this server exits.".into()
                }
                Asker::Cli { .. } => format!(
                    "Installed, but the service is not running. It retries every 30 seconds.{see_log}"
                ),
            },
            Self::PortHeld => match asker {
                Asker::Server { .. } => {
                    "Installed. The service takes over once this server exits.".into()
                }
                Asker::Cli { .. } => format!(
                    "Installed, but another server holds {address}. The service takes over once it exits."
                ),
            },
            Self::Silent => format!(
                "Installed and started, but it is not answering on {address}.{see_log}"
            ),
            Self::Unverified => format!(
                "Installed and started, but its readiness is unverified: something answers on {address} without naming its process.{see_log}"
            ),
            Self::Serving => {
                "Automations keep running after the app quits and start at login.".into()
            }
        }
    }
}

/// `None` when the manager does not have the service loaded; otherwise the
/// loaded state with its process, if it reports one.
fn manager_state() -> Option<Option<u32>> {
    if cfg!(target_os = "macos") {
        let domain = gui_domain().ok()?;
        let printed = run("launchctl", &["print", &format!("{domain}/{LABEL}")]).ok()?;
        Some(launchd_pid(&printed))
    } else {
        run("systemctl", &["--user", "is-active", "--quiet", UNIT]).ok()?;
        let main = run(
            "systemctl",
            &["--user", "show", "--property=MainPID", "--value", UNIT],
        )
        .ok();
        Some(main.and_then(|text| positive_pid(text.trim())))
    }
}

/// The job's own `pid = N` line from `launchctl print` (the first one; nested
/// sections come later).
fn launchd_pid(printed: &str) -> Option<u32> {
    printed
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid = "))
        .and_then(positive_pid)
}

fn positive_pid(text: &str) -> Option<u32> {
    text.parse::<u32>().ok().filter(|pid| *pid != 0)
}

fn service_log() -> Option<PathBuf> {
    cfg!(target_os = "macos")
        .then(|| log_dir().ok())
        .flatten()
        .map(|dir| dir.join("bruhs-server.service.log"))
}

/// One bounded request to the loopback port. Only a well-formed status that
/// names its process identifies the responder.
fn probe(port: u16) -> Responder {
    probe_within(
        std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        PROBE_BUDGET,
    )
}

/// The whole probe (connect, request and every read) shares one deadline, so
/// a responder that drips bytes or keeps the connection open cannot hold the
/// command past `budget`; `PROBE_LIMIT` separately caps the bytes kept.
fn probe_within(address: std::net::SocketAddr, budget: std::time::Duration) -> Responder {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + budget;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
    };
    let Some(left) = remaining() else {
        return Responder::Silent;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, left.min(Duration::from_secs(1)))
    else {
        return Responder::Silent;
    };
    let Some(left) = remaining() else {
        return Responder::Silent;
    };
    let port = address.port();
    let request = format!(
        "GET /api/service HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    if stream.set_write_timeout(Some(left)).is_err()
        || stream.write_all(request.as_bytes()).is_err()
    {
        return Responder::Silent;
    }
    let mut response = Vec::new();
    let mut buffer = [0_u8; 4096];
    while response.len() < PROBE_LIMIT {
        // Each read gets only what is left of the shared deadline.
        let Some(left) = remaining() else { break };
        if stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let room = PROBE_LIMIT - response.len();
                response.extend_from_slice(&buffer[..read.min(room)]);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    // A deadline can still leave a complete response behind; judge the bytes.
    if response.is_empty() {
        Responder::Silent
    } else {
        responder_from(&response)
    }
}

/// Total time the `service status` probe may take.
const PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);
/// Bytes of the response kept for judging.
const PROBE_LIMIT: usize = 64 * 1024;

fn responder_from(response: &[u8]) -> Responder {
    let text = String::from_utf8_lossy(response);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Responder::Unidentified;
    };
    let ok = head
        .lines()
        .next()
        .is_some_and(|status| status.split_whitespace().nth(1) == Some("200"));
    let chunked = head.lines().any(|line| {
        line.to_ascii_lowercase()
            .starts_with("transfer-encoding: chunked")
    });
    if !ok || chunked {
        return Responder::Unidentified;
    }
    serde_json::from_str::<BackgroundService>(body.trim())
        .ok()
        .and_then(|status| status.responder_pid)
        .filter(|pid| *pid != 0)
        .map_or(Responder::Unidentified, Responder::Process)
}

fn installed_binary(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if cfg!(target_os = "macos") {
        let after = text.split("<key>ProgramArguments</key>").nth(1)?;
        let start = after.find("<string>")? + "<string>".len();
        let end = after[start..].find("</string>")? + start;
        Some(
            after[start..end]
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\""),
        )
    } else {
        text.lines()
            .find_map(|line| line.strip_prefix("ExecStart="))
            .map(|value| value.trim_matches('"').to_owned())
    }
}

pub fn apply(
    action: BackgroundServiceAction,
    web_dist: &Path,
    asker: Asker,
) -> Result<BackgroundService, ServiceError> {
    let path = definition_path()?;
    let home = service_home()?;
    // Held across staging, the definition switch and pruning.
    let lock = lock_home(&home, LOCK_WAIT)?;
    let manager: Box<dyn Manager> = if cfg!(target_os = "macos") {
        Box::new(Launchd {
            domain: gui_domain()?,
            label: LABEL,
            runner: &real_runner,
            clock: &REAL_CLOCK,
            settle: SETTLE,
        })
    } else {
        Box::new(Systemd {
            unit: UNIT,
            runner: &real_runner,
            clock: &REAL_CLOCK,
            settle: SETTLE,
        })
    };
    match action {
        BackgroundServiceAction::Install => {
            let source = std::env::current_exe()
                .and_then(std::fs::canonicalize)
                .map_err(|error| {
                    ServiceError::Failed(format!("Could not find this server binary: {error}"))
                })?;
            // The service runs its own verified copy, never the installing binary.
            let release = stage_release(&lock, &home, &source, web_dist)?;
            // Unknown is not absent: an unreadable definition stops the install
            // before anything is overwritten.
            let previous = read_definition(&path)?;
            let previous_release =
                installed_binary(&path).and_then(|binary| release_of(&home, Path::new(&binary)));
            let env = environment(&release.web);
            let definition = if cfg!(target_os = "macos") {
                let logs = log_dir()?;
                std::fs::create_dir_all(&logs)
                    .map_err(|error| ServiceError::Failed(error.to_string()))?;
                launchd_plist(&release.binary, &env, &logs)
            } else {
                systemd_unit(&release.binary, &env)
            };
            switch_definition(
                manager.as_ref(),
                &path,
                definition.as_bytes(),
                previous.as_deref(),
                &replace_durably,
            )?;
            // Keep this release and the one before it, for rollback.
            let mut keep = vec![release.dir.as_path()];
            keep.extend(previous_release.as_deref());
            prune_releases(&lock, &home, &keep);
        }
        BackgroundServiceAction::Uninstall => {
            uninstall(&lock, manager.as_ref(), &home, &path)?;
            if cfg!(target_os = "linux") {
                let _ = run("systemctl", &["--user", "daemon-reload"]);
            }
        }
    }
    drop(lock);
    Ok(status(asker))
}

/// `bruhs-server service install|uninstall|status`. Returns an exit code when handled.
pub fn cli(web_dist: &Path) -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("service") {
        return None;
    }
    let asker = Asker::Cli {
        port: crate::config::Config::from_env()
            .map_or(crate::config::DEFAULT_PORT, |config| config.port),
    };
    let result = match args.next().as_deref() {
        Some("install") => apply(BackgroundServiceAction::Install, web_dist, asker),
        Some("uninstall") => apply(BackgroundServiceAction::Uninstall, web_dist, asker),
        Some("status") | None => Ok(status(asker)),
        Some(other) => {
            eprintln!("Unknown service command “{other}”. Use install, uninstall or status.");
            return Some(2);
        }
    };
    match result {
        Ok(status) => {
            println!("{}", status.detail);
            if let Some(binary) = status.binary {
                println!("Binary: {binary}");
            }
            if let Some(pid) = status.service_pid {
                println!("Service process: {pid}");
            }
            Some(0)
        }
        Err(error) => {
            eprintln!("{error}");
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_escape_values_and_restart_only_on_failure() {
        let env = vec![
            ("BRUHS_WEB_DIST".to_owned(), "/Apps/<b&r>/web".to_owned()),
            ("PATH".to_owned(), "/usr/bin:/opt/100%".to_owned()),
        ];
        let plist = launchd_plist(
            Path::new("/Apps/bruhs \"x\"/bruhs-server"),
            &env,
            Path::new("/logs"),
        );
        assert!(plist.contains("<string>/Apps/&lt;b&amp;r&gt;/web</string>"));
        assert!(plist.contains("<string>/Apps/bruhs &quot;x&quot;/bruhs-server</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n\t\t<false/>"));
        let unit = systemd_unit(Path::new("/opt/bruhs/bruhs-server"), &env);
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/opt/100%%\""));
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn only_the_services_own_answering_process_counts_as_serving() {
        use Readiness::{NotInstalled, NotRunning, PortHeld, Serving, Silent, Unverified, Waiting};
        let process = Responder::Process;
        assert_eq!(readiness(false, false, None, process(7)), NotInstalled);
        assert_eq!(readiness(true, false, None, process(7)), Waiting);
        // Loaded without a process, or with pid 0, is not running.
        assert_eq!(readiness(true, true, None, process(7)), NotRunning);
        assert_eq!(readiness(true, true, Some(0), process(0)), NotRunning);
        // Matching, mismatched and zero responders.
        assert_eq!(readiness(true, true, Some(7), process(7)), Serving);
        assert_eq!(readiness(true, true, Some(7), process(8)), PortHeld);
        assert_eq!(readiness(true, true, Some(7), process(0)), PortHeld);
        // Silence and an answer without identity never establish serving.
        assert_eq!(readiness(true, true, Some(7), Responder::Silent), Silent);
        assert_eq!(
            readiness(true, true, Some(7), Responder::Unidentified),
            Unverified
        );
    }

    #[test]
    fn readiness_wording_never_claims_a_stuck_service_is_running() {
        let cli = Asker::Cli { port: 3001 };
        let server = Asker::Server { pid: 1 };
        let log = Some("/logs/bruhs-server.service.log");
        assert_eq!(
            Readiness::Serving.detail(log, cli),
            "Automations keep running after the app quits and start at login."
        );
        assert_eq!(
            Readiness::Silent.detail(log, cli),
            "Installed and started, but it is not answering on 127.0.0.1:3001. Its log is at /logs/bruhs-server.service.log."
        );
        assert!(Readiness::Unverified
            .detail(None, cli)
            .contains("readiness is unverified"));
        assert_eq!(
            Readiness::PortHeld.detail(log, server),
            "Installed. The service takes over once this server exits."
        );
        for state in [
            Readiness::Waiting,
            Readiness::NotRunning,
            Readiness::PortHeld,
            Readiness::Silent,
            Readiness::Unverified,
        ] {
            for asker in [cli, server] {
                assert!(!state.detail(log, asker).contains("keep running"));
            }
        }
    }

    #[test]
    fn the_job_pid_is_read_from_launchctl_and_zero_means_none() {
        let printed = "gui/501/dev.bruhs.server = {\n\tactive count = 1\n\tstate = running\n\tpid = 44689\n\tendpoints = {\n\t\tpid = 1\n\t}\n}";
        assert_eq!(launchd_pid(printed), Some(44689));
        assert_eq!(launchd_pid("\tstate = waiting\n"), None);
        assert_eq!(launchd_pid("\tpid = 0\n"), None);
        assert_eq!(positive_pid("0"), None);
        assert_eq!(positive_pid("x"), None);
    }

    #[test]
    fn a_probe_identifies_only_well_formed_statuses_that_name_their_process() {
        let response = |status: &str, headers: &str, body: &str| {
            format!("HTTP/1.1 {status}\r\ncontent-type: application/json{headers}\r\n\r\n{body}")
                .into_bytes()
        };
        let current = r#"{"supported":true,"installed":true,"loaded":true,"detail":"x","serving":true,"responder_pid":44689}"#;
        assert_eq!(
            responder_from(&response("200 OK", "", current)),
            Responder::Process(44689)
        );
        // An older server's status has no identity: readiness is unverified.
        let old = r#"{"supported":true,"installed":true,"loaded":true,"binary":null,"detail":"x"}"#;
        assert_eq!(
            responder_from(&response("200 OK", "", old)),
            Responder::Unidentified
        );
        let zero =
            r#"{"supported":true,"installed":true,"loaded":true,"detail":"x","responder_pid":0}"#;
        assert_eq!(
            responder_from(&response("200 OK", "", zero)),
            Responder::Unidentified
        );
        for bad in [
            response("404 Not Found", "", current),
            response("200 OK", "\r\ntransfer-encoding: chunked", current),
            response("200 OK", "", "not json"),
            b"HTTP/1.1 200 OK".to_vec(),
        ] {
            assert_eq!(responder_from(&bad), Responder::Unidentified);
        }
    }

    #[test]
    fn older_status_json_reads_with_safe_defaults() {
        let old: BackgroundService = serde_json::from_str(
            r#"{"supported":true,"installed":true,"loaded":true,"binary":"/b","detail":"x"}"#,
        )
        .unwrap();
        assert!(old.loaded);
        assert!(!old.serving, "loaded never implies serving");
        assert_eq!(
            (old.service_pid, old.responder_pid, old.log_path),
            (None, None, None)
        );
        assert!(!old.debug_build, "an older server reads as a release");
    }

    #[test]
    fn the_status_says_whether_this_server_is_a_debug_build() {
        let status = status(Asker::Server {
            pid: std::process::id(),
        });
        assert_eq!(status.debug_build, cfg!(debug_assertions));
    }

    #[test]
    fn a_closed_port_is_silence_not_an_unverified_answer() {
        // Bind and drop to find a port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .unwrap();
        assert_eq!(probe(port), Responder::Silent);
    }

    /// A loopback responder that reads the request, then runs `respond`.
    fn responder(
        respond: impl FnOnce(std::net::TcpStream) + Send + 'static,
    ) -> std::net::SocketAddr {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                respond(stream);
            }
        });
        address
    }

    #[test]
    fn a_dripping_responder_cannot_hold_the_probe_past_its_deadline() {
        use std::io::Write;
        use std::time::{Duration, Instant};
        // One byte every 50 ms for 10 s: each read succeeds, so only an
        // overall deadline ends the probe.
        let address = responder(|mut stream| {
            for _ in 0..200 {
                if stream.write_all(b"H").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let started = Instant::now();
        let result = probe_within(address, Duration::from_millis(500));
        let took = started.elapsed();
        assert!(took < Duration::from_secs(2), "probe took {took:?}");
        // Bytes arrived but never a status naming its process.
        assert_eq!(result, Responder::Unidentified);
    }

    #[test]
    fn a_complete_answer_on_a_kept_open_connection_still_identifies_its_process() {
        use std::io::Write;
        use std::time::{Duration, Instant};
        let address = responder(|mut stream| {
            let body = r#"{"supported":true,"installed":true,"loaded":true,"detail":"x","responder_pid":4242}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            // Ignore Connection: close and keep the socket open.
            std::thread::sleep(Duration::from_secs(5));
        });
        let started = Instant::now();
        let result = probe_within(address, Duration::from_millis(500));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(result, Responder::Process(4242));
    }

    #[test]
    fn a_responder_that_accepts_but_never_writes_is_silence() {
        use std::time::Duration;
        let address = responder(|_stream| std::thread::sleep(Duration::from_secs(5)));
        assert_eq!(
            probe_within(address, Duration::from_millis(300)),
            Responder::Silent
        );
    }

    /// A fake install source: an executable "binary" and a two-file web bundle,
    /// placed under a folder named like a protected one.
    fn source(root: &Path, binary_bytes: &[u8]) -> (PathBuf, PathBuf) {
        let base = root.join("Documents/project/target/debug");
        std::fs::create_dir_all(base.join("dist/js")).unwrap();
        std::fs::write(base.join("bruhs-server"), binary_bytes).unwrap();
        std::fs::write(base.join("dist/index.html"), b"<html></html>").unwrap();
        std::fs::write(base.join("dist/js/app.js"), b"console.log(1)").unwrap();
        (base.join("bruhs-server"), base.join("dist"))
    }

    fn tree_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for path in walk_all(root) {
            if is_real_file(&path) {
                out.insert(path.display().to_string(), std::fs::read(&path).unwrap());
            }
        }
        out
    }
    fn walk_all(root: &Path) -> Vec<PathBuf> {
        let mut out = vec![root.to_path_buf()];
        if is_real_dir(root) {
            for entry in std::fs::read_dir(root).unwrap().flatten() {
                out.extend(walk_all(&entry.path()));
            }
        }
        out
    }

    fn lock(home: &Path) -> HomeLock {
        lock_home(home, std::time::Duration::from_secs(1)).unwrap()
    }

    #[test]
    fn a_release_is_an_independent_verified_executable_copy() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("app-data/service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let release = stage_release(&lock(&home), &home, &binary, &dist).unwrap();
        assert!(release.dir.starts_with(home.join("releases")));
        assert!(!release.binary.starts_with(temp.path().join("Documents")));
        assert_eq!(std::fs::read(&release.binary).unwrap(), b"server v1");
        std::fs::write(&binary, b"rebuilt").unwrap();
        assert_eq!(std::fs::read(&release.binary).unwrap(), b"server v1");
        assert!(is_executable(&release.binary));
        assert!(
            !release.dir.join(STAGING_MARKER).exists(),
            "marker removed when complete"
        );
        let manifest = read_manifest(&release.dir).unwrap();
        assert!(verify_release(&release.dir, &manifest));
        assert_eq!(
            manifest.reactive_protocol,
            bruhs_models::crew::CREW_PROTOCOL
        );
    }

    #[test]
    fn a_manifest_without_the_protocol_reads_as_zero() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let release = stage_release(&lock(&home), &home, &binary, &dist).unwrap();
        let file = release.dir.join(RELEASE_MANIFEST);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("reactive_protocol");
        std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(read_manifest(&release.dir).unwrap().reactive_protocol, 0);
    }

    #[test]
    fn the_same_files_reuse_the_release_and_leave_no_staging() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let first = stage_release(&held, &home, &binary, &dist).unwrap();
        let second = stage_release(&held, &home, &binary, &dist).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read_dir(home.join("releases")).unwrap().count(), 1);
    }

    #[test]
    fn an_owned_release_that_no_longer_verifies_is_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let release = stage_release(&held, &home, &binary, &dist).unwrap();
        std::fs::write(release.web.join("index.html"), b"changed").unwrap();
        let again = stage_release(&held, &home, &binary, &dist).unwrap();
        assert_eq!(
            std::fs::read(again.web.join("index.html")).unwrap(),
            b"<html></html>"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_non_executable_or_linked_release_boundary_is_not_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let release = stage_release(&held, &home, &binary, &dist).unwrap();
        let manifest = read_manifest(&release.dir).unwrap();
        // Not executable: rejected, then repaired by staging again.
        std::fs::set_permissions(&release.binary, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!verify_release(&release.dir, &manifest));
        let again = stage_release(&held, &home, &binary, &dist).unwrap();
        assert!(is_executable(&again.binary));
        // A linked binary, web root or manifest never verifies.
        for part in [RELEASE_BINARY, RELEASE_WEB, RELEASE_MANIFEST] {
            let probe = temp.path().join(format!("probe-{part}"));
            copy_tree(&again.dir, &probe).unwrap();
            std::fs::copy(again.dir.join(RELEASE_BINARY), probe.join(RELEASE_BINARY)).unwrap();
            std::fs::set_permissions(
                probe.join(RELEASE_BINARY),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            let target = again.dir.join(part);
            let _ = std::fs::remove_dir_all(probe.join(part));
            let _ = std::fs::remove_file(probe.join(part));
            std::os::unix::fs::symlink(&target, probe.join(part)).unwrap();
            assert!(!verify_release(&probe, &manifest), "{part} link");
        }
        // A linked release directory is not ours either.
        let linked = temp.path().join("linked-release");
        std::os::unix::fs::symlink(&again.dir, &linked).unwrap();
        assert!(read_manifest(&linked).is_none());
        assert!(!verify_release(&linked, &manifest));
    }

    #[test]
    fn a_foreign_directory_at_the_release_name_stays_byte_exact() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let release = stage_release(&held, &home, &binary, &dist).unwrap();
        // Replace our release with an unmarked directory at the same name.
        std::fs::remove_dir_all(&release.dir).unwrap();
        std::fs::create_dir_all(release.dir.join("notes")).unwrap();
        std::fs::write(release.dir.join("notes/keep.txt"), b"mine").unwrap();
        let before = tree_bytes(&release.dir);
        let error = stage_release(&held, &home, &binary, &dist).unwrap_err();
        assert!(error.to_string().contains("left untouched"), "{error}");
        assert_eq!(tree_bytes(&release.dir), before);
        let leftovers: Vec<_> = std::fs::read_dir(home.join("releases"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_web_root_or_child_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        std::os::unix::fs::symlink(temp.path(), dist.join("escape")).unwrap();
        let error = stage_release(&lock(&home), &home, &binary, &dist).unwrap_err();
        assert!(error.to_string().contains("symbolic link"), "{error}");
        let linked_root = temp.path().join("dist-link");
        std::fs::remove_file(dist.join("escape")).unwrap();
        std::os::unix::fs::symlink(&dist, &linked_root).unwrap();
        let error = stage_release(&lock(&home), &home, &binary, &linked_root).unwrap_err();
        assert!(
            error.to_string().contains("not a plain directory"),
            "{error}"
        );
    }

    #[test]
    fn a_failed_stage_leaves_nothing_behind() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let missing = temp.path().join("no-such-binary");
        assert!(stage_release(&lock(&home), &home, &missing, temp.path()).is_err());
        let left = std::fs::read_dir(home.join("releases")).map_or(0, |entries| entries.count());
        assert_eq!(left, 0);
    }

    #[test]
    fn pruning_uses_markers_not_names_and_spares_live_staging() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let old = stage_release(&held, &home, &binary, &dist).unwrap();
        std::fs::write(&binary, b"server v2").unwrap();
        let previous = stage_release(&held, &home, &binary, &dist).unwrap();
        std::fs::write(&binary, b"server v3").unwrap();
        let current = stage_release(&held, &home, &binary, &dist).unwrap();
        let releases = home.join("releases");
        let marker = |pid: u32| {
            serde_json::to_vec(&StagingMarker {
                owner: OWNER.into(),
                pid,
                started_ns: 1,
            })
            .unwrap()
        };
        // Named like ours but unmarked: never removed.
        let named_only = releases.join(".staging-1-2");
        std::fs::create_dir_all(&named_only).unwrap();
        std::fs::write(named_only.join("data"), b"x").unwrap();
        // Marked by a live process (this test): spared.
        let live = releases.join(".staging-live");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(live.join(STAGING_MARKER), marker(std::process::id())).unwrap();
        // Marked by a process that no longer exists: ours to remove.
        let dead = releases.join("anything-dead");
        std::fs::create_dir_all(&dead).unwrap();
        std::fs::write(dead.join(STAGING_MARKER), marker(u32::MAX - 7)).unwrap();
        // A plain foreign directory and a foreign file.
        let foreign = releases.join("notes");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(releases.join("readme.txt"), b"keep").unwrap();
        let before = tree_bytes(&named_only);
        let removed = prune_releases(
            &held,
            &home,
            &[current.dir.as_path(), previous.dir.as_path()],
        );
        assert!(
            removed.contains(&old.dir) && removed.contains(&dead),
            "{removed:?}"
        );
        assert!(current.dir.exists() && previous.dir.exists());
        assert!(live.exists() && foreign.exists() && releases.join("readme.txt").exists());
        assert_eq!(tree_bytes(&named_only), before);
        prune_releases(&held, &home, &[]);
        assert!(!current.dir.exists() && !previous.dir.exists());
        assert!(named_only.exists() && live.exists() && foreign.exists());
    }

    #[test]
    fn a_second_installer_waits_for_the_home_lock_and_then_gives_up() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let held = lock(&home);
        let started = std::time::Instant::now();
        let error = lock_home(&home, std::time::Duration::from_millis(300))
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("Another service install"),
            "{error}"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(300));
        drop(held);
        assert!(lock_home(&home, std::time::Duration::from_millis(300)).is_ok());
    }

    #[test]
    fn only_a_definition_pointing_into_an_owned_release_counts_for_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let (binary, dist) = source(temp.path(), b"server v1");
        let release = stage_release(&lock(&home), &home, &binary, &dist).unwrap();
        assert_eq!(
            release_of(&home, &release.binary),
            Some(release.dir.clone())
        );
        assert_eq!(release_of(&home, &binary), None);
    }

    /// Records calls; loads fail while `fail_loads` > 0, and unloads return the
    /// queued outcomes in order (then `Stopped`).
    #[derive(Default)]
    struct FakeManager {
        calls: std::cell::RefCell<Vec<String>>,
        fail_loads: std::cell::Cell<u32>,
        unloads: std::cell::RefCell<std::collections::VecDeque<Result<Unloaded, String>>>,
        /// Answers to `state()`, in order; `Running` once empty (the cautious answer).
        states: std::cell::RefCell<std::collections::VecDeque<JobState>>,
        /// `state()` calls, kept apart from `calls` so earlier call-order checks hold.
        queries: std::cell::Cell<u32>,
    }
    impl FakeManager {
        fn failing_loads(count: u32) -> Self {
            Self {
                fail_loads: std::cell::Cell::new(count),
                ..Self::default()
            }
        }
        fn with_states(self, states: Vec<JobState>) -> Self {
            *self.states.borrow_mut() = states.into();
            self
        }
        fn with_unloads(self, outcomes: Vec<Result<Unloaded, &str>>) -> Self {
            *self.unloads.borrow_mut() = outcomes
                .into_iter()
                .map(|outcome| outcome.map_err(str::to_owned))
                .collect();
            self
        }
    }
    impl Manager for FakeManager {
        fn unload(&self) -> Result<Unloaded, ServiceError> {
            self.calls.borrow_mut().push("unload".into());
            match self.unloads.borrow_mut().pop_front() {
                Some(Err(message)) => Err(ServiceError::Failed(message)),
                Some(Ok(outcome)) => Ok(outcome),
                None => Ok(Unloaded::Stopped),
            }
        }
        fn load(&self, path: &Path) -> Result<(), ServiceError> {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            self.calls.borrow_mut().push(format!("load {text}"));
            if self.fail_loads.get() > 0 {
                self.fail_loads.set(self.fail_loads.get() - 1);
                return Err(ServiceError::Failed("load refused.".into()));
            }
            Ok(())
        }
        fn state(&self) -> JobState {
            self.queries.set(self.queries.get() + 1);
            self.states
                .borrow_mut()
                .pop_front()
                .unwrap_or(JobState::Running)
        }
    }

    /// A clock that advances when asked to sleep, so waits cost no real time.
    struct FakeClock {
        base: std::time::Instant,
        elapsed: std::cell::Cell<std::time::Duration>,
        sleeps: std::cell::Cell<u32>,
    }
    impl FakeClock {
        fn new() -> Self {
            Self {
                base: std::time::Instant::now(),
                elapsed: std::cell::Cell::new(std::time::Duration::ZERO),
                sleeps: std::cell::Cell::new(0),
            }
        }
        fn leaked() -> &'static FakeClock {
            Box::leak(Box::new(Self::new()))
        }
    }
    impl Clock for FakeClock {
        fn now(&self) -> std::time::Instant {
            self.base + self.elapsed.get()
        }
        fn sleep(&self, pause: std::time::Duration) {
            self.elapsed.set(self.elapsed.get() + pause);
            self.sleeps.set(self.sleeps.get() + 1);
        }
    }

    type Writer = Box<dyn Fn(&Path, &[u8]) -> Result<(), ServiceError>>;

    /// Fails (without touching the file) whenever asked to write `fail_on`.
    fn failing_write(fail_on: &'static [u8]) -> Writer {
        Box::new(move |path: &Path, bytes: &[u8]| {
            if bytes == fail_on {
                Err(ServiceError::Failed("disk full.".into()))
            } else {
                replace_durably(path, bytes)
            }
        })
    }

    /// Switches the file to `bytes` and then fails, like a directory flush
    /// failing after the rename, whenever asked to write `mutate_on`.
    fn mutating_then_failing_write(mutate_on: &'static [u8]) -> Writer {
        Box::new(move |path: &Path, bytes: &[u8]| {
            replace_durably(path, bytes)?;
            if bytes == mutate_on {
                return Err(ServiceError::Failed("flush failed after rename.".into()));
            }
            Ok(())
        })
    }

    fn calls(manager: &FakeManager) -> Vec<String> {
        manager.calls.borrow().clone()
    }

    fn out(code: Option<i32>, stdout: &str, stderr: &str) -> QueryOutput {
        QueryOutput {
            code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn launchctl_print_is_absent_only_for_its_own_no_such_service_answer() {
        let label = "dev.bruhs.server";
        // Observed on this machine (read-only queries, nothing installed).
        let absent = out(
            Some(113),
            "",
            "Bad request.\nCould not find service \"dev.bruhs.server\" in domain for user gui: 502\n",
        );
        assert_eq!(parse_launchctl_print(&absent, label), JobState::NotLoaded);
        let no_domain = out(
            Some(112),
            "",
            "Bad request.\nCould not find domain for user gui: 999999\n",
        );
        assert!(matches!(
            parse_launchctl_print(&no_domain, label),
            JobState::Indeterminate(_)
        ));
        assert_eq!(
            parse_launchctl_print(&out(Some(0), "state = running", ""), label),
            JobState::Running
        );
        for unsettled in [
            out(Some(1), "", "Operation not permitted"),
            out(
                Some(113),
                "",
                "Could not find service \"other.label\" in domain",
            ),
            out(Some(5), "", "Input/output error"),
            out(None, "", ""),
        ] {
            assert!(
                matches!(
                    parse_launchctl_print(&unsettled, label),
                    JobState::Indeterminate(_)
                ),
                "{unsettled:?}"
            );
        }
    }

    #[test]
    fn systemctl_show_needs_every_field_and_a_working_bus() {
        let show = |text: &str| parse_systemctl_show(&out(Some(0), text, ""));
        assert_eq!(
            show("LoadState=not-found\nActiveState=inactive\nMainPID=0\n"),
            JobState::NotLoaded
        );
        assert_eq!(
            show("LoadState=loaded\nActiveState=inactive\nMainPID=0\n"),
            JobState::Inactive
        );
        assert_eq!(
            show("LoadState=loaded\nActiveState=failed\nMainPID=0\n"),
            JobState::Inactive
        );
        assert_eq!(
            show("LoadState=loaded\nActiveState=active\nMainPID=42\n"),
            JobState::Running
        );
        assert_eq!(
            show("LoadState=loaded\nActiveState=deactivating\nMainPID=0\n"),
            JobState::Running
        );
        assert_eq!(
            show("LoadState=loaded\nActiveState=inactive\nMainPID=42\n"),
            JobState::Running
        );
        for unsettled in [
            "LoadState=loaded\nActiveState=inactive\n",
            "LoadState=error\nActiveState=inactive\nMainPID=0\n",
            "LoadState=loaded\nActiveState=inactive\nMainPID=x\n",
            "",
        ] {
            assert!(
                matches!(show(unsettled), JobState::Indeterminate(_)),
                "{unsettled:?}"
            );
        }
        let bus = out(Some(1), "", "Failed to connect to bus: No medium found");
        assert!(matches!(
            parse_systemctl_show(&bus),
            JobState::Indeterminate(_)
        ));
        let denied = out(Some(4), "", "Access denied");
        assert!(matches!(
            parse_systemctl_show(&denied),
            JobState::Indeterminate(_)
        ));
    }

    #[test]
    fn unload_outcomes_never_treat_an_unsettled_state_as_absence() {
        assert!(matches!(
            confirm_unloaded(true, JobState::NotLoaded, ""),
            Ok(Unloaded::Stopped)
        ));
        assert!(matches!(
            confirm_unloaded(false, JobState::NotLoaded, ""),
            Ok(Unloaded::WasAbsent)
        ));
        assert!(matches!(
            confirm_unloaded(true, JobState::Inactive, ""),
            Ok(Unloaded::Stopped)
        ));
        let running = confirm_unloaded(true, JobState::Running, "")
            .unwrap_err()
            .to_string();
        assert!(running.contains("still loaded"), "{running}");
        for unload_ok in [true, false] {
            let unknown = confirm_unloaded(
                unload_ok,
                JobState::Indeterminate("exit 1: denied".into()),
                "",
            )
            .unwrap_err()
            .to_string();
            assert!(
                unknown.contains("Could not confirm") && unknown.contains("denied"),
                "{unknown}"
            );
        }
    }

    /// Answers launchctl/systemctl calls from a script keyed by the subcommand.
    fn scripted(
        answers: Vec<(&'static str, Result<QueryOutput, String>)>,
    ) -> impl Fn(&str, &[&str]) -> Result<QueryOutput, String> {
        move |_program: &str, args: &[&str]| {
            let verb = args
                .iter()
                .find(|arg| !arg.starts_with("--"))
                .copied()
                .unwrap_or("");
            answers.iter().find(|(name, _)| *name == verb).map_or_else(
                || Err(format!("unscripted {verb}")),
                |(_, answer)| answer.clone(),
            )
        }
    }

    fn launchd(runner: &Runner) -> Launchd<'_> {
        launchd_with(runner, FakeClock::leaked())
    }

    fn launchd_with<'a>(runner: &'a Runner, clock: &'a FakeClock) -> Launchd<'a> {
        Launchd {
            domain: "gui/502".into(),
            label: "dev.bruhs.server",
            runner,
            clock,
            settle: SETTLE,
        }
    }

    #[test]
    fn launchd_unload_is_absent_only_when_launchd_says_so() {
        let not_found = out(
            Some(113),
            "",
            "Could not find service \"dev.bruhs.server\" in domain for user gui: 502",
        );
        // bootout fails because nothing is loaded, and print confirms it.
        let runner = scripted(vec![
            (
                "bootout",
                Ok(out(Some(3), "", "Boot-out failed: 3: No such process")),
            ),
            ("print", Ok(not_found.clone())),
        ]);
        assert!(matches!(launchd(&runner).unload(), Ok(Unloaded::WasAbsent)));
        // bootout works and the job is gone.
        let runner = scripted(vec![
            ("bootout", Ok(out(Some(0), "", ""))),
            ("print", Ok(not_found)),
        ]);
        assert!(matches!(launchd(&runner).unload(), Ok(Unloaded::Stopped)));
        // Permission, domain and spawn failures of the check are never absence.
        for check in [
            Ok(out(Some(1), "", "Operation not permitted")),
            Ok(out(
                Some(112),
                "",
                "Could not find domain for user gui: 502",
            )),
            Err("launchctl could not run: No such file or directory".to_owned()),
        ] {
            let runner = scripted(vec![
                (
                    "bootout",
                    Ok(out(Some(5), "", "Boot-out failed: 5: Input/output error")),
                ),
                ("print", check.clone()),
            ]);
            let error = launchd(&runner).unload().unwrap_err().to_string();
            assert!(error.contains("Could not confirm"), "{check:?}: {error}");
        }
        // Still in the domain after bootout.
        let runner = scripted(vec![
            ("bootout", Ok(out(Some(0), "", ""))),
            ("print", Ok(out(Some(0), "state = running", ""))),
        ]);
        assert!(launchd(&runner)
            .unload()
            .unwrap_err()
            .to_string()
            .contains("still loaded"));
    }

    #[test]
    fn systemd_unload_is_absent_only_when_systemd_says_so() {
        let systemd = |runner: &'static Runner| Systemd {
            unit: "bruhs-server.service",
            runner,
            clock: FakeClock::leaked(),
            settle: SETTLE,
        };
        let leak = |runner: Box<Runner>| -> &'static Runner { Box::leak(runner) };
        let gone = leak(Box::new(scripted(vec![
            (
                "disable",
                Ok(out(
                    Some(1),
                    "",
                    "Unit file bruhs-server.service does not exist.",
                )),
            ),
            (
                "show",
                Ok(out(
                    Some(0),
                    "LoadState=not-found\nActiveState=inactive\nMainPID=0\n",
                    "",
                )),
            ),
        ])));
        assert!(matches!(systemd(gone).unload(), Ok(Unloaded::WasAbsent)));
        let bus = leak(Box::new(scripted(vec![
            ("disable", Ok(out(Some(1), "", "Failed to connect to bus"))),
            (
                "show",
                Ok(out(
                    Some(1),
                    "",
                    "Failed to connect to bus: No medium found",
                )),
            ),
        ])));
        assert!(systemd(bus)
            .unload()
            .unwrap_err()
            .to_string()
            .contains("Could not confirm"));
        let running = leak(Box::new(scripted(vec![
            ("disable", Ok(out(Some(0), "", ""))),
            (
                "show",
                Ok(out(
                    Some(0),
                    "LoadState=loaded\nActiveState=active\nMainPID=7\n",
                    "",
                )),
            ),
        ])));
        assert!(systemd(running)
            .unload()
            .unwrap_err()
            .to_string()
            .contains("still loaded"));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_definition_is_an_error_not_an_absent_one() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            read_definition(&temp.path().join("missing.plist")).unwrap(),
            None
        );
        // A directory where the definition should be.
        let dir = temp.path().join("as-dir.plist");
        std::fs::create_dir(&dir).unwrap();
        assert!(read_definition(&dir).is_err());
        // An unreadable file (skipped when running as root, which can read it).
        let locked = temp.path().join("locked.plist");
        std::fs::write(&locked, b"old").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&locked).is_err() {
            assert!(read_definition(&locked).is_err());
            // Restoring around an unreadable definition reports, never assumes.
            let manager = FakeManager::default();
            let error = switch_definition(
                &manager,
                &locked,
                b"new",
                Some(b"old"),
                &failing_write(b"new"),
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("also failed") && error.contains("Could not read"),
                "{error}"
            );
            assert!(calls(&manager).is_empty());
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn a_definition_switch_loads_the_new_definition() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agents/dev.bruhs.server.plist");
        let manager = FakeManager::default();
        switch_definition(&manager, &path, b"new", None, &replace_durably).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(calls(&manager), ["unload", "load new"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn a_write_that_fails_before_switching_keeps_the_old_definition_loaded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default();
        let error = switch_definition(
            &manager,
            &path,
            b"new",
            Some(b"old"),
            &failing_write(b"new"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("disk full") && error.contains("restored"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert!(calls(&manager).is_empty(), "nothing unloaded");
    }

    #[test]
    fn a_write_that_switches_then_fails_is_restored_before_any_manager_change() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default();
        let error = switch_definition(
            &manager,
            &path,
            b"new",
            Some(b"old"),
            &mutating_then_failing_write(b"new"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("flush failed after rename") && error.contains("restored"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert!(calls(&manager).is_empty());
        // With no previous definition, the switched-in file is removed again.
        let fresh = temp.path().join("fresh.plist");
        let error = switch_definition(
            &manager,
            &fresh,
            b"new",
            None,
            &mutating_then_failing_write(b"new"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("restored"), "{error}");
        assert!(!fresh.exists());
        assert!(calls(&manager).is_empty());
    }

    #[test]
    fn a_mutating_write_whose_restore_also_fails_reports_both() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default();
        // New switches in then fails; restoring old fails without writing.
        let write = |path: &Path, bytes: &[u8]| -> Result<(), ServiceError> {
            if bytes == b"old" {
                return Err(ServiceError::Failed("disk full.".into()));
            }
            replace_durably(path, bytes)?;
            Err(ServiceError::Failed("flush failed after rename.".into()))
        };
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &write)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("flush failed after rename")
                && error.contains("also failed")
                && error.contains("disk full"),
            "{error}"
        );
        assert!(calls(&manager).is_empty());
    }

    #[test]
    fn an_unconfirmed_unload_keeps_the_old_definition_and_loads_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default().with_unloads(vec![Err("still loaded.")]);
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("still loaded") && error.contains("nothing new was loaded"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(calls(&manager), ["unload"]);
    }

    #[test]
    fn an_unconfirmed_unload_whose_restore_fails_reports_both() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default().with_unloads(vec![Err("still loaded.")]);
        let error = switch_definition(
            &manager,
            &path,
            b"new",
            Some(b"old"),
            &failing_write(b"old"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("still loaded") && error.contains("also failed"),
            "{error}"
        );
        assert_eq!(
            calls(&manager),
            ["unload"],
            "never loads after an unconfirmed unload"
        );
    }

    #[test]
    fn a_failed_load_restores_and_reloads_the_previous_definition() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::failing_loads(1);
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("previous service setup was restored"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(
            calls(&manager),
            ["unload", "load new", "unload", "load old"]
        );
    }

    #[test]
    fn a_failed_load_that_cannot_be_confirmed_stopped_is_left_and_reported() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::failing_loads(1)
            .with_unloads(vec![Ok(Unloaded::Stopped), Err("still loaded.")]);
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("load refused") && error.contains("nothing was rolled back"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"new", "left as is");
        assert_eq!(calls(&manager), ["unload", "load new", "unload"]);
    }

    #[test]
    fn a_failed_first_install_removes_its_definition() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        let manager = FakeManager::failing_loads(1).with_unloads(vec![Ok(Unloaded::WasAbsent)]);
        assert!(switch_definition(&manager, &path, b"new", None, &replace_durably).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_failed_rollback_is_reported_with_the_original_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::failing_loads(2);
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("load refused") && error.contains("also failed"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let manager = FakeManager::failing_loads(1);
        let error = switch_definition(
            &manager,
            &path,
            b"new",
            Some(b"old"),
            &failing_write(b"old"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("also failed") && error.contains("disk full"),
            "{error}"
        );
    }

    /// Scripted answers per subcommand, in order.
    type Script = Vec<(&'static str, Vec<Result<QueryOutput, String>>)>;
    /// Calls seen per subcommand.
    type Counts = std::rc::Rc<std::cell::RefCell<BTreeMap<String, u32>>>;

    /// Answers each subcommand from its own queue; the last answer repeats.
    /// Also counts calls per subcommand.
    fn sequenced(
        answers: Script,
    ) -> (
        impl Fn(&str, &[&str]) -> Result<QueryOutput, String>,
        Counts,
    ) {
        let counts = std::rc::Rc::new(std::cell::RefCell::new(BTreeMap::new()));
        let seen = counts.clone();
        let queues = std::cell::RefCell::new(
            answers
                .into_iter()
                .map(|(verb, list)| (verb, std::collections::VecDeque::from(list)))
                .collect::<Vec<_>>(),
        );
        let runner = move |_program: &str, args: &[&str]| {
            let verb = args
                .iter()
                .find(|arg| !arg.starts_with("--"))
                .copied()
                .unwrap_or("");
            *seen.borrow_mut().entry(verb.to_owned()).or_insert(0) += 1;
            let mut queues = queues.borrow_mut();
            let Some((_, queue)) = queues.iter_mut().find(|(name, _)| *name == verb) else {
                return Err(format!("unscripted {verb}"));
            };
            if queue.len() > 1 {
                queue.pop_front().unwrap()
            } else {
                queue.front().cloned().unwrap()
            }
        };
        (runner, counts)
    }

    fn running() -> Result<QueryOutput, String> {
        Ok(out(Some(0), "state = running", ""))
    }
    fn not_found() -> Result<QueryOutput, String> {
        Ok(out(
            Some(113),
            "",
            "Could not find service \"dev.bruhs.server\" in domain for user gui: 502",
        ))
    }

    #[test]
    fn settling_waits_out_teardown_and_never_waits_on_unknown() {
        let clock = FakeClock::new();
        let answers = std::cell::RefCell::new(std::collections::VecDeque::from(vec![
            JobState::Running,
            JobState::Running,
            JobState::NotLoaded,
        ]));
        let query = || answers.borrow_mut().pop_front().unwrap();
        assert_eq!(settle_state(&query, &clock, SETTLE), JobState::NotLoaded);
        assert_eq!(clock.sleeps.get(), 2);
        // Persistent running stops at the deadline, in fake time only.
        let clock = FakeClock::new();
        assert_eq!(
            settle_state(&|| JobState::Running, &clock, SETTLE),
            JobState::Running
        );
        assert!(clock.elapsed.get() >= SETTLE.deadline);
        assert!(clock.elapsed.get() < SETTLE.deadline + SETTLE.interval * 2);
        // Unknown is final at once: no sleep, never absence.
        let clock = FakeClock::new();
        let unknown = || JobState::Indeterminate("exit 1: denied".into());
        assert!(matches!(
            settle_state(&unknown, &clock, SETTLE),
            JobState::Indeterminate(_)
        ));
        assert_eq!(clock.sleeps.get(), 0);
    }

    #[test]
    fn a_job_still_listed_during_teardown_is_confirmed_stopped_once_it_goes() {
        // The R2-lease case: bootout succeeded, print still lists the job briefly.
        let (runner, counts) = sequenced(vec![
            ("bootout", vec![Ok(out(Some(0), "", ""))]),
            ("print", vec![running(), running(), not_found()]),
        ]);
        let clock = FakeClock::new();
        assert!(matches!(
            launchd_with(&runner, &clock).unload(),
            Ok(Unloaded::Stopped)
        ));
        assert_eq!(counts.borrow()["print"], 3);
        assert_eq!(clock.sleeps.get(), 2);
    }

    #[test]
    fn a_job_that_stays_listed_is_still_loaded_at_the_deadline() {
        let (runner, _) = sequenced(vec![
            ("bootout", vec![Ok(out(Some(0), "", ""))]),
            ("print", vec![running()]),
        ]);
        let clock = FakeClock::new();
        let error = launchd_with(&runner, &clock)
            .unload()
            .unwrap_err()
            .to_string();
        assert!(error.contains("still loaded"), "{error}");
        assert!(clock.elapsed.get() >= SETTLE.deadline);
    }

    #[test]
    fn an_unknown_state_during_teardown_fails_at_once_and_keeps_command_detail() {
        // Running, then a permission error: unknown is final, never absence.
        let (runner, counts) = sequenced(vec![
            ("bootout", vec![Ok(out(Some(0), "", ""))]),
            (
                "print",
                vec![running(), Ok(out(Some(1), "", "Operation not permitted"))],
            ),
        ]);
        let clock = FakeClock::new();
        let error = launchd_with(&runner, &clock)
            .unload()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Could not confirm") && error.contains("not permitted"),
            "{error}"
        );
        assert_eq!(counts.borrow()["print"], 2);
        // A refused bootout is queried once, with its detail kept, and never waits.
        let (runner, counts) = sequenced(vec![
            (
                "bootout",
                vec![Ok(out(
                    Some(5),
                    "",
                    "Boot-out failed: 5: Input/output error",
                ))],
            ),
            ("print", vec![running()]),
        ]);
        let clock = FakeClock::new();
        let error = launchd_with(&runner, &clock)
            .unload()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("still loaded") && error.contains("Input/output error"),
            "{error}"
        );
        assert_eq!(counts.borrow()["print"], 1);
        assert_eq!(clock.sleeps.get(), 0);
    }

    #[test]
    fn systemd_waits_for_a_unit_that_is_still_deactivating() {
        let (runner, counts) = sequenced(vec![
            ("disable", vec![Ok(out(Some(0), "", ""))]),
            (
                "show",
                vec![
                    Ok(out(
                        Some(0),
                        "LoadState=loaded\nActiveState=deactivating\nMainPID=7\n",
                        "",
                    )),
                    Ok(out(
                        Some(0),
                        "LoadState=loaded\nActiveState=inactive\nMainPID=0\n",
                        "",
                    )),
                ],
            ),
        ]);
        let clock = FakeClock::new();
        let systemd = Systemd {
            unit: "bruhs-server.service",
            runner: &runner,
            clock: &clock,
            settle: SETTLE,
        };
        assert!(matches!(systemd.unload(), Ok(Unloaded::Stopped)));
        assert_eq!(counts.borrow()["show"], 2);
    }

    #[test]
    fn a_reinstall_whose_old_job_stopped_late_reloads_the_restored_definition() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        // Unload is unconfirmed, but after the restore the job is confirmed gone.
        let manager = FakeManager::default()
            .with_unloads(vec![Err("still loaded.")])
            .with_states(vec![JobState::NotLoaded]);
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
            .unwrap_err()
            .to_string();
        assert!(error.contains("restored and loaded again"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(calls(&manager), ["unload", "load old"]);
        assert_eq!(manager.queries.get(), 1);
        // Inactive (systemd) confirms the same way.
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default()
            .with_unloads(vec![Err("still loaded.")])
            .with_states(vec![JobState::Inactive]);
        assert!(
            switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably).is_err()
        );
        assert_eq!(calls(&manager), ["unload", "load old"]);
    }

    #[test]
    fn a_running_or_unknown_old_job_is_never_loaded_twice() {
        for state in [
            JobState::Running,
            JobState::Indeterminate("exit 1: denied".into()),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("dev.bruhs.server.plist");
            std::fs::write(&path, b"old").unwrap();
            let manager = FakeManager::default()
                .with_unloads(vec![Err("still loaded.")])
                .with_states(vec![state.clone()]);
            let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("nothing new was loaded"),
                "{state:?}: {error}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            assert_eq!(calls(&manager), ["unload"], "{state:?}");
            assert_eq!(manager.queries.get(), 1);
            if let JobState::Indeterminate(why) = &state {
                assert!(error.contains(why), "{error}");
            }
        }
    }

    #[test]
    fn a_failed_reload_after_late_teardown_is_reported_with_both_errors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::failing_loads(1)
            .with_unloads(vec![Err("still loaded.")])
            .with_states(vec![JobState::NotLoaded]);
        let error = switch_definition(&manager, &path, b"new", Some(b"old"), &replace_durably)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("still loaded")
                && error.contains("loading it again failed")
                && error.contains("load refused"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn recovery_never_queries_or_loads_without_a_restored_previous_definition() {
        let temp = tempfile::tempdir().unwrap();
        // First install: there is nothing to reload; the new definition is removed.
        let fresh = temp.path().join("fresh.plist");
        let manager = FakeManager::default()
            .with_unloads(vec![Err("still loaded.")])
            .with_states(vec![JobState::NotLoaded]);
        assert!(switch_definition(&manager, &fresh, b"new", None, &replace_durably).is_err());
        assert!(!fresh.exists());
        assert_eq!(manager.queries.get(), 0);
        assert_eq!(calls(&manager), ["unload"]);
        // A restore that fails: the file is unknown, so nothing is loaded.
        let path = temp.path().join("dev.bruhs.server.plist");
        std::fs::write(&path, b"old").unwrap();
        let manager = FakeManager::default()
            .with_unloads(vec![Err("still loaded.")])
            .with_states(vec![JobState::NotLoaded]);
        let error = switch_definition(
            &manager,
            &path,
            b"new",
            Some(b"old"),
            &failing_write(b"old"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("also failed"), "{error}");
        assert_eq!(manager.queries.get(), 0);
        assert_eq!(calls(&manager), ["unload"]);
    }

    #[test]
    fn uninstall_confirms_a_late_teardown_before_removing_anything() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let path = temp.path().join("dev.bruhs.server.plist");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let release = stage_release(&held, &home, &binary, &dist).unwrap();
        std::fs::write(&path, b"definition").unwrap();
        // Persistent running: everything is kept.
        let (runner, _) = sequenced(vec![
            ("bootout", vec![Ok(out(Some(0), "", ""))]),
            ("print", vec![running()]),
        ]);
        let clock = FakeClock::new();
        let error = uninstall(&held, &launchd_with(&runner, &clock), &home, &path)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("still loaded") && error.contains("kept"),
            "{error}"
        );
        assert!(path.exists() && release.dir.exists());
        // Listed briefly, then gone: confirmed, then removed.
        let (runner, _) = sequenced(vec![
            ("bootout", vec![Ok(out(Some(0), "", ""))]),
            ("print", vec![running(), not_found()]),
        ]);
        let clock = FakeClock::new();
        assert_eq!(
            uninstall(&held, &launchd_with(&runner, &clock), &home, &path).unwrap(),
            Unloaded::Stopped
        );
        assert!(!path.exists() && !release.dir.exists());
    }

    #[test]
    fn uninstall_keeps_the_definition_and_releases_unless_the_stop_is_confirmed() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("service");
        let path = temp.path().join("dev.bruhs.server.plist");
        let (binary, dist) = source(temp.path(), b"server v1");
        let held = lock(&home);
        let release = stage_release(&held, &home, &binary, &dist).unwrap();
        std::fs::write(&path, b"definition").unwrap();
        let before = tree_bytes(&release.dir);
        // A live service that cannot be confirmed stopped: nothing is removed.
        let manager = FakeManager::default().with_unloads(vec![Err("still loaded.")]);
        let error = uninstall(&held, &manager, &home, &path)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("still loaded") && error.contains("kept"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"definition");
        assert_eq!(tree_bytes(&release.dir), before);
        // Confirmed stopped: the definition and owned releases go.
        let manager = FakeManager::default();
        assert_eq!(
            uninstall(&held, &manager, &home, &path).unwrap(),
            Unloaded::Stopped
        );
        assert!(!path.exists() && !release.dir.exists());
        // Already absent is a confirmed outcome too.
        let manager = FakeManager::default().with_unloads(vec![Ok(Unloaded::WasAbsent)]);
        assert_eq!(
            uninstall(&held, &manager, &home, &path).unwrap(),
            Unloaded::WasAbsent
        );
    }

    #[test]
    fn the_service_environment_points_the_web_bundle_at_the_release() {
        let env = environment(Path::new("/app-data/service/releases/abc/web"));
        assert!(env
            .iter()
            .any(|(key, value)| key == "BRUHS_WEB_DIST"
                && value == "/app-data/service/releases/abc/web"));
        assert_eq!(
            env.iter()
                .filter(|(key, _)| key == "BRUHS_WEB_DIST")
                .count(),
            1
        );
        assert!(
            env.iter().all(|(key, _)| key != "HOME"),
            "HOME is never set"
        );
    }

    #[test]
    fn the_installer_carries_settings_but_not_its_own_lifetime() {
        let env = environment(Path::new("/web"));
        assert!(env.iter().all(|(key, _)| key != "BRUHS_EXIT_WITH_PARENT"));
        assert!(env
            .iter()
            .any(|(key, value)| key == "BRUHS_WEB_DIST" && value == "/web"));
    }

    #[test]
    fn the_installer_never_carries_in_memory_secrets() {
        assert!(!carried("BRUHS_SECRET_STORE"));
        assert!(!carried("BRUHS_EXIT_WITH_PARENT"));
        assert!(carried("BRUHS_WEB_ORIGIN"));
        assert!(carried("PATH"));
        assert!(!carried("HOME"));
    }

    /// D24: a debug build's test upstream never follows the server into
    /// the service, which then calls the real apps.
    #[test]
    fn the_service_never_copies_the_upstream_switch() {
        assert_eq!(bruhs_core::apps::APPS_UPSTREAM_ENV, "BRUHS_APPS_UPSTREAM");
        assert!(!carried(bruhs_core::apps::APPS_UPSTREAM_ENV));
        assert!(carried("BRUHS_APPS_OTHER"));
    }
}
